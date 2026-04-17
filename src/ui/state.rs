use crate::io::full_exr_cache::FullExrCacheData;
use crate::io::image_cache::ImageCache;
use crate::processing::channel_classification::ChannelGroupConfig;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct UiState {
    pub expanded_groups: HashMap<String, bool>,
}

#[derive(Default)]
pub struct AppState {
    pub image_cache: Option<ImageCache>,
    pub current_file_path: Option<PathBuf>,
    pub full_exr_cache: Option<Arc<FullExrCacheData>>,
    pub ui_state: UiState,
    pub channel_config: Option<ChannelGroupConfig>,
    pub current_browsed_folder: Option<PathBuf>,
}


impl UiState {
    pub fn new() -> Self {
        Self {
            expanded_groups: HashMap::new(),
        }
    }

    pub fn is_group_expanded(&self, group_name: &str) -> bool {
        self.expanded_groups
            .get(group_name)
            .copied()
            .unwrap_or(true)
    }

    pub fn toggle_group_expansion(&mut self, group_name: &str) {
        let current = self.is_group_expanded(group_name);
        self.expanded_groups
            .insert(group_name.to_string(), !current);
    }

    pub fn set_group_expansion(&mut self, group_name: &str, expanded: bool) {
        self.expanded_groups
            .insert(group_name.to_string(), expanded);
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedAppState = Arc<RwLock<AppState>>;

pub fn create_shared_app_state() -> SharedAppState {
    Arc::new(RwLock::new(AppState::default()))
}
