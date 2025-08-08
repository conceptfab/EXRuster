#![windows_subsystem = "windows"]

slint::include_modules!();

mod image_cache;
mod image_processing;
mod file_operations;
mod ui_handlers;
mod thumbnails;
mod exr_metadata;
mod progress;

use std::sync::{Arc, Mutex};
use ui_handlers::{ImageCacheType, CurrentFilePathType};
use slint::{ModelRc, VecModel, SharedString};
use std::rc::Rc;

fn main() -> Result<(), slint::PlatformError> {
    // Ustaw Rayon thread pool na podstawie CPU cores
    rayon::ThreadPoolBuilder::new()
        .num_threads((num_cpus::get() - 1).max(1)) // Zostaw 1 core dla UI
        .build_global()
        .expect("Failed to initialize thread pool");

    let ui = AppWindow::new()?;
    
    let image_cache: ImageCacheType = Arc::new(Mutex::new(None));
    let current_file_path: CurrentFilePathType = Arc::new(Mutex::new(None));

    // Setup UI callbacks...
    setup_ui_callbacks(&ui, image_cache.clone(), current_file_path.clone());
    
    ui.run()
}

// usunięto nieużywany helper lock_or_recover z main.rs (istnieje w ui_handlers.rs)

fn setup_ui_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
) {
    // Model konsoli i podpięcie do UI
    let console_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    ui.set_console_lines(ModelRc::new(console_model.clone()));
    ui.set_console_text(SharedString::from(""));
    // Stałe klony Rc do użycia w różnych callbackach (uniknięcie przeniesień)
    let console_for_open = console_model.clone();
    let console_for_exp_outer = console_model.clone();
    let console_for_gamma_outer = console_model.clone();
    let console_for_tree = console_model.clone();
    let console_for_folder = console_model.clone();
    let console_for_thumb_open = console_model.clone();

    // Implementacja czyszczenia konsoli
    ui.on_clear_console({
        let ui_handle = ui.as_weak();
        let console_for_clear = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                // Wyczyść model linii (widok) i treść tekstową
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
        let console = console_for_open.clone();
        move || {
            ui_handlers::handle_open_exr(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });

    // Throttled exposure/gamma handling
    let throttled_update = Arc::new(Mutex::new(None));
    
    ui.on_exposure_changed({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let throttled_update = throttled_update.clone();
        let console_exp = console_for_exp_outer.clone();
        
        move |exposure: f32| {
            // Pierwsza aktualizacja - inicjalizuj throttled update
            let mut guard = throttled_update.lock().unwrap();
            if guard.is_none() {
                let ui_weak = ui_handle.clone();
                let cache_weak = image_cache.clone();
                let console_for_update = console_exp.clone();
                
                *guard = Some(ui_handlers::ThrottledUpdate::new(move |exp, gamma| {
                    if let Some(_ui) = ui_weak.upgrade() {
                        ui_handlers::handle_parameter_changed_throttled(
                            ui_weak.clone(), 
                            cache_weak.clone(), 
                            console_for_update.clone(),
                            exp, 
                            gamma
                        );
                    }
                }));
            }
            
            if let Some(ref updater) = *guard {
                updater.update_exposure(exposure);
            }
        }
    });

    // DODAJ TEN CALLBACK DLA GAMMA!
    ui.on_gamma_changed({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let throttled_update = throttled_update.clone();
        let console_gamma = console_for_gamma_outer.clone();
        
        move |gamma: f32| {
            let mut guard = throttled_update.lock().unwrap();
            if guard.is_none() {
                let ui_weak = ui_handle.clone();
                let cache_weak = image_cache.clone();
                let console_for_update = console_gamma.clone();
                
                *guard = Some(ui_handlers::ThrottledUpdate::new(move |exp, gamma| {
                    if let Some(_ui) = ui_weak.upgrade() {
                        ui_handlers::handle_parameter_changed_throttled(
                            ui_weak.clone(), 
                            cache_weak.clone(), 
                            console_for_update.clone(),
                            exp, 
                            gamma
                        );
                    }
                }));
            }
            
            if let Some(ref updater) = *guard {
                updater.update_gamma(gamma);
            }
        }
    });

    // DODAJ TEN NOWY CALLBACK:
    ui.on_layer_tree_clicked({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let current_file_path = current_file_path.clone();
        let console = console_for_tree.clone();
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

    // Wybór folderu roboczego → generuj miniaturki w prawej kolumnie
    ui.on_choose_working_folder({
        let ui_handle = ui.as_weak();
        let console_model = console_for_folder.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                // log start wyboru folderu
                {
                    let line = SharedString::from("[folder] choosing working folder...");
                    console_model.push(line.clone());
                    let mut joined = ui.get_console_text().to_string();
                    if !joined.is_empty() { joined.push('\n'); }
                    joined.push_str(&line);
                    ui.set_console_text(joined.into());
                }

                if let Some(dir) = crate::file_operations::open_folder_dialog() {
                    ui.set_status_text(format!("Loading thumbnails: {}", dir.display()).into());
                    // Generuj miniaturki (parametry z UI)
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let t0 = std::time::Instant::now();
                    match crate::thumbnails::generate_exr_thumbnails_in_dir(&dir, 384, exposure, gamma) {
                        Ok(mut thumbs) => {
                            // sortowanie alfabetyczne wg nazwy pliku
                            thumbs.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
                            // mapowanie do modelu Slint
                            let items: Vec<ThumbItem> = thumbs.into_iter().map(|t| ThumbItem {
                                img: t.image,
                                name: t.file_name.into(),
                                size: human_size(t.file_size_bytes).into(),
                                layers: format!("{} layers", t.num_layers).into(),
                                path: t.path.display().to_string().into(),
                            }).collect();
                            let count = items.len();
                            ui.set_thumbnails(ModelRc::new(VecModel::from(items)));
                            let ms = t0.elapsed().as_millis();
                            ui.set_status_text("Thumbnails loaded".into());
                            // log sukces
                            {
                                let msg = format!("[folder] {} EXR files | thumbnails in {} ms", count, ms);
                                let line = SharedString::from(msg);
                                console_model.push(line.clone());
                                let mut joined = ui.get_console_text().to_string();
                                if !joined.is_empty() { joined.push('\n'); }
                                joined.push_str(&line);
                                ui.set_console_text(joined.into());
                            }
                        }
                        Err(e) => {
                            ui.set_status_text(format!("Error loading thumbnails: {}", e).into());
                            let line = SharedString::from(format!("[error][folder] {}", e));
                            console_model.push(line.clone());
                            let mut joined = ui.get_console_text().to_string();
                            if !joined.is_empty() { joined.push('\n'); }
                            joined.push_str(&line);
                            ui.set_console_text(joined.into());
                        }
                    }
                } else {
                    // log cancel
                    let line = SharedString::from("[folder] selection canceled");
                    console_model.push(line.clone());
                    let mut joined = ui.get_console_text().to_string();
                    if !joined.is_empty() { joined.push('\n'); }
                    joined.push_str(&line);
                    ui.set_console_text(joined.into());
                }
            }
        }
    });

    // Klik na miniaturce → otwieramy plik tak jak z menu "Open EXR"
    ui.on_open_thumbnail({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console_model = console_for_thumb_open.clone();
        move |path_str: slint::SharedString| {
            if let Some(_ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                // log klik miniatury
                {
                    let line = SharedString::from(format!("[thumbnails] opening file {}", path.display()));
                    console_model.push(line.clone());
                }
                // użyj tej samej procedury co przy pojedynczym pliku
                ui_handlers::handle_open_exr_from_path(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console_model.clone(), path);
            }
        }
    });
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{} B", bytes) } else { format!("{:.1} {}", v, UNITS[i]) }
}