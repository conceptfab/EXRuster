pub mod browser_handlers;
pub mod export_handlers;
pub mod file_handlers;
pub mod image_controls;
pub mod layers;
pub mod progress;
pub mod setup;
pub mod state;
pub mod thumbnails;
pub mod ui_handlers;

// Essential re-exports used by main.rs and internal modules
pub use ui_handlers::{
    handle_exit, handle_open_exr, handle_open_exr_from_path, handle_parameter_changed_throttled,
    load_thumbnails_for_directory, push_console, update_preview_image, ThrottledUpdate,
};
// Export handlers are called directly from setup.rs, not re-exported
// pub use export_handlers::{};
pub use layers::{handle_layer_tree_click, toggle_all_layer_groups};
pub use setup::setup_ui_callbacks;
pub use state::{create_shared_app_state, SharedAppState};
// Don't re-export progress - use full path crate::ui::progress::
