#![feature(portable_simd)]
#![windows_subsystem = "windows"]

slint::include_modules!();

mod image_cache;
mod image_processing;
mod file_operations;
mod ui_handlers;
mod thumbnails;
mod exr_metadata;
mod progress;
mod utils;
mod color_processing;
mod full_exr_cache;
mod gpu_context;

#[cfg(target_os = "windows")]
mod platform_win;

use std::sync::{Arc, Mutex};
use crate::ui_handlers::push_console;
use ui_handlers::{ImageCacheType, CurrentFilePathType, FullExrCache, GpuContextType};
use slint::{VecModel, SharedString, Model};
use std::rc::Rc;
use crate::gpu_context::GpuContext;
use pollster;

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
            let done = platform_win::try_set_runtime_window_icon();
            let n = retries_c.get();
            if done || n >= 40 {
                if let Some(t) = timer_weak.upgrade() { t.stop(); }
            } else {
                retries_c.set(n + 1);
            }
        });
    }
    

    let gpu_context: GpuContextType = Arc::new(Mutex::new(None));
    

    {
        let gpu_context_clone = gpu_context.clone();
        let ui_weak = ui.as_weak();
        

        std::thread::spawn(move || {

            let gpu_result = pollster::block_on(async {

                GpuContext::new().await
            });
            
            match gpu_result {
                Ok(context) => {
                    let adapter_info = context.get_adapter_info();
                    println!("GPU: {} - inicjalizacja pomyślna", adapter_info.name);
                    
                    // Zaktualizuj kontekst GPU
                    if let Ok(mut guard) = gpu_context_clone.lock() {
                        *guard = Some(context);
                    }
                    
                    // Ustaw globalny kontekst GPU w ui_handlers
                    ui_handlers::set_global_gpu_context(gpu_context_clone.clone());
                    
                    // Zaktualizuj UI z informacją o GPU
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_gpu_status_text(format!("GPU: {} - dostępny", adapter_info.name).into());
                    }
                }
                Err(e) => {
                    println!("GPU: inicjalizacja nieudana - {}", e);
                    println!("Aplikacja będzie działać w trybie CPU");
                    
                    // Zaktualizuj UI z informacją o braku GPU
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_gpu_status_text("GPU: niedostępny (tryb CPU)".into());
                    }
                }
            }
        });
    }
    
    let image_cache: ImageCacheType = Arc::new(Mutex::new(None));
    let current_file_path: CurrentFilePathType = Arc::new(Mutex::new(None));
    let full_exr_cache: FullExrCache = Arc::new(Mutex::new(None));

    // Setup UI callbacks...
    let console_model = setup_ui_callbacks(&ui, image_cache.clone(), current_file_path.clone(), full_exr_cache.clone(), gpu_context.clone());


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
                ui_handlers::handle_open_exr_from_path(
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
    
                            ui_handlers::load_thumbnails_for_directory(ui.as_weak(), dir, console_model.clone());
                        }
                    }
                }
            }
        }
    }
    
    ui.run()
}



