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

use std::sync::{Arc, Mutex};
use crate::ui_handlers::push_console;
use ui_handlers::{ImageCacheType, CurrentFilePathType};
use slint::{ModelRc, VecModel, SharedString, Model};
use std::rc::Rc;
use crate::utils::human_size;
use crate::progress::ProgressSink;

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
    let console_model = setup_ui_callbacks(&ui, image_cache.clone(), current_file_path.clone());

    // Obsługa argumentów uruchomieniowych: otwórz wskazany plik EXR i ewentualnie wczytaj miniatury folderu
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
                            let exposure = ui.get_exposure_value();
                            let gamma = ui.get_gamma_value();
                            let prog = crate::progress::UiProgress::new(ui.as_weak());
                            match crate::thumbnails::generate_exr_thumbnails_in_dir(dir, 150, exposure, gamma, Some(&prog)) {
                                Ok(mut thumbs) => {
                                    use crate::thumbnails::ExrThumbnailInfo; // ensure type in scope
                                    thumbs.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
                                    let items: Vec<ThumbItem> = thumbs.into_iter().map(|t: ExrThumbnailInfo| ThumbItem {
                                        img: t.image,
                                        name: t.file_name.into(),
                                        size: human_size(t.file_size_bytes).into(),
                                        layers: format!("{} layers", t.num_layers).into(),
                                        path: t.path.display().to_string().into(),
                                        width: t.width as i32,
                                        height: t.height as i32,
                                    }).collect();
                                    ui.set_thumbnails(ModelRc::new(VecModel::from(items)));
                                    ui.set_bottom_panel_visible(true);
                                }
                                Err(e) => {
                                    ui.set_status_text(format!("Error loading thumbnails: {}", e).into());
                                }
                            }
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
        move || {
            ui_handlers::handle_open_exr(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });
}

fn setup_image_control_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
    console_model: Rc<VecModel<SharedString>>,
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
) {
    // Debug klawiszy: wypisz do statusu i konsoli
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
                    ui.set_status_text(format!("Loading thumbnails: {}", dir.display()).into());
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let t0 = std::time::Instant::now();
                     let prog = crate::progress::UiProgress::new(ui.as_weak());
                     match crate::thumbnails::generate_exr_thumbnails_in_dir(&dir, 150, exposure, gamma, Some(&prog)) {
                        Ok(mut thumbs) => {
                            prog.set(0.95, Some("Sorting thumbnails..."));
                            thumbs.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
                            let items: Vec<ThumbItem> = thumbs.into_iter().map(|t| ThumbItem {
                                img: t.image,
                                name: t.file_name.into(),
                                size: human_size(t.file_size_bytes).into(),
                                layers: format!("{} layers", t.num_layers).into(),
                                path: t.path.display().to_string().into(),
                                width: t.width as i32,
                                height: t.height as i32,
                            }).collect();
                            let count = items.len();
                            ui.set_thumbnails(ModelRc::new(VecModel::from(items)));
                            let ms = t0.elapsed().as_millis();
                            ui.set_status_text("Thumbnails loaded".into());
                            prog.finish(Some("Thumbnails loaded"));
                            ui.set_bottom_panel_visible(true);
                            push_console(&ui, &console_model, format!("[folder] {} EXR files | thumbnails in {} ms", count, ms));
                        }
                        Err(e) => {
                            ui.set_status_text(format!("Error loading thumbnails: {}", e).into());
                            push_console(&ui, &console_model, format!("[error][folder] {}", e));
                            prog.reset();
                        }
                    }
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
        move |path_str: slint::SharedString| {
            if let Some(_ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                {
                    let line = SharedString::from(format!("[thumbnails] opening file {}", path.display()));
                    console_model.push(line.clone());
                }
                ui_handlers::handle_open_exr_from_path(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console_model.clone(), path);
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
                        path,
                    );
                }
            }
        }
    });
}

fn setup_ui_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
) -> Rc<VecModel<SharedString>> {
    let console_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    ui.set_console_text(SharedString::from(""));

    setup_menu_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone());
    setup_image_control_callbacks(ui, image_cache.clone(), current_file_path.clone(), console_model.clone());
    setup_panel_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone());

    console_model
}