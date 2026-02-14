use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::provider::{TypeaheadItem, TypeaheadSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mention {
    File(PathBuf),
}

impl TypeaheadItem for Mention {
    fn token_text(&self) -> String {
        match self {
            Mention::File(path) => path_text(path),
        }
    }

    fn description(&self) -> Option<String> {
        Some("File".to_string())
    }
}

#[derive(Debug, Clone)]
struct FileEntry {
    relative: PathBuf,
    display: String,
    display_lower: String,
    file_name_lower: String,
}

#[derive(Debug, Clone)]
struct FilterCache {
    query: String,
    indices: Vec<usize>,
}

#[derive(Debug)]
struct WorkerRequest {
    request_id: u64,
    query: String,
}

#[derive(Debug)]
struct WorkerResponse {
    request_id: u64,
    query: String,
    matches: Vec<Mention>,
}

#[derive(Debug)]
enum WorkerEvent {
    Ready,
    Response(WorkerResponse),
}

#[derive(Debug)]
pub struct FileMentionSource {
    request_tx: Sender<WorkerRequest>,
    response_rx: Receiver<WorkerEvent>,
    index_ready: bool,
    last_requested_query: Option<String>,
    pending_request_id: Option<u64>,
    last_completed_request_id: u64,
    latest_query: Option<String>,
    latest_matches: Vec<Mention>,
    next_request_id: u64,
}

impl FileMentionSource {
    pub fn new(root: PathBuf) -> Self {
        let (request_tx, response_rx) = spawn_file_worker(root);
        Self {
            request_tx,
            response_rx,
            index_ready: false,
            last_requested_query: None,
            pending_request_id: None,
            last_completed_request_id: 0,
            latest_query: None,
            latest_matches: Vec::new(),
            next_request_id: 0,
        }
    }

    pub fn has_pending_work(&self) -> bool {
        !self.index_ready || self.pending_request_id.is_some()
    }

    pub fn loading_for_query(&self, query: &str) -> bool {
        let normalized = query.to_ascii_lowercase();
        !self.index_ready
            || (self.pending_request_id.is_some()
                && self.latest_query.as_deref() != Some(normalized.as_str()))
    }

    fn drain_worker_responses(&mut self) {
        while let Ok(event) = self.response_rx.try_recv() {
            match event {
                WorkerEvent::Ready => {
                    self.index_ready = true;
                }
                WorkerEvent::Response(response) => {
                    if response.request_id < self.last_completed_request_id {
                        continue;
                    }

                    self.last_completed_request_id = response.request_id;
                    self.latest_query = Some(response.query);
                    self.latest_matches = response.matches;

                    if self.pending_request_id == Some(response.request_id) {
                        self.pending_request_id = None;
                    } else if self
                        .pending_request_id
                        .is_some_and(|request_id| request_id < response.request_id)
                    {
                        self.pending_request_id = None;
                    }
                }
            }
        }
    }

    fn send_query_request(&mut self, query: &str) {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;
        let request = WorkerRequest {
            request_id,
            query: query.to_string(),
        };

        if self.request_tx.send(request).is_ok() {
            self.last_requested_query = Some(query.to_string());
            self.pending_request_id = Some(request_id);
        } else {
            self.pending_request_id = None;
        }
    }
}

impl TypeaheadSource<Mention> for FileMentionSource {
    fn query(&mut self, query: &str) -> Vec<Mention> {
        let normalized = query.to_ascii_lowercase();
        self.drain_worker_responses();
        if self.last_requested_query.as_deref() != Some(normalized.as_str()) {
            self.send_query_request(&normalized);
            self.drain_worker_responses();
        }

        if self.latest_query.as_deref() == Some(normalized.as_str()) {
            self.latest_matches.clone()
        } else {
            Vec::new()
        }
    }
}

fn spawn_file_worker(root: PathBuf) -> (Sender<WorkerRequest>, Receiver<WorkerEvent>) {
    let (request_tx, request_rx) = mpsc::channel::<WorkerRequest>();
    let (response_tx, response_rx) = mpsc::channel::<WorkerEvent>();
    let worker = move || run_file_worker(root, request_rx, response_tx);

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        drop(handle.spawn_blocking(worker));
    } else {
        drop(std::thread::spawn(worker));
    }

    (request_tx, response_rx)
}

