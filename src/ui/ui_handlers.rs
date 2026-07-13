use crate::AppWindow;
use slint::{ComponentHandle, SharedString, VecModel, Weak};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

// Type aliases - shared across UI modules
pub type ConsoleModel = Rc<VecModel<SharedString>>;

// Utility functions - shared across UI modules

/// Max console lines before truncation from top
const MAX_CONSOLE_LINES: usize = 500;

/// Dodaje linię do modelu konsoli i aktualizuje tekst w `TextEdit` (console-text)
pub fn push_console(ui: &crate::AppWindow, _console: &ConsoleModel, line: String) {
    let mut text = ui.get_console_text().to_string();
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&line);

    // Truncate from top if exceeding max lines
    let line_count = text.bytes().filter(|&b| b == b'\n').count() + 1;
    if line_count > MAX_CONSOLE_LINES {
        let skip = line_count - MAX_CONSOLE_LINES;
        if let Some(pos) = text.match_indices('\n').nth(skip - 1).map(|(i, _)| i) {
            text = text[pos + 1..].to_string();
        }
    }

    ui.set_console_text(text.into());
}

/// Kompatybilność wsteczna - używa panic recovery
#[inline]
pub fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Obsługuje callback wyjścia z aplikacji
pub fn handle_exit(ui_handle: Weak<AppWindow>) {
    if let Some(ui) = ui_handle.upgrade() {
        let _ = ui.window().hide();
    }
}

// Re-exports from specialized modules
pub use crate::ui::image_controls::{
    handle_parameter_changed_throttled, ThrottledUpdate,
};

pub use crate::ui::thumbnails::load_thumbnails_for_directory;

pub use crate::ui::file_handlers::{handle_open_exr, handle_open_exr_from_path};
