#![feature(portable_simd)]
#![windows_subsystem = "windows"]

slint::include_modules!();

mod processing;
mod io;
mod ui;
mod utils;

#[cfg(target_os = "windows")]
mod platform;

use std::sync::{Arc, Mutex};
use crate::ui::{create_shared_state};
use ui::{ImageCacheType, CurrentFilePathType, FullExrCache, SharedUiState};
// BufferPool is now available via crate::utils::BufferPool re-export

fn main() -> Result<(), slint::PlatformError> {
    // Ustaw obsługę panic aby aplikacja nie znikała
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("PANIC: {}", panic_info);
        eprintln!("Aplikacja przechodzi w tryb awaryjny...");
    }));
    
    // Ustaw Rayon thread pool na podstawie CPU cores
    rayon::ThreadPoolBuilder::new()
        .num_threads((num_cpus::get() - 1).max(1)) // Zostaw 1 core dla UI
        .build_global()
        .expect("Failed to initialize thread pool");

    let ui = AppWindow::new()?;


    #[cfg(target_os = "windows")]
    {
        use slint::TimerMode;
        use std::cell::Cell;
        use std::rc::{Rc, Weak};
        // Opóźnij i ponawiaj próbę, aż okno zostanie utworzone
        let timer = Rc::new(slint::Timer::default());
        let timer_weak: Weak<slint::Timer> = Rc::downgrade(&timer);
        let retries = Rc::new(Cell::new(0));
        let retries_c = retries.clone();
        timer.start(TimerMode::Repeated, std::time::Duration::from_millis(150), move || {
            let done = crate::platform::try_set_runtime_window_icon();
            let n = retries_c.get();
            if done || n >= 40 {
                if let Some(t) = timer_weak.upgrade() { t.stop(); }
            } else {
                retries_c.set(n + 1);
            }
        });
    }
    

    println!("Application running in CPU-only mode");
    
    let image_cache: ImageCacheType = Arc::new(Mutex::new(None));
    let current_file_path: CurrentFilePathType = Arc::new(Mutex::new(None));
    let full_exr_cache: FullExrCache = Arc::new(Mutex::new(None));
    let ui_state: SharedUiState = create_shared_state();
    
    // Initialize global buffer pool for performance optimization
    let buffer_pool = Arc::new(crate::utils::BufferPool::new(32)); // Pool of 32 buffers per type
    crate::io::image_cache::set_global_buffer_pool(buffer_pool.clone());

    // Setup UI callbacks...
    let console_model = crate::ui::setup_ui_callbacks(&ui, image_cache.clone(), current_file_path.clone(), full_exr_cache.clone(), ui_state.clone());


    {
        use std::ffi::OsString;
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        if !args.is_empty() {
            if let Some(first_exr) = args.iter().find_map(|a| {
                let p = std::path::PathBuf::from(a);
                if p.is_file() {
                    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case("exr") { return Some(p); }
                    }
                }
                None
            }) {
                crate::ui::handle_open_exr_from_path(
                    ui.as_weak(),
                    current_file_path.clone(),
                    image_cache.clone(),
                    console_model.clone(),
                    full_exr_cache.clone(),
                    first_exr.clone(),
                );

                if let Some(dir) = first_exr.parent() {
                    if let Ok(read) = std::fs::read_dir(dir) {
                        let exr_count = read
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| p.is_file())
                            .filter(|p| p.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("exr")).unwrap_or(false))
                            .count();

                        if exr_count > 1 {
    
                            crate::ui::load_thumbnails_for_directory(ui.as_weak(), dir, console_model.clone());
                        }
                    }
                }
            }
        }
    }
    
    ui.run()
}







