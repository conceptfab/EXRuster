use crate::AppWindow;
use crate::ui::ui_handlers::{push_console, ConsoleModel};

/// Trait for UI components that can report errors consistently
pub trait UiErrorReporter {
    /// Reports an error with consistent UI updates (status text + console log)
    fn report_error(&self, console: &ConsoleModel, context: &str, error: impl std::fmt::Display);
    
    /// Reports an error with a custom status message
    fn report_error_with_status(&self, console: &ConsoleModel, context: &str, status_msg: &str, error: impl std::fmt::Display);
    
    /// Reports a simple message without error formatting
    #[allow(dead_code)]
    fn report_info(&self, console: &ConsoleModel, context: &str, message: &str);
}

impl UiErrorReporter for AppWindow {
    fn report_error(&self, console: &ConsoleModel, context: &str, error: impl std::fmt::Display) {
        let error_msg = format!("[error][{}] {}", context, error);
        let status_msg = format!("{} error: {}", context, error);
        
        push_console(self, console, error_msg);
        self.set_status_text(status_msg.into());
    }
    
    fn report_error_with_status(&self, console: &ConsoleModel, context: &str, status_msg: &str, error: impl std::fmt::Display) {
        let error_msg = format!("[error][{}] {}", context, error);
        
        push_console(self, console, error_msg);
        self.set_status_text(status_msg.into());
    }
    
    fn report_info(&self, console: &ConsoleModel, context: &str, message: &str) {
        let info_msg = format!("[{}] {}", context, message);
        push_console(self, console, info_msg);
    }
}

/// Macro for standard error handling pattern with UI updates
/// 
/// Usage:
/// ```rust
/// handle_ui_error!(result, ui, console, "histogram" => {
///     // success handling code here
/// });
/// ```
#[macro_export]
macro_rules! handle_ui_error {
    ($result:expr, $ui:expr, $console:expr, $context:expr => $success_block:block) => {
        match $result {
            Ok(value) => {
                let _value = value;
                $success_block
            },
            Err(e) => {
                use $crate::utils::error_handling::UiErrorReporter;
                $ui.report_error($console, $context, e);
            }
        }
    };
    
    ($result:expr, $ui:expr, $console:expr, $context:expr, $status_msg:expr => $success_block:block) => {
        match $result {
            Ok(value) => {
                let _value = value;
                $success_block
            },
            Err(e) => {
                use $crate::utils::error_handling::UiErrorReporter;
                $ui.report_error_with_status($console, $context, $status_msg, e);
            }
        }
    };
}

/// Macro for handling Option values with error reporting
/// 
/// Usage:
/// ```rust
/// handle_ui_option!(some_option, ui, console, "file", "No file loaded" => {
///     // success handling code here with value available
/// });
/// ```
#[macro_export]
macro_rules! handle_ui_option {
    ($option:expr, $ui:expr, $console:expr, $context:expr, $error_msg:expr => $success_block:block) => {
        match $option {
            Some(value) => {
                let _value = value;
                $success_block
            },
            None => {
                use $crate::utils::error_handling::UiErrorReporter;
                $ui.report_error($console, $context, $error_msg);
            }
        }
    };
}

/// Helper for resetting progress indicators on error
#[allow(dead_code)]
pub fn reset_progress_on_error(prog: &crate::ui::progress::UiProgress) {
    use crate::ui::progress::ProgressSink;
    prog.reset();
}

/// Standard error handling pattern that includes progress reset
#[macro_export]
macro_rules! handle_ui_error_with_progress {
    ($result:expr, $ui:expr, $console:expr, $progress:expr, $context:expr => $success_block:block) => {
        match $result {
            Ok(value) => {
                let _value = value;
                $success_block
            },
            Err(e) => {
                use $crate::utils::error_handling::{UiErrorReporter, reset_progress_on_error};
                $ui.report_error($console, $context, e);
                reset_progress_on_error($progress);
            }
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_error_formatting() {
        // Tests would go here, but we can't easily test UI components
        // without more complex setup. These macros are tested through integration.
    }
}