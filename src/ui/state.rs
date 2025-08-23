use std::sync::{Arc, Mutex};
use std::collections::HashMap;

#[derive(Debug)]
pub struct UiState {
    pub expanded_groups: HashMap<String, bool>,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            expanded_groups: HashMap::new(),
        }
    }


    pub fn is_group_expanded(&self, group_name: &str) -> bool {
        self.expanded_groups.get(group_name).copied().unwrap_or(true)
    }

    pub fn toggle_group_expansion(&mut self, group_name: &str) {
        let current = self.is_group_expanded(group_name);
        self.expanded_groups.insert(group_name.to_string(), !current);
    }

    pub fn set_group_expansion(&mut self, group_name: &str, expanded: bool) {
        self.expanded_groups.insert(group_name.to_string(), expanded);
    }






}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedUiState = Arc<Mutex<UiState>>;

pub fn create_shared_state() -> SharedUiState {
    Arc::new(Mutex::new(UiState::new()))
}