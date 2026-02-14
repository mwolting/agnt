use super::provider::TypeaheadItem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    NewSession,
}

impl TypeaheadItem for Command {
    fn token_text(&self) -> String {
        match self {
            Command::NewSession => "new".to_string(),
        }
    }

    fn description(&self) -> Option<String> {
        match self {
            Command::NewSession => Some("Create a new session".to_string()),
        }
    }

    fn match_terms(&self) -> Vec<String> {
        match self {
            Command::NewSession => vec!["new".to_string(), "session".to_string()],
        }
    }
}
