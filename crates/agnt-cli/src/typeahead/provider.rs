use std::marker::PhantomData;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

pub trait TypeaheadItem: Clone + Send + Sync + 'static {
    fn token_text(&self) -> String;

    fn description(&self) -> Option<String> {
        None
    }

    fn match_terms(&self) -> Vec<String> {
        vec![self.token_text()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeaheadMatchSet<T> {
    pub leader: char,
    pub query: String,
    pub token_start: usize,
    pub cursor_pos: usize,
    pub loading: bool,
    pub show_loading: bool,
    pub matches: Vec<T>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeaheadQueryResult<T> {
    pub matches: Vec<T>,
    pub loading: bool,
}

pub trait TypeaheadSource<T>: Send + 'static
where
    T: TypeaheadItem,
{
    type State: Send + 'static;

    fn init(self) -> Self::State;

    fn query(state: &mut Self::State, query: &str) -> Vec<T>;
}

#[derive(Debug, Clone)]
struct ProviderStatus<T>
where
    T: TypeaheadItem,
{
    ready: bool,
    pending_query: Option<String>,
    latest_query: Option<String>,
    latest_matches: Arc<Vec<T>>,
}

impl<T> Default for ProviderStatus<T>
where
    T: TypeaheadItem,
{
    fn default() -> Self {
        Self {
            ready: false,
            pending_query: None,
            latest_query: None,
            latest_matches: Arc::new(Vec::new()),
        }
    }
}

#[derive(Debug)]
pub struct TypeaheadProvider<T, S>
where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    leader: char,
    request_tx: Option<mpsc::UnboundedSender<String>>,
    status_rx: watch::Receiver<ProviderStatus<T>>,
    updates_rx: watch::Receiver<u64>,
    worker_handle: Option<JoinHandle<()>>,
    last_requested_query: Option<String>,
    _source: PhantomData<S>,
}

impl<T, S> TypeaheadProvider<T, S>
where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    pub fn new(leader: char, source: S) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded_channel::<String>();
        let (status_tx, status_rx) = watch::channel(ProviderStatus::<T>::default());
        let (updates_tx, updates_rx) = watch::channel(0u64);

        let handle = tokio::runtime::Handle::current();
        let worker_handle = handle.spawn(run_source_worker::<T, S>(
            source, request_rx, status_tx, updates_tx,
        ));

        Self {
            leader,
            request_tx: Some(request_tx),
            status_rx,
            updates_rx,
            worker_handle: Some(worker_handle),
            last_requested_query: None,
            _source: PhantomData,
        }
    }

    pub fn leader(&self) -> char {
        self.leader
    }

    pub fn query(&mut self, query: &str) -> TypeaheadQueryResult<T> {
        if self.last_requested_query.as_deref() != Some(query) {
            let query_owned = query.to_string();
            if let Some(request_tx) = &self.request_tx
                && request_tx.send(query_owned.clone()).is_ok()
            {
                self.last_requested_query = Some(query_owned);
            }
        }

        let status = self.status_rx.borrow();
        let waiting_for_latest = self.last_requested_query.as_deref() == Some(query)
            && status.latest_query.as_deref() != Some(query);
        let loading = !status.ready
            || (status.pending_query.is_some() && status.latest_query.as_deref() != Some(query))
            || waiting_for_latest;
        let matches = if status.latest_query.as_deref() == Some(query) {
            status.latest_matches.as_ref().clone()
        } else {
            Vec::new()
        };

        TypeaheadQueryResult { matches, loading }
    }

    pub fn updates(&self) -> watch::Receiver<u64> {
        self.updates_rx.clone()
    }

    pub async fn shutdown(&mut self) {
        self.request_tx.take();
        if let Some(worker_handle) = self.worker_handle.take() {
            let _ = worker_handle.await;
        }
    }
}

impl<T, S> Drop for TypeaheadProvider<T, S>
where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    fn drop(&mut self) {
        self.request_tx.take();
        if let Some(worker_handle) = self.worker_handle.take() {
            worker_handle.abort();
        }
    }
}

