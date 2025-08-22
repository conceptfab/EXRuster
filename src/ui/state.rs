use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::time::Instant;
use std::collections::HashMap;

#[derive(Debug)]
pub struct UiState {
    pub item_to_layer: HashMap<String, String>,
    pub display_to_real_layer: HashMap<String, String>,
    pub expanded_groups: HashMap<String, bool>,
    pub expanded_layers: HashMap<String, bool>,
    #[allow(dead_code)] // Prepared for future refactoring
    pub current_file_path: Option<PathBuf>,
    #[allow(dead_code)] // Prepared for future refactoring
    pub last_preview_log: Option<Instant>,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            item_to_layer: HashMap::new(),
            display_to_real_layer: HashMap::new(),
            expanded_groups: HashMap::new(),
            expanded_layers: HashMap::new(),
            current_file_path: None,
            last_preview_log: None,
        }
    }

    #[allow(dead_code)] // Prepared for future refactoring
    pub fn clear_layer_mappings(&mut self) {
        self.item_to_layer.clear();
        self.display_to_real_layer.clear();
    }

    pub fn is_group_expanded(&self, group_name: &str) -> bool {
        self.expanded_groups.get(group_name).copied().unwrap_or(true)
    }

    pub fn is_layer_expanded(&self, layer_name: &str) -> bool {
        self.expanded_layers.get(layer_name).copied().unwrap_or(true)
    }

    pub fn toggle_group_expansion(&mut self, group_name: &str) {
        let current = self.is_group_expanded(group_name);
        self.expanded_groups.insert(group_name.to_string(), !current);
    }

    pub fn toggle_layer_expansion(&mut self, layer_name: &str) {
        let current = self.is_layer_expanded(layer_name);
        self.expanded_layers.insert(layer_name.to_string(), !current);
    }

    #[allow(dead_code)] // Prepared for future refactoring
    pub fn insert_layer_mapping(&mut self, item: String, layer: String) {
        self.item_to_layer.insert(item, layer);
    }

    #[allow(dead_code)] // Prepared for future refactoring
    pub fn insert_display_mapping(&mut self, display: String, real: String) {
        self.display_to_real_layer.insert(display, real);
    }

    pub fn get_layer_for_item(&self, item: &str) -> Option<&String> {
        self.item_to_layer.get(item)
    }

    pub fn get_real_layer_for_display(&self, display: &str) -> Option<&String> {
        self.display_to_real_layer.get(display)
    }

    pub fn get_display_for_real_layer(&self, real_layer: &str) -> Option<String> {
        self.display_to_real_layer
            .iter()
            .find_map(|(k, v)| if v == real_layer { Some(k.clone()) } else { None })
    }

    #[allow(dead_code)] // Prepared for future refactoring
    pub fn update_last_preview_log(&mut self) {
        self.last_preview_log = Some(Instant::now());
    }

    #[allow(dead_code)] // Prepared for future refactoring
    pub fn should_log_preview(&self, min_interval_ms: u64) -> bool {
        self.last_preview_log
            .map(|t| Instant::now().duration_since(t).as_millis() >= min_interval_ms as u128)
            .unwrap_or(true)
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