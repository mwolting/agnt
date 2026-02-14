use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::typeahead::{
    CachedPrefixSource, Command, FileMentionSource, Mention, TypeaheadItem, TypeaheadMatchSet,
    TypeaheadProvider, TypeaheadSource, extract_query_token,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TriggerToken {
    leader: char,
    token_start: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveTypeahead {
    Command(TypeaheadMatchSet<Command>),
    Mention(TypeaheadMatchSet<Mention>),
}

impl ActiveTypeahead {
    pub fn match_count(&self) -> usize {
        match self {
            ActiveTypeahead::Command(set) => set.matches.len(),
            ActiveTypeahead::Mention(set) => set.matches.len(),
        }
    }

    fn token_start(&self) -> usize {
        match self {
            ActiveTypeahead::Command(set) => set.token_start,
            ActiveTypeahead::Mention(set) => set.token_start,
        }
    }

    fn cursor_pos(&self) -> usize {
        match self {
            ActiveTypeahead::Command(set) => set.cursor_pos,
            ActiveTypeahead::Mention(set) => set.cursor_pos,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeaheadActivation {
    Command {
        command: Command,
        token_start: usize,
        token_end: usize,
    },
    Mention {
        mention: Mention,
        token_start: usize,
        token_end: usize,
    },
}

pub struct TypeaheadState {
    selected_index: usize,
    window_start: usize,
    command_typeahead: TypeaheadProvider<Command, CachedPrefixSource<Command>>,
    mention_typeahead: TypeaheadProvider<Mention, FileMentionSource>,
    last_presented_command: Option<TypeaheadMatchSet<Command>>,
    last_presented_mention: Option<TypeaheadMatchSet<Mention>>,
    trigger_seq: u64,
    suppressed_seq: Option<u64>,
    last_trigger_token: Option<TriggerToken>,
    loading_indicator: Option<LoadingIndicatorState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadingIndicatorState {
    leader: char,
    token_start: usize,
    since: Instant,
    visible: bool,
}

const LOADING_TEXT_DELAY: Duration = Duration::from_millis(100);
const VISIBLE_MATCH_LIMIT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeaheadWindowItem {
    Value(usize),
    Divider,
}

pub fn build_typeahead_window_items(
    match_count: usize,
    window_start: usize,
    window_slots: usize,
) -> Vec<TypeaheadWindowItem> {
    if match_count == 0 || window_slots == 0 {
        return Vec::new();
    }

    let include_divider = match_count > window_slots;
    let total_rows = match_count + usize::from(include_divider);
    let start = window_start % total_rows;
    let visible_rows = window_slots.min(total_rows);

    (0..visible_rows)
        .map(|offset| {
            let row_index = (start + offset) % total_rows;
            if include_divider && row_index == match_count {
                TypeaheadWindowItem::Divider
            } else {
                TypeaheadWindowItem::Value(row_index)
            }
        })
        .collect()
}

impl TypeaheadState {
    pub fn new_for_current_project() -> Self {
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(project_root)
    }

    pub fn new(project_root: PathBuf) -> Self {
        let command_source: CachedPrefixSource<Command> = vec![Command::NewSession].into();
        let command_typeahead = TypeaheadProvider::new('/', command_source);
        let mention_typeahead = TypeaheadProvider::new('@', FileMentionSource::new(project_root));

        Self {
            selected_index: 0,
            window_start: 0,
            command_typeahead,
            mention_typeahead,
            last_presented_command: None,
            last_presented_mention: None,
            trigger_seq: 0,
            suppressed_seq: None,
            last_trigger_token: None,
            loading_indicator: None,
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn window_start(&self) -> usize {
        self.window_start
    }

    pub fn sync(&mut self, input: &str, cursor_pos: usize) {
        let current = current_trigger_token(input, cursor_pos);
        if current != self.last_trigger_token {
            if current.is_some() {
                self.trigger_seq = self.trigger_seq.wrapping_add(1);
                self.selected_index = 0;
                self.window_start = 0;
            } else {
                self.selected_index = 0;
                self.window_start = 0;
                self.suppressed_seq = None;
            }
            self.last_trigger_token = current;
        }
    }

    pub fn visible_matches(&mut self, input: &str, cursor_pos: usize) -> Option<ActiveTypeahead> {
        self.sync(input, cursor_pos);
        if self.suppressed_seq == Some(self.trigger_seq) {
            self.loading_indicator = None;
            return None;
        }

        let prefix = &input[..cursor_pos.min(input.len())];
        let (token_start, leader, query) = extract_query_token(prefix)?;
        let mut active = match leader {
            '/' => Some(ActiveTypeahead::Command(build_match_set(
                &mut self.command_typeahead,
                query,
                token_start,
                prefix.len(),
            ))),
            '@' => Some(ActiveTypeahead::Mention(build_match_set(
                &mut self.mention_typeahead,
                query,
                token_start,
                prefix.len(),
            ))),
            _ => None,
        };

        if let Some(active) = active.as_mut() {
            self.apply_loading_delay(active);
            self.stabilize_loading_transition(active);
        } else {
            self.loading_indicator = None;
        }

        if let Some(active) = &active {
            let count = active.match_count();
            if count == 0 {
                self.selected_index = 0;
            } else if self.selected_index >= count {
                self.selected_index = count - 1;
            }
            self.normalize_window_for_count(count);
        }

        active
    }

    fn apply_loading_delay(&mut self, active: &mut ActiveTypeahead) {
        let (leader, token_start, loading, show_loading) = match active {
            ActiveTypeahead::Command(set) => (
                set.leader,
                set.token_start,
                &set.loading,
                &mut set.show_loading,
            ),
            ActiveTypeahead::Mention(set) => (
                set.leader,
                set.token_start,
                &set.loading,
                &mut set.show_loading,
            ),
        };

        if !*loading {
            *show_loading = false;
            self.loading_indicator = None;
            return;
        }

        let now = Instant::now();
        let same_state = self
            .loading_indicator
            .as_ref()
            .is_some_and(|state| state.leader == leader && state.token_start == token_start);

        if !same_state {
            self.loading_indicator = Some(LoadingIndicatorState {
                leader,
                token_start,
                since: now,
                visible: false,
            });
            *show_loading = false;
            return;
        }

        let Some(state) = self.loading_indicator.as_mut() else {
            *show_loading = false;
            return;
        };
        if state.visible {
            *show_loading = true;
            return;
        }

        if now.duration_since(state.since) >= LOADING_TEXT_DELAY {
            state.visible = true;
            *show_loading = true;
        } else {
            *show_loading = false;
        }
    }

    fn stabilize_loading_transition(&mut self, active: &mut ActiveTypeahead) {
        match active {
            ActiveTypeahead::Command(set) => {
                stabilize_set_during_loading_delay(&mut self.last_presented_command, set);
            }
            ActiveTypeahead::Mention(set) => {
                stabilize_set_during_loading_delay(&mut self.last_presented_mention, set);
            }
        }
    }

    pub fn dismiss(&mut self, input: &str, cursor_pos: usize) {
        self.sync(input, cursor_pos);
        if self.last_trigger_token.is_some() {
            self.suppressed_seq = Some(self.trigger_seq);
        }
    }

    pub fn move_selection(&mut self, direction: i32, input: &str, cursor_pos: usize) {
        let Some(active) = self.visible_matches(input, cursor_pos) else {
            return;
        };
        let count = active.match_count();
        if count == 0 {
            return;
        }

        if direction < 0 {
            if self.selected_index == 0 {
                self.selected_index = count - 1;
            } else {
                self.selected_index -= 1;
            }
        } else if self.selected_index + 1 >= count {
            self.selected_index = 0;
        } else {
            self.selected_index += 1;
        }

        self.update_window_for_move(direction, count);
    }

    pub fn activate_selected(
        &mut self,
        input: &str,
        cursor_pos: usize,
    ) -> Option<TypeaheadActivation> {
        let active = self.visible_matches(input, cursor_pos)?;
        let count = active.match_count();
        if count == 0 {
            return None;
        }

        let selected = self.selected_index.min(count - 1);
        let token_start = active.token_start();
        let token_end = active.cursor_pos();
        let activation = match active {
            ActiveTypeahead::Command(set) => {
                let command = set.matches.get(selected)?.clone();
                TypeaheadActivation::Command {
                    command,
                    token_start,
                    token_end,
                }
            }
            ActiveTypeahead::Mention(set) => {
                let mention = set.matches.get(selected)?.clone();
                TypeaheadActivation::Mention {
                    mention,
                    token_start,
                    token_end,
                }
            }
        };
        self.selected_index = 0;
        self.window_start = 0;
        self.suppressed_seq = Some(self.trigger_seq);
        Some(activation)
    }

    fn normalize_window_for_count(&mut self, count: usize) {
        if count == 0 {
            self.window_start = 0;
            return;
        }

        let total_rows = count + usize::from(count > VISIBLE_MATCH_LIMIT);
        if total_rows <= VISIBLE_MATCH_LIMIT {
            self.window_start = 0;
            return;
        }

        self.window_start %= total_rows;
        let mut guard = 0usize;
        while !selected_is_visible(count, self.window_start, self.selected_index)
            && guard < total_rows
        {
            self.window_start = (self.window_start + 1) % total_rows;
            guard += 1;
        }
    }

    fn update_window_for_move(&mut self, direction: i32, count: usize) {
        let total_rows = count + usize::from(count > VISIBLE_MATCH_LIMIT);
        if total_rows <= VISIBLE_MATCH_LIMIT {
            self.window_start = 0;
            return;
        }

        self.window_start %= total_rows;
        let step = if direction < 0 { total_rows - 1 } else { 1 };
        let mut guard = 0usize;
        while !selected_is_visible(count, self.window_start, self.selected_index)
            && guard < total_rows
        {
            self.window_start = (self.window_start + step) % total_rows;
            guard += 1;
        }
    }

    pub fn updates(&self) -> [watch::Receiver<u64>; 2] {
        [
            self.command_typeahead.updates(),
            self.mention_typeahead.updates(),
        ]
    }

    pub async fn shutdown(&mut self) {
        self.command_typeahead.shutdown().await;
        self.mention_typeahead.shutdown().await;
    }
}

fn build_match_set<T, S>(
    provider: &mut TypeaheadProvider<T, S>,
    query: &str,
    token_start: usize,
    cursor_pos: usize,
) -> TypeaheadMatchSet<T>
where
    T: TypeaheadItem,
    S: TypeaheadSource<T>,
{
    let result = provider.query(query);
    TypeaheadMatchSet {
        leader: provider.leader(),
        query: query.to_string(),
        token_start,
        cursor_pos,
        loading: result.loading,
        show_loading: result.loading,
        matches: result.matches,
    }
}

fn stabilize_set_during_loading_delay<T: TypeaheadItem>(
    previous_set: &mut Option<TypeaheadMatchSet<T>>,
    set: &mut TypeaheadMatchSet<T>,
) {
    let delaying_loading = set.loading && !set.show_loading && set.matches.is_empty();
    if delaying_loading
        && let Some(previous) = previous_set.as_ref()
        && previous.token_start == set.token_start
    {
        set.matches = previous.matches.clone();
    }

    if !set.loading || !set.matches.is_empty() {
        *previous_set = Some(set.clone());
    }
}

fn current_trigger_token(input: &str, cursor_pos: usize) -> Option<TriggerToken> {
    let prefix = &input[..cursor_pos.min(input.len())];
    let (token_start, leader, _) = extract_query_token(prefix)?;
    match leader {
        '/' | '@' => Some(TriggerToken {
            leader,
            token_start,
        }),
        _ => None,
    }
}

fn selected_is_visible(count: usize, window_start: usize, selected_index: usize) -> bool {
    build_typeahead_window_items(count, window_start, VISIBLE_MATCH_LIMIT)
        .into_iter()
        .any(|row| matches!(row, TypeaheadWindowItem::Value(index) if index == selected_index))
}
