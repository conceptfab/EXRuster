use slint::{Weak, ComponentHandle, VecModel, SharedString};
use std::sync::{Arc, Mutex, MutexGuard};
use std::path::PathBuf;
use std::time::Instant;
use std::collections::HashMap;
use std::rc::Rc;
use crate::io::image_cache::ImageCache;
use anyhow::Result;
// Usunięte nieużywane importy związane z exportem
// Import komponentów Slint
use crate::AppWindow;

// Re-exports from image_controls module
pub use crate::ui::image_controls::{
    ThrottledUpdate, 
    handle_parameter_changed_throttled, 
    update_preview_image
};

// Re-exports from thumbnails module  
pub use crate::ui::thumbnails::{
    load_thumbnails_for_directory
};

// Re-exports from file_handlers module
pub use crate::ui::file_handlers::{
    handle_open_exr,
    handle_open_exr_from_path
};

/// Centralny stan aplikacji - zastępuje globalne static variables
/// TODO: Implement full migration from global statics to dependency injection
#[allow(dead_code)]
pub struct AppState {
    pub item_to_layer: HashMap<String, String>,
    pub display_to_real_layer: HashMap<String, String>,
    pub current_file_path: Option<PathBuf>,
    pub last_preview_log: Option<Instant>,
}

#[allow(dead_code)]
impl AppState {
    pub fn new() -> Self {
        Self {
            item_to_layer: HashMap::new(),
            display_to_real_layer: HashMap::new(),
            current_file_path: None,
            last_preview_log: None,
        }
    }
    
    /// Synchronizuje stan z globalnymi zmiennymi (przejściowe rozwiązanie)
    pub fn sync_with_globals(&mut self) {
        // W przyszłości: migracja krok po kroku z globalnych na dependency injection
        // TODO: Implement proper sync with current_file_path global
    }
}

pub type ImageCacheType = Arc<Mutex<Option<ImageCache>>>;
pub type CurrentFilePathType = Arc<Mutex<Option<PathBuf>>>;
pub type ConsoleModel = Rc<VecModel<SharedString>>;
use crate::io::full_exr_cache::FullExrCacheData;
pub type FullExrCache = Arc<Mutex<Option<std::sync::Arc<FullExrCacheData>>>>;


/// Dodaje linię do modelu konsoli i aktualizuje tekst w `TextEdit` (console-text)
pub fn push_console(ui: &crate::AppWindow, console: &ConsoleModel, line: String) {
    console.push(line.clone().into());
    let mut joined = ui.get_console_text().to_string();
    if !joined.is_empty() { joined.push('\n'); }
    joined.push_str(&line);
    ui.set_console_text(joined.into());
}




/// Standardowy wzorzec dla bezpiecznego dostępu do Mutex z kontekstem błędu
/// TODO: Replace lock_or_recover usage with this function for better error handling
#[allow(dead_code)]
#[inline]
pub(crate) fn safe_lock<'a, T>(mutex: &'a Arc<Mutex<T>>, context: &'static str) -> Result<MutexGuard<'a, T>> {
    mutex.lock()
        .map_err(|_| anyhow::anyhow!("Mutex poisoned: {}", context))
}

/// Kompatybilność wsteczna - używa panic recovery
#[inline]
pub fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// Uproszczone: usunięty stan drzewa i globalny TREE_STATE



// Removed old handle_layer_tree_click - now in layers.rs


/// Obsługuje callback wyjścia z aplikacji
pub fn handle_exit(ui_handle: Weak<AppWindow>) {
    if let Some(ui) = ui_handle.upgrade() {
        let _ = ui.window().hide();
    }
}
