async fn run_source_worker<T, S>(
    source: S,
    mut request_rx: mpsc::UnboundedReceiver<String>,
    status_tx: watch::Sender<ProviderStatus<T>>,
    updates_tx: watch::Sender<u64>,
) where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    let mut source_state = match tokio::task::spawn_blocking(move || source.init()).await {
        Ok(state) => state,
        Err(_) => return,
    };
    let mut status = ProviderStatus {
        ready: true,
        pending_query: None,
        latest_query: None,
        latest_matches: Arc::new(Vec::new()),
    };
    let mut update_seq: u64 = 0;

    if status_tx.send(status.clone()).is_err() {
        return;
    }
    notify_update(&updates_tx, &mut update_seq);

    while let Some(mut query) = request_rx.recv().await {
        while let Ok(newer) = request_rx.try_recv() {
            query = newer;
        }

        status.pending_query = Some(query.clone());
        if status_tx.send(status.clone()).is_err() {
            return;
        }
        notify_update(&updates_tx, &mut update_seq);

        let state_in = source_state;
        let query_for_worker = query.clone();
        let query_result = tokio::task::spawn_blocking(move || {
            let mut state = state_in;
            let matches = S::query(&mut state, &query_for_worker);
            (state, matches)
        })
        .await;
        let (next_state, matches) = match query_result {
            Ok(out) => out,
            Err(_) => return,
        };
        source_state = next_state;

        status.pending_query = None;
        status.latest_query = Some(query);
        status.latest_matches = Arc::new(matches);
        if status_tx.send(status.clone()).is_err() {
            return;
        }
        notify_update(&updates_tx, &mut update_seq);
    }
}

fn notify_update(updates_tx: &watch::Sender<u64>, update_seq: &mut u64) {
    *update_seq = update_seq.wrapping_add(1);
    let _ = updates_tx.send(*update_seq);
}

#[derive(Debug, Clone)]
struct CachedEntry<T> {
    item: T,
    terms_lower: Vec<String>,
}

#[derive(Debug, Clone)]
struct FilterCache {
    query: String,
    indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct CachedPrefixSource<T>
where
    T: TypeaheadItem,
{
    entries: Vec<CachedEntry<T>>,
    cache: FilterCache,
}

impl<T> From<Vec<T>> for CachedPrefixSource<T>
where
    T: TypeaheadItem,
{
    fn from(items: Vec<T>) -> Self {
        let entries = items
            .into_iter()
            .map(|item| {
                let terms_lower = item
                    .match_terms()
                    .into_iter()
                    .map(|term| term.to_ascii_lowercase())
                    .collect();
                CachedEntry { item, terms_lower }
            })
            .collect::<Vec<_>>();

        Self {
            cache: FilterCache {
                query: String::new(),
                indices: (0..entries.len()).collect(),
            },
            entries,
        }
    }
}

impl<T> TypeaheadSource<T> for CachedPrefixSource<T>
where
    T: TypeaheadItem,
{
    type State = Self;

    fn init(self) -> Self::State {
        self
    }

    fn query(state: &mut Self::State, query: &str) -> Vec<T> {
        let normalized = query.to_ascii_lowercase();

        if normalized == state.cache.query {
            return state
                .cache
                .indices
                .iter()
                .map(|index| state.entries[*index].item.clone())
                .collect();
        }

        let growing = normalized.starts_with(&state.cache.query);
        let candidate_indices = if growing {
            state.cache.indices.clone()
        } else {
            (0..state.entries.len()).collect()
        };

        let mut filtered = Vec::new();
        for index in candidate_indices {
            if matches_cached_entry(&state.entries[index], &normalized) {
                filtered.push(index);
            }
        }

        state.cache.query = normalized;
        state.cache.indices = filtered;
        state
            .cache
            .indices
            .iter()
            .map(|index| state.entries[*index].item.clone())
            .collect()
    }
}

fn matches_cached_entry<T>(entry: &CachedEntry<T>, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    entry.terms_lower.iter().any(|term| term.starts_with(query))
}

pub fn extract_query_token(cursor_prefix: &str) -> Option<(usize, char, &str)> {
    let token_start = cursor_prefix
        .char_indices()
        .rfind(|(_, c)| c.is_whitespace())
        .map(|(idx, c)| idx + c.len_utf8())
        .unwrap_or(0);

    let token = &cursor_prefix[token_start..];
    if token.is_empty() {
        return None;
    }

    let mut chars = token.chars();
    let leader = chars.next()?;
    let query = chars.as_str();

    Some((token_start, leader, query))
}
