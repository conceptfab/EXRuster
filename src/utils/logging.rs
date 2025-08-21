#![allow(dead_code)]

use crate::AppWindow;
use crate::ui::ui_handlers::{push_console, ConsoleModel};

/// Log levels for console messages
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
    Success,
}

impl LogLevel {
    fn as_tag(&self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warning => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Success => "success",
        }
    }
}

/// Builder for creating formatted log messages
pub struct LogMessage {
    level: LogLevel,
    context: Option<String>,
    message: String,
    update_status: bool,
}

impl LogMessage {
    /// Create a new log message with the specified level
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            context: None,
            message: message.into(),
            update_status: false,
        }
    }
    
    /// Add context to the log message (e.g., "histogram", "file_load")
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
    
    /// Also update the UI status text with this message
    pub fn with_status_update(mut self) -> Self {
        self.update_status = true;
        self
    }
    
    /// Format the message for console output
    fn format_for_console(&self) -> String {
        match &self.context {
            Some(ctx) => format!("[{}][{}] {}", self.level.as_tag(), ctx, self.message),
            None => format!("[{}] {}", self.level.as_tag(), self.message),
        }
    }
    
    /// Format the message for status text (cleaner format)
    fn format_for_status(&self) -> String {
        match &self.context {
            Some(ctx) => format!("{}: {}", ctx, self.message),
            None => self.message.clone(),
        }
    }
}

/// Trait for components that support console logging
pub trait ConsoleLogger {
    /// Log an error message
    fn log_error<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a>;
    
    /// Log an info message
    fn log_info<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a>;
    
    /// Log a warning message
    fn log_warning<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a>;
    
    /// Log a success message
    fn log_success<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a>;
    
    /// Log a debug message
    fn log_debug<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a>;
    
    /// Send a pre-built log message
    fn send_log(&self, console: &ConsoleModel, log_message: LogMessage);
}

/// Builder that allows chaining configuration before sending the log
pub struct LogMessageBuilder<'a> {
    ui: &'a AppWindow,
    console: &'a ConsoleModel,
    message: LogMessage,
}

impl<'a> LogMessageBuilder<'a> {
    fn new(ui: &'a AppWindow, console: &'a ConsoleModel, message: LogMessage) -> Self {
        Self { ui, console, message }
    }
    
    /// Add context to the log message
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.message = self.message.with_context(context);
        self
    }
    
    /// Also update the UI status text
    pub fn with_status_update(mut self) -> Self {
        self.message = self.message.with_status_update();
        self
    }
    
    /// Send the log message to console (and optionally status)
    pub fn send(self) {
        self.ui.send_log(self.console, self.message);
    }
}

impl ConsoleLogger for AppWindow {
    fn log_error<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a> {
        let log_message = LogMessage::new(LogLevel::Error, message);
        LogMessageBuilder::new(self, console, log_message)
    }
    
    fn log_info<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a> {
        let log_message = LogMessage::new(LogLevel::Info, message);
        LogMessageBuilder::new(self, console, log_message)
    }
    
    fn log_warning<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a> {
        let log_message = LogMessage::new(LogLevel::Warning, message);
        LogMessageBuilder::new(self, console, log_message)
    }
    
    fn log_success<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a> {
        let log_message = LogMessage::new(LogLevel::Success, message);
        LogMessageBuilder::new(self, console, log_message)
    }
    
    fn log_debug<'a>(&'a self, console: &'a ConsoleModel, message: impl Into<String>) -> LogMessageBuilder<'a> {
        let log_message = LogMessage::new(LogLevel::Debug, message);
        LogMessageBuilder::new(self, console, log_message)
    }
    
    fn send_log(&self, console: &ConsoleModel, log_message: LogMessage) {
        let console_text = log_message.format_for_console();
        push_console(self, console, console_text);
        
        if log_message.update_status {
            let status_text = log_message.format_for_status();
            self.set_status_text(status_text.into());
        }
    }
}

/// Extension methods for easier logging without builder pattern
pub trait LoggingExt {
    /// Quick error logging with context and status update
    fn log_error_ctx(&self, console: &ConsoleModel, context: &str, message: impl Into<String>);
    
    /// Quick info logging with context
    fn log_info_ctx(&self, console: &ConsoleModel, context: &str, message: impl Into<String>);
    
    /// Quick success logging with context and status update
    fn log_success_ctx(&self, console: &ConsoleModel, context: &str, message: impl Into<String>);
}

impl LoggingExt for AppWindow {
    fn log_error_ctx(&self, console: &ConsoleModel, context: &str, message: impl Into<String>) {
        self.log_error(console, message)
            .with_context(context)
            .with_status_update()
            .send();
    }
    
    fn log_info_ctx(&self, console: &ConsoleModel, context: &str, message: impl Into<String>) {
        self.log_info(console, message)
            .with_context(context)
            .send();
    }
    
    fn log_success_ctx(&self, console: &ConsoleModel, context: &str, message: impl Into<String>) {
        self.log_success(console, message)
            .with_context(context)
            .with_status_update()
            .send();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_log_message_formatting() {
        let msg = LogMessage::new(LogLevel::Error, "Test error")
            .with_context("test");
        
        assert_eq!(msg.format_for_console(), "[error][test] Test error");
        assert_eq!(msg.format_for_status(), "test: Test error");
    }
    
    #[test]
    fn test_log_message_no_context() {
        let msg = LogMessage::new(LogLevel::Info, "Test info");
        
        assert_eq!(msg.format_for_console(), "[info] Test info");
        assert_eq!(msg.format_for_status(), "Test info");
    }
    
    #[test]
    fn test_log_levels() {
        assert_eq!(LogLevel::Error.as_tag(), "error");
        assert_eq!(LogLevel::Warning.as_tag(), "warn");
        assert_eq!(LogLevel::Info.as_tag(), "info");
        assert_eq!(LogLevel::Debug.as_tag(), "debug");
        assert_eq!(LogLevel::Success.as_tag(), "success");
    }
}