fn run_file_worker(
    root: PathBuf,
    request_rx: Receiver<WorkerRequest>,
    response_tx: Sender<WorkerEvent>,
) {
    let entries = collect_file_entries(&root);
    if response_tx.send(WorkerEvent::Ready).is_err() {
        return;
    }
    let mut cache = FilterCache {
        query: String::new(),
        indices: (0..entries.len()).collect(),
    };

    while let Ok(mut request) = request_rx.recv() {
        while let Ok(newer) = request_rx.try_recv() {
            request = newer;
        }

        let matches = find_matches(&entries, &mut cache, &request.query);
        if response_tx
            .send(WorkerEvent::Response(WorkerResponse {
                request_id: request.request_id,
                query: request.query,
                matches,
            }))
            .is_err()
        {
            return;
        }
    }
}

fn find_matches(entries: &[FileEntry], cache: &mut FilterCache, query: &str) -> Vec<Mention> {
    if query == cache.query {
        return cache
            .indices
            .iter()
            .map(|index| Mention::File(entries[*index].relative.clone()))
            .collect();
    }

    let growing = query.starts_with(&cache.query);
    let candidate_indices = if growing {
        cache.indices.clone()
    } else {
        (0..entries.len()).collect()
    };

    let mut scored = Vec::new();
    for index in candidate_indices {
        if let Some(score) = file_match_score(&entries[index], query) {
            scored.push((index, score));
        }
    }

    scored.sort_by(|(left_index, left_score), (right_index, right_score)| {
        left_score.cmp(right_score).then_with(|| {
            entries[*left_index]
                .display
                .cmp(&entries[*right_index].display)
        })
    });

    cache.query.clear();
    cache.query.push_str(query);
    cache.indices = scored.iter().map(|(index, _)| *index).collect();
    cache
        .indices
        .iter()
        .map(|index| Mention::File(entries[*index].relative.clone()))
        .collect()
}

fn file_match_score(entry: &FileEntry, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(3);
    }

    if entry.file_name_lower.starts_with(query) {
        return Some(0);
    }

    if entry
        .display_lower
        .split('/')
        .any(|segment| segment.starts_with(query))
    {
        return Some(1);
    }

    if entry.display_lower.contains(query) {
        return Some(2);
    }

    None
}

fn collect_file_entries(root: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    walk_dir_with_scoped_ignores(root, &mut entries);
    entries.sort_by(|left, right| left.display.cmp(&right.display));
    entries
}

fn walk_dir_with_scoped_ignores(root: &Path, entries: &mut Vec<FileEntry>) {
    let mut queue = VecDeque::new();
    queue.push_back((root.to_path_buf(), Vec::<Gitignore>::new()));

    while let Some((dir, mut ignore_stack)) = queue.pop_front() {
        if dir.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if is_ignored_by_stack(&ignore_stack, &dir, true) {
            continue;
        }

        if let Some(matcher) = load_local_gitignore(&dir) {
            ignore_stack.push(matcher);
        }

        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };

        let mut children = read_dir.flatten().collect::<Vec<_>>();
        children.sort_by_key(|entry| entry.file_name());

        // Queue directories first at this level to preserve breadth-first coverage,
        // then collect files from each visited directory.
        let mut child_dirs = Vec::new();
        let mut child_files = Vec::new();

        for entry in children {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let is_dir = file_type.is_dir();
            if is_ignored_by_stack(&ignore_stack, &path, is_dir) {
                continue;
            }

            if is_dir {
                if file_type.is_symlink() {
                    continue;
                }
                child_dirs.push(path);
                continue;
            }

            if file_type.is_file() {
                child_files.push(path);
            }
        }

        for child_dir in child_dirs {
            if child_dir.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            queue.push_back((child_dir, ignore_stack.clone()));
        }

        for path in child_files {
            let relative = path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or(path.clone());
            let display = path_text(&relative);
            let file_name_lower = relative
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();

            entries.push(FileEntry {
                relative,
                display_lower: display.to_ascii_lowercase(),
                display,
                file_name_lower,
            });
        }
    }
}

fn load_local_gitignore(dir: &Path) -> Option<Gitignore> {
    let gitignore_path = dir.join(".gitignore");
    if !gitignore_path.is_file() {
        return None;
    }

    let mut builder = GitignoreBuilder::new(dir);
    let _ = builder.add(&gitignore_path);
    builder.build().ok()
}

