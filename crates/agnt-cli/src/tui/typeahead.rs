use std::path::PathBuf;

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
    command_typeahead: TypeaheadProvider<Command, CachedPrefixSource<Command>>,
    mention_typeahead: TypeaheadProvider<Mention, FileMentionSource>,
    trigger_seq: u64,
    suppressed_seq: Option<u64>,
    last_trigger_token: Option<TriggerToken>,
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
            command_typeahead,
            mention_typeahead,
            trigger_seq: 0,
            suppressed_seq: None,
            last_trigger_token: None,
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn sync(&mut self, input: &str, cursor_pos: usize) {
        let current = current_trigger_token(input, cursor_pos);
        if current != self.last_trigger_token {
            if current.is_some() {
                self.trigger_seq = self.trigger_seq.wrapping_add(1);
                self.selected_index = 0;
            } else {
                self.selected_index = 0;
                self.suppressed_seq = None;
            }
            self.last_trigger_token = current;
        }
    }

    pub fn visible_matches(&mut self, input: &str, cursor_pos: usize) -> Option<ActiveTypeahead> {
        self.sync(input, cursor_pos);
        if self.suppressed_seq == Some(self.trigger_seq) {
            return None;
        }

        let prefix = &input[..cursor_pos.min(input.len())];
        let (token_start, leader, query) = extract_query_token(prefix)?;
        let active = match leader {
            '/' => Some(ActiveTypeahead::Command(build_match_set(
                &mut self.command_typeahead,
                query,
                token_start,
                prefix.len(),
            ))),
            '@' => {
                let matches = self.mention_typeahead.query(query);
                let loading = self.mention_typeahead.source().loading_for_query(query);
                Some(ActiveTypeahead::Mention(TypeaheadMatchSet {
                    leader: self.mention_typeahead.leader(),
                    query: query.to_string(),
                    token_start,
                    cursor_pos: prefix.len(),
                    loading,
                    matches,
                }))
            }
            _ => None,
        };

        if let Some(active) = &active {
            let count = active.match_count();
            if count == 0 {
                self.selected_index = 0;
            } else if self.selected_index >= count {
                self.selected_index = count - 1;
            }
        }

        active
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
        self.suppressed_seq = Some(self.trigger_seq);
        Some(activation)
    }

    pub fn has_background_work(&self, input: &str, cursor_pos: usize) -> bool {
        let prefix = &input[..cursor_pos.min(input.len())];
        let Some((_, leader, _)) = extract_query_token(prefix) else {
            return false;
        };

        match leader {
            '@' => self.mention_typeahead.source().has_pending_work(),
            _ => false,
        }
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
    TypeaheadMatchSet {
        leader: provider.leader(),
        query: query.to_string(),
        token_start,
        cursor_pos,
        loading: false,
        matches: provider.query(query),
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
