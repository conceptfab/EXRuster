pub mod channel_config;
pub mod error_handling;
pub mod utils;

// Re-export specific functions that are needed by other modules
pub use error_handling::UiErrorReporter;
pub use utils::{get_channel_info, human_size, normalize_channel_name, split_layer_and_short};
