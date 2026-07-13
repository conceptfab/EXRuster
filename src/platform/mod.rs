#[cfg(target_os = "windows")]
pub mod platform_win;

#[cfg(target_os = "windows")]
pub use platform_win::try_set_runtime_window_icon;

#[cfg(target_os = "macos")]
pub mod platform_mac;

#[cfg(target_os = "macos")]
pub use platform_mac::try_make_windows_opaque;
