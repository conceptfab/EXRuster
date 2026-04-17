//! Minimal logging macros used in place of ad-hoc `println!`/`eprintln!`.
//!
//! All runtime diagnostics go through these three macros so the prefix
//! (`[INFO]`, `[WARN]`, `[ERROR]`) is consistent across modules, and a
//! future switch to a real logging crate (tracing/log) has a single point
//! of change. Tests keep using raw `println!` so expected stdout is not
//! disturbed.
//!
//! The macros are propagated crate-wide via `#[macro_use]` on the module
//! declarations, so they are callable by bare name from any file.

#[macro_export]
macro_rules! log_info {
    ($($t:tt)*) => { eprintln!("[INFO] {}", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($t:tt)*) => { eprintln!("[WARN] {}", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($t:tt)*) => { eprintln!("[ERROR] {}", format_args!($($t)*)) };
}
