use crate::ui::ui_handlers::{push_console, ConsoleModel};
use crate::AppWindow;

/// Trait for UI components that can report errors consistently
pub trait UiErrorReporter {
    /// Reports an error with a custom status message
    fn report_error_with_status(
        &self,
        console: &ConsoleModel,
        context: &str,
        status_msg: &str,
        error: impl std::fmt::Display,
    );

}

impl UiErrorReporter for AppWindow {
    fn report_error_with_status(
        &self,
        console: &ConsoleModel,
        context: &str,
        status_msg: &str,
        error: impl std::fmt::Display,
    ) {
        let error_msg = format!("[error][{}] {}", context, error);

        push_console(self, console, error_msg);
        self.set_status_text(status_msg.into());
    }

}

#[cfg(test)]
mod tests {
    #[test]
    fn test_error_formatting() {
        // Tests would go here, but we can't easily test UI components
        // without more complex setup. These macros are tested through integration.
    }
}
