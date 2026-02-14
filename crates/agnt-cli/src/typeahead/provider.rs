use std::marker::PhantomData;

pub trait TypeaheadItem: Clone {
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
    pub matches: Vec<T>,
}

pub trait TypeaheadSource<T: TypeaheadItem> {
    fn query(&mut self, query: &str) -> Vec<T>;
}

#[derive(Debug, Clone)]
pub struct TypeaheadProvider<T, S>
where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    leader: char,
    source: S,
    _item: PhantomData<T>,
}

impl<T, S> TypeaheadProvider<T, S>
where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    pub fn new(leader: char, source: S) -> Self {
        Self {
            leader,
            source,
            _item: PhantomData,
        }
    }

    pub fn leader(&self) -> char {
        self.leader
    }

    pub fn query(&mut self, query: &str) -> Vec<T> {
        self.source.query(query)
    }

    pub fn source(&self) -> &S {
        &self.source
    }
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
    fn query(&mut self, query: &str) -> Vec<T> {
        let normalized = query.to_ascii_lowercase();

        if normalized == self.cache.query {
            return self
                .cache
                .indices
                .iter()
                .map(|index| self.entries[*index].item.clone())
                .collect();
        }

        let growing = normalized.starts_with(&self.cache.query);
        let candidate_indices = if growing {
            self.cache.indices.clone()
        } else {
            (0..self.entries.len()).collect()
        };

        let mut filtered = Vec::new();
        for index in candidate_indices {
            if matches_cached_entry(&self.entries[index], &normalized) {
                filtered.push(index);
            }
        }

        self.cache.query = normalized;
        self.cache.indices = filtered;
        self.cache
            .indices
            .iter()
            .map(|index| self.entries[*index].item.clone())
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
