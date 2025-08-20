use slint::{VecModel, SharedString, Weak, ComponentHandle};
use std::sync::{Arc, Mutex, MutexGuard};
use std::path::PathBuf;
use std::rc::Rc;
use crate::io::image_cache::ImageCache;
use anyhow::Result;
use crate::AppWindow;

// Type aliases - shared across UI modules
pub type ImageCacheType = Arc<Mutex<Option<ImageCache>>>;
pub type CurrentFilePathType = Arc<Mutex<Option<PathBuf>>>;
pub type ConsoleModel = Rc<VecModel<SharedString>>;
use crate::io::full_exr_cache::FullExrCacheData;
pub type FullExrCache = Arc<Mutex<Option<std::sync::Arc<FullExrCacheData>>>>;


// Utility functions - shared across UI modules

/// Dodaje linię do modelu konsoli i aktualizuje tekst w `TextEdit` (console-text)
pub fn push_console(ui: &crate::AppWindow, console: &ConsoleModel, line: String) {
    console.push(line.clone().into());
    let mut joined = ui.get_console_text().to_string();
    if !joined.is_empty() { joined.push('\n'); }
    joined.push_str(&line);
    ui.set_console_text(joined.into());
}

/// Kompatybilność wsteczna - używa panic recovery
#[inline]
pub fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Standardowy wzorzec dla bezpiecznego dostępu do Mutex z kontekstem błędu
#[allow(dead_code)]
#[inline]
pub(crate) fn safe_lock<'a, T>(mutex: &'a Arc<Mutex<T>>, context: &'static str) -> Result<MutexGuard<'a, T>> {
    mutex.lock()
        .map_err(|_| anyhow::anyhow!("Mutex poisoned: {}", context))
}

/// Obsługuje callback wyjścia z aplikacji
pub fn handle_exit(ui_handle: Weak<AppWindow>) {
    if let Some(ui) = ui_handle.upgrade() {
        let _ = ui.window().hide();
    }
}

// Re-exports from specialized modules
pub use crate::ui::image_controls::{
    ThrottledUpdate, 
    handle_parameter_changed_throttled, 
    update_preview_image
};

pub use crate::ui::thumbnails::{
    load_thumbnails_for_directory
};

pub use crate::ui::file_handlers::{
    handle_open_exr,
    handle_open_exr_from_path
};