fn is_ignored_by_stack(ignore_stack: &[Gitignore], path: &Path, is_dir: bool) -> bool {
    let mut state: Option<bool> = None;
    for matcher in ignore_stack {
        match matcher.matched(path, is_dir) {
            Match::Ignore(_) => state = Some(true),
            Match::Whitelist(_) => state = Some(false),
            Match::None => {}
        }
    }
    state.unwrap_or(false)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::{FileMentionSource, Mention};
    use crate::typeahead::provider::TypeaheadSource;

    fn wait_for_matches(source: &mut FileMentionSource, query: &str) -> Vec<Mention> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let matches = source.query(query);
            if !matches.is_empty() {
                return matches;
            }

            assert!(
                Instant::now() < deadline,
                "timed out waiting for typeahead results for query '{query}'"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn file_mentions_index_and_match() {
        let cwd = std::env::current_dir().expect("cwd");
        let mut source = FileMentionSource::new(cwd);

        let all = wait_for_matches(&mut source, "");
        assert!(
            !all.is_empty(),
            "file mention source should index at least one file"
        );

        let cargo = wait_for_matches(&mut source, "cargo");
        assert!(
            !cargo.is_empty(),
            "expected at least one match for 'cargo' in this repo"
        );
    }

    #[test]
    fn indexing_covers_both_deep_and_sibling_paths() {
        let root = std::env::temp_dir().join(format!(
            "agnt-typeahead-index-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));

        std::fs::create_dir_all(root.join("aaa/deep")).expect("create deep dir");
        std::fs::create_dir_all(root.join("io_uring")).expect("create io_uring dir");
        std::fs::write(root.join("aaa/deep/starved.txt"), "a\n").expect("write deep file");
        std::fs::write(root.join("io_uring/register.rs"), "b\n").expect("write io_uring file");

        let mut source = FileMentionSource::new(root.clone());
        let matches = wait_for_matches(&mut source, "io_uring");

        assert!(
            matches.into_iter().any(|mention| {
                let Mention::File(path) = mention;
                path.to_string_lossy().replace('\\', "/") == "io_uring/register.rs"
            }),
            "expected io_uring/register.rs to be indexed"
        );

        let deep_matches = wait_for_matches(&mut source, "starved");
        assert!(
            deep_matches.into_iter().any(|mention| {
                let Mention::File(path) = mention;
                path.to_string_lossy().replace('\\', "/") == "aaa/deep/starved.txt"
            }),
            "expected aaa/deep/starved.txt to be indexed"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_source_excludes_target_directory() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf();

        let mut source = FileMentionSource::new(workspace_root);
        let all = wait_for_matches(&mut source, "");
        for mention in all {
            let Mention::File(path) = mention;
            let text = path.to_string_lossy().replace('\\', "/");
            assert!(
                !text.starts_with("target/"),
                "target entry leaked into file source: {text}"
            );
        }
    }

    #[test]
    fn nested_gitignore_does_not_escape_its_directory_scope() {
        let root = std::env::temp_dir().join(format!(
            "agnt-typeahead-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::create_dir_all(root.join("target")).expect("create target");
        std::fs::create_dir_all(root.join(".jj")).expect("create .jj");

        std::fs::write(root.join(".gitignore"), "/target\n").expect("write root gitignore");
        std::fs::write(root.join(".jj/.gitignore"), "/*\n").expect("write nested gitignore");
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write src file");
        std::fs::write(root.join("target/should_not_match.txt"), "nope\n")
            .expect("write target file");
        std::fs::write(root.join(".jj/hidden.txt"), "nope\n").expect("write jj file");

        let mut source = FileMentionSource::new(root.clone());
        let all = wait_for_matches(&mut source, "");
        let paths = all
            .into_iter()
            .map(|mention| {
                let Mention::File(path) = mention;
                path.to_string_lossy().replace('\\', "/")
            })
            .collect::<Vec<_>>();

        assert!(
            paths.iter().any(|p| p == "src/main.rs"),
            "expected src/main.rs to be visible; got {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("target/")),
            "target directory should be ignored; got {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(".jj/")),
            ".jj nested rules should only hide .jj contents; got {paths:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
