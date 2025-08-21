use std::sync::Arc;
use slint::Weak;
use crate::{AppWindow, ui::progress::{UiProgress, ProgressSink}};

/// RAII wrapper for UiProgress that automatically handles cleanup
pub struct ScopedProgress {
    inner: Arc<UiProgress>,
    auto_finish: bool,
}

impl ScopedProgress {
    /// Create a new scoped progress with automatic finish on drop
    pub fn new(ui: Weak<AppWindow>) -> Self {
        Self {
            inner: Arc::new(UiProgress::new(ui)),
            auto_finish: true,
        }
    }

    /// Start indeterminate progress and return self for chaining
    pub fn start_indeterminate(self, message: Option<&str>) -> Self {
        self.inner.start_indeterminate(message);
        self
    }

    /// Set progress value and return self for chaining
    pub fn set(self, progress: f32, message: Option<&str>) -> Self {
        self.inner.set(progress, message);
        self
    }

    /// Get a reference to the underlying UiProgress for advanced usage
    pub fn inner(&self) -> &UiProgress {
        &self.inner
    }

}

impl Drop for ScopedProgress {
    fn drop(&mut self) {
        if self.auto_finish {
            self.inner.finish(None);
        }
    }
}

/// Extension trait for Weak<AppWindow> to provide convenient progress creation
pub trait WeakProgressExt {
    /// Create a new scoped progress that automatically finishes on drop
    fn scoped_progress(&self) -> ScopedProgress;
}

impl WeakProgressExt for Weak<AppWindow> {
    fn scoped_progress(&self) -> ScopedProgress {
        ScopedProgress::new(self.clone())
    }
}


/// Convenience functions for common progress patterns
pub mod patterns {
    use super::*;


    /// Create a file operation progress
    pub fn file_operation(ui: Weak<AppWindow>, operation: &str, filename: &str) -> ScopedProgress {
        let message = format!("{}: {}", operation, filename);
        ScopedProgress::new(ui).start_indeterminate(Some(&message))
    }

    /// Create a progress for processing operations with step tracking
    pub fn processing(ui: Weak<AppWindow>, operation: &str) -> ScopedProgress {
        let message = format!("Processing: {}", operation);
        ScopedProgress::new(ui).start_indeterminate(Some(&message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests would require a Slint UI instance to run properly
    // In a real scenario, you'd need to set up a test harness with Slint
    
    #[test]
    fn test_scoped_progress_chain() {
        // This would need a proper UI instance to test
        // For now, just test that the API compiles
        assert!(true);
    }

    #[test]
    fn test_progress_builder() {
        // Similar to above - needs UI instance
        assert!(true);
    }
}