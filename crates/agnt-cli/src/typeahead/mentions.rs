use std::collections::VecDeque;
use std::path::{Path, PathBuf};

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
        None
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

#[derive(Debug, Clone)]
pub struct FileMentionSource {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FileMentionState {
    entries: Vec<FileEntry>,
    cache: FilterCache,
}

impl FileMentionSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl TypeaheadSource<Mention> for FileMentionSource {
    type State = FileMentionState;

    fn init(self) -> Self::State {
        let entries = collect_file_entries(&self.root);
        let cache = FilterCache {
            query: String::new(),
            indices: (0..entries.len()).collect(),
        };

        FileMentionState { entries, cache }
    }

    fn query(state: &mut Self::State, query: &str) -> Vec<Mention> {
        let normalized = query.to_ascii_lowercase();
        find_matches(&state.entries, &mut state.cache, &normalized)
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
    use crate::typeahead::TypeaheadProvider;

    async fn wait_until_ready(
        provider: &mut TypeaheadProvider<Mention, FileMentionSource>,
        query: &str,
    ) -> Vec<Mention> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let result = provider.query(query);
            if !result.loading {
                return result.matches;
            }

            assert!(
                Instant::now() < deadline,
                "timed out waiting for typeahead readiness for query '{query}'"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn file_mentions_index_and_match() {
        let cwd = std::env::current_dir().expect("cwd");
        let mut provider = TypeaheadProvider::new('@', FileMentionSource::new(cwd));

        let all = wait_until_ready(&mut provider, "").await;
        assert!(
            !all.is_empty(),
            "file mention source should index at least one file"
        );

        let cargo = wait_until_ready(&mut provider, "cargo").await;
        assert!(
            !cargo.is_empty(),
            "expected at least one match for 'cargo' in this repo"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn indexing_covers_both_deep_and_sibling_paths() {
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

        let mut provider = TypeaheadProvider::new('@', FileMentionSource::new(root.clone()));
        let matches = wait_until_ready(&mut provider, "io_uring").await;

        assert!(
            matches.into_iter().any(|mention| {
                let Mention::File(path) = mention;
                path.to_string_lossy().replace('\\', "/") == "io_uring/register.rs"
            }),
            "expected io_uring/register.rs to be indexed"
        );

        let deep_matches = wait_until_ready(&mut provider, "starved").await;
        assert!(
            deep_matches.into_iter().any(|mention| {
                let Mention::File(path) = mention;
                path.to_string_lossy().replace('\\', "/") == "aaa/deep/starved.txt"
            }),
            "expected aaa/deep/starved.txt to be indexed"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn file_source_excludes_target_directory() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf();

        let mut provider = TypeaheadProvider::new('@', FileMentionSource::new(workspace_root));
        let all = wait_until_ready(&mut provider, "").await;
        for mention in all {
            let Mention::File(path) = mention;
            let text = path.to_string_lossy().replace('\\', "/");
            assert!(
                !text.starts_with("target/"),
                "target entry leaked into file source: {text}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nested_gitignore_does_not_escape_its_directory_scope() {
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

        let mut provider = TypeaheadProvider::new('@', FileMentionSource::new(root.clone()));
        let all = wait_until_ready(&mut provider, "").await;
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
