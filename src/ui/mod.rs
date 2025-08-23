pub mod ui_handlers;
pub mod progress;
pub mod state;
pub mod layers;
pub mod image_controls;
pub mod thumbnails;
pub mod file_handlers;
pub mod setup;
pub mod export_handlers;

// Essential re-exports used by main.rs and internal modules
pub use ui_handlers::{
    push_console, lock_or_recover, handle_exit, handle_open_exr, handle_open_exr_from_path,
    handle_parameter_changed_throttled, load_thumbnails_for_directory,
    update_preview_image, ImageCacheType, CurrentFilePathType, FullExrCache, ThrottledUpdate
};
pub use export_handlers::{
    handle_export_base_layer, handle_export_layers, export_base_layer_png16, 
    export_base_layer_tiff16, export_base_layer_tiff32_float, UiExportConfig
};
pub use layers::{handle_layer_tree_click, toggle_all_layer_groups};
pub use state::{SharedUiState, create_shared_state};
pub use setup::{setup_ui_callbacks};
// Don't re-export progress - use full path crate::ui::progress::