fn setup_menu_callbacks(
    ui: &AppWindow,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console_model: Rc<VecModel<SharedString>>,
    full_exr_cache: FullExrCache,
    _gpu_context: GpuContextType,
) {
    ui.on_clear_console({
        let ui_handle = ui.as_weak();
        let console_for_clear = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                console_for_clear.set_vec(vec![]);
                ui.set_console_text(SharedString::from(""));
                ui.set_status_text(SharedString::from("Console cleared"));
            }
        }
    });

    ui.on_exit({
        let ui_handle = ui.as_weak();
        move || {
            ui_handlers::handle_exit(ui_handle.clone());
        }
    });

    ui.on_open_exr({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone(); // Use console_model directly
        let full_exr_cache = full_exr_cache.clone();
        move || {
            ui_handlers::handle_open_exr(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone(), full_exr_cache.clone());
        }
    });

    // Export: Convert (EXR -> TIFF)
    ui.on_export_convert({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        let full_exr_cache = full_exr_cache.clone();
        move || {
            ui_handlers::handle_export_convert(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone(), full_exr_cache.clone());
        }
    });

    // Export: Beauty (PNG16)
    ui.on_export_beauty({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move || {
            ui_handlers::handle_export_beauty(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });

    // Export: Channels (PNG16 grayscale)
    ui.on_export_channels({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        let full_exr_cache = full_exr_cache.clone();
        move || {
            ui_handlers::handle_export_channels(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone(), full_exr_cache.clone());
        }
    });
}

fn setup_image_control_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
    console_model: Rc<VecModel<SharedString>>,
    _gpu_context: GpuContextType,
) {
    let ui_weak_for_throttle = ui.as_weak();
    let cache_weak_for_throttle = image_cache.clone();
    let console_for_throttle = console_model.clone(); // Use console_model directly

    let throttled_updater = ui_handlers::ThrottledUpdate::new(move |exp, gamma| {
        if let Some(_ui) = ui_weak_for_throttle.upgrade() {
            ui_handlers::handle_parameter_changed_throttled(
                ui_weak_for_throttle.clone(), 
                cache_weak_for_throttle.clone(), 
                console_for_throttle.clone(),
                exp, 
                gamma
            );
        }
    });
    let throttled_update = Arc::new(Mutex::new(throttled_updater));

    ui.on_exposure_changed({
        let throttled_update = throttled_update.clone();
        
        move |exposure: f32| {
            let updater = throttled_update.lock().unwrap();
                updater.update_exposure(exposure);
        }
    });

    ui.on_gamma_changed({
        let throttled_update = throttled_update.clone();
        
        move |gamma: f32| {
            let updater = throttled_update.lock().unwrap();
            updater.update_gamma(gamma);
        }
    });

    // Tonemap mode changed
    ui.on_tonemap_mode_changed({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move |mode: i32| {
            if let Some(ui) = ui_handle.upgrade() {
                let cache_guard = ui_handlers::lock_or_recover(&image_cache);
                if let Some(ref cache) = *cache_guard {
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let image = ui_handlers::update_preview_image(&ui, cache, exposure, gamma, mode, &console);
                    ui.set_exr_image(image);
                    push_console(&ui, &console, format!("[preview] updated → tonemap mode: {}", mode));
                    ui.set_status_text(format!("Tonemap: {}", match mode {0=>"ACES",1=>"Reinhard",2=>"Linear", _=>"?"}).into());
                }
            }
        }
    });

    // Re-render podgląd przy zmianie geometrii obszaru podglądu (1:1 względem widżetu, z DPI)
    ui.on_preview_geometry_changed({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move |_w, _h| {
            if let Some(ui) = ui_handle.upgrade() {
                let cache_guard = ui_handlers::lock_or_recover(&image_cache);
                if let Some(ref cache) = *cache_guard {
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let mode = ui.get_tonemap_mode() as i32;
                    let image = ui_handlers::update_preview_image(&ui, cache, exposure, gamma, mode, &console);
                    ui.set_exr_image(image);
                    
                    // Dodatkowe logowanie dla zmiany geometrii
                    let preview_w = ui.get_preview_area_width() as f32;
                    let preview_h = ui.get_preview_area_height() as f32;
                    let dpr = ui.window().scale_factor() as f32;
                    let img_w = cache.width as f32;
                    let img_h = cache.height as f32;
                    let container_ratio = if preview_h > 0.0 { preview_w / preview_h } else { 1.0 };
                    let image_ratio = if img_h > 0.0 { img_w / img_h } else { 1.0 };
                    let display_w_logical = if container_ratio > image_ratio { preview_h * image_ratio } else { preview_w };
                    let display_h_logical = if container_ratio > image_ratio { preview_h } else { preview_w / image_ratio };
                    let win_w = ui.get_window_width() as u32;
                    let win_h = ui.get_window_height() as u32;
                    let win_w_px = (win_w as f32 * dpr).round() as u32;
                    let win_h_px = (win_h as f32 * dpr).round() as u32;
                    ui_handlers::push_console(&ui, &console, format!(
                        "[preview] resized → window={}x{} (≈{}x{} px @{}x) | view={}x{} @{}x | img={}x{} | display≈{}x{} px",
                        win_w, win_h, win_w_px, win_h_px, dpr,
                        preview_w as u32, preview_h as u32, dpr,
                        img_w as u32, img_h as u32,
                        (display_w_logical * dpr).round() as u32,
                        (display_h_logical * dpr).round() as u32
                    ));
                }
            }
        }
    });

    ui.on_layer_tree_clicked({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let current_file_path = current_file_path.clone();
        let console = console_model.clone(); // Use console_model directly
        move |clicked_item: slint::SharedString| {
            ui_handlers::handle_layer_tree_click(
                ui_handle.clone(),
                image_cache.clone(), 
                clicked_item.to_string(),
                current_file_path.clone(),
                console.clone()
            );
        }
    });
}

fn setup_panel_callbacks(
    ui: &AppWindow,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console_model: Rc<VecModel<SharedString>>,
    full_exr_cache: FullExrCache,
    _gpu_context: GpuContextType,
) {

    ui.on_key_pressed_debug({
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone();
        move |key: slint::SharedString| {
            if let Some(ui) = ui_handle.upgrade() {
                let k = if key.is_empty() { SharedString::from("<empty>") } else { key.clone() };
                ui.set_status_text(format!("key: {}", k).into());
                push_console(&ui, &console_model, format!("[key] {}", k));
            }
        }
    });
    ui.on_choose_working_folder({
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone(); // Use console_model directly
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                push_console(&ui, &console_model, "[folder] choosing working folder...".to_string());

                if let Some(dir) = crate::file_operations::open_folder_dialog() {
                    ui_handlers::load_thumbnails_for_directory(ui.as_weak(), &dir, console_model.clone());
                } else {
                    push_console(&ui, &console_model, "[folder] selection canceled".to_string());
                }
            }
        }
    });

    ui.on_open_thumbnail({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console_model = console_model.clone(); // Use console_model directly
        let full_exr_cache = full_exr_cache.clone();
        move |path_str: slint::SharedString| {
            if let Some(_ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                {
                    let line = SharedString::from(format!("[thumbnails] opening file {}", path.display()));
                    console_model.push(line.clone());
                }
                ui_handlers::handle_open_exr_from_path(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console_model.clone(), full_exr_cache.clone(), path);
            }
        }
    });

    // Nawigacja miniatur klawiszami (delta: -1 wstecz, +1 dalej)
    ui.on_navigate_thumbnails({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console_model = console_model.clone();
        move |delta: i32| {
            if delta == 0 { return; }
            if let Some(ui) = ui_handle.upgrade() {
                let model = ui.get_thumbnails();
                let count = model.row_count();
                if count == 0 { return; }

                let current_path = ui.get_opened_thumbnail_path().to_string();
                let mut idx: i32 = -1;
                if !current_path.is_empty() {
                    for i in 0..count {
                        if let Some(item) = model.row_data(i) {
                            if item.path.as_str() == current_path {
                                idx = i as i32;
                                break;
                            }
                        }
                    }
                }

                let next_idx: i32 = if idx >= 0 {
                    (idx + delta).rem_euclid(count as i32)
                } else {
                    if delta > 0 { 0 } else { (count as i32) - 1 }
                };

                if let Some(item) = model.row_data(next_idx as usize) {
                    ui.set_opened_thumbnail_path(item.path.clone());
                    let path = std::path::PathBuf::from(item.path.as_str());
                    ui_handlers::handle_open_exr_from_path(
                        ui.as_weak(),
                        current_file_path.clone(),
                        image_cache.clone(),
                        console_model.clone(),
                        full_exr_cache.clone(),
                        path,
                    );
                }
            }
        }
    });

    // Usunięcie pliku z poziomu miniatur (menu kontekstowe)
    ui.on_delete_thumbnail({
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone();
        move |path_str: slint::SharedString| {
            if let Some(ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                if path.is_file() {
                    let display = path.display().to_string();
                    match trash::delete(&path) {
                        Ok(_) => {
                            push_console(&ui, &console_model, format!("[delete] removed {}", display));
                            ui.set_status_text(format!("Deleted: {}", display).into());
                            // Po usunięciu odśwież miniatury dla katalogu pliku
                            if let Some(dir) = path.parent() {
                                crate::ui_handlers::load_thumbnails_for_directory(ui.as_weak(), dir, console_model.clone());
                            }
                        }
                        Err(e) => {
                            ui.set_status_text(format!("Delete error: {}", e).into());
                            push_console(&ui, &console_model, format!("[error][delete] {} → {}", display, e));
                        }
                    }
                }
            }
        }
    });
}

fn setup_ui_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
    full_exr_cache: FullExrCache,
    gpu_context: GpuContextType,
) -> Rc<VecModel<SharedString>> {
    let console_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    ui.set_console_text(SharedString::from(""));

    setup_menu_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone(), full_exr_cache.clone(), gpu_context.clone());
    setup_image_control_callbacks(ui, image_cache.clone(), current_file_path.clone(), console_model.clone(), gpu_context.clone());
    setup_panel_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone(), full_exr_cache.clone(), gpu_context.clone());
    
    // Setup GPU status callback
    setup_gpu_status_callback(ui, gpu_context.clone());

    console_model
}

fn setup_gpu_status_callback(
    ui: &AppWindow,
    gpu_context: GpuContextType,
) {
    ui.on_gpu_status_changed({
        let ui_handle = ui.as_weak();
        let gpu_context = gpu_context.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui_handlers::update_gpu_status(&ui, &gpu_context);
            }
        }
    });
    
    ui.on_check_gpu_availability({
        let ui_handle = ui.as_weak();
        let gpu_context = gpu_context.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                ui_handlers::check_gpu_availability(&ui, &gpu_context);
            }
        }
    });
    
    ui.on_toggle_gpu_acceleration({
        let ui_handle = ui.as_weak();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let current_state = ui.get_gpu_acceleration_enabled();
                ui.set_gpu_acceleration_enabled(!current_state);
                
                // Zaktualizuj globalny stan w ui_handlers
                ui_handlers::set_global_gpu_acceleration(!current_state);
                
                // Zaktualizuj status
                let new_status = if !current_state {
                    "GPU: akceleracja włączona"
                } else {
                    "GPU: akceleracja wyłączona"
                };
                ui.set_gpu_status_text(new_status.into());
            }
        }
    });
}