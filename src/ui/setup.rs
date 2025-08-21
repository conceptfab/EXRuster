use slint::{VecModel, SharedString, Model, ComponentHandle};
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use crate::ui::{push_console, lock_or_recover, ImageCacheType, CurrentFilePathType, FullExrCache, SharedUiState};
use crate::utils::error_handling::UiErrorReporter;
use crate::AppWindow;

/// Setup menu-related callbacks (file operations, console management, histogram, layers)
pub fn setup_menu_callbacks(
    ui: &AppWindow,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console_model: Rc<VecModel<SharedString>>,
    full_exr_cache: FullExrCache,
    ui_state: SharedUiState,
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
            crate::ui::handle_exit(ui_handle.clone());
        }
    });

    ui.on_open_exr({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        let full_exr_cache = full_exr_cache.clone();
        move || {
            crate::ui::handle_open_exr(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone(), full_exr_cache.clone());
        }
    });

    // Callback dla żądania histogramu
    ui.on_histogram_requested({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                let mut cache_guard = lock_or_recover(&image_cache);
                if let Some(ref mut cache) = *cache_guard {
                    match cache.update_histogram() {
                        Ok(()) => {
                            if let Some(hist_data) = cache.get_histogram_data() {
                                // Apply histogram data to UI using the new unified method
                                hist_data.apply_to_ui(&ui);
                                
                                // Additional statistics not covered by apply_to_ui
                                ui.set_histogram_total_pixels(hist_data.total_pixels as i32);
                                
                                // Percentyle
                                let p1 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.01);
                                let p50 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.50);
                                let p99 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.99);
                                ui.set_histogram_p1(p1);
                                ui.set_histogram_p50(p50);
                                ui.set_histogram_p99(p99);
                                
                                push_console(&ui, &console, format!("[histogram] computed: min={:.3}, max={:.3}, median={:.3}", p1, p50, p99));
                                ui.set_status_text("Histogram updated".into());
                            }
                        }
                        Err(e) => {
                            ui.report_error(&console, "histogram", e);
                        }
                    }
                }
            }
        }
    });

    {
        let ui_state_for_layer_click = ui_state.clone();
        ui.on_layer_tree_clicked({
            let ui_handle = ui.as_weak();
            let image_cache = image_cache.clone();
            let current_file_path = current_file_path.clone();
            let console = console_model.clone();
            move |clicked_item: slint::SharedString| {
                crate::ui::handle_layer_tree_click(
                    ui_handle.clone(),
                    image_cache.clone(), 
                    clicked_item.to_string(),
                    current_file_path.clone(),
                    console.clone(),
                    ui_state_for_layer_click.clone()
                );
            }
        });
    }
}

/// Setup image control callbacks (exposure, gamma, tonemap mode)
pub fn setup_image_control_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    _current_file_path: CurrentFilePathType,
    console_model: Rc<VecModel<SharedString>>,
) {
    let ui_weak_for_throttle = ui.as_weak();
    let cache_weak_for_throttle = image_cache.clone();
    let console_for_throttle = console_model.clone();

    let throttled_updater = crate::ui::ThrottledUpdate::new(move |exp, gamma| {
        if let Some(_ui) = ui_weak_for_throttle.upgrade() {
            crate::ui::handle_parameter_changed_throttled(
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
                let cache_guard = crate::ui::lock_or_recover(&image_cache);
                if let Some(ref cache) = *cache_guard {
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let image = crate::ui::update_preview_image(&ui, cache, exposure, gamma, mode, &console);
                    ui.set_exr_image(image);
                    push_console(&ui, &console, format!("[preview] updated → tonemap mode: {}", mode));
                    ui.set_status_text(format!("Tonemap: {}", match mode {0=>"ACES",1=>"Reinhard",2=>"Linear",3=>"Filmic",4=>"Hable",5=>"Local", _=>"?"}).into());
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
                let cache_guard = crate::ui::lock_or_recover(&image_cache);
                if let Some(ref cache) = *cache_guard {
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let mode = ui.get_tonemap_mode() as i32;
                    let image = crate::ui::update_preview_image(&ui, cache, exposure, gamma, mode, &console);
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
                    crate::ui::push_console(&ui, &console, format!(
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
}

/// Setup panel callbacks (working folder, thumbnails, navigation)
pub fn setup_panel_callbacks(
    ui: &AppWindow,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console_model: Rc<VecModel<SharedString>>,
    full_exr_cache: FullExrCache,
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
        let console_model = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                push_console(&ui, &console_model, "[folder] choosing working folder...".to_string());

                if let Some(dir) = crate::io::file_operations::open_folder_dialog() {
                    crate::ui::load_thumbnails_for_directory(ui.as_weak(), &dir, console_model.clone());
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
        let console_model = console_model.clone();
        let full_exr_cache = full_exr_cache.clone();
        move |path_str: slint::SharedString| {
            if let Some(_ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                {
                    let line = SharedString::from(format!("[thumbnails] opening file {}", path.display()));
                    console_model.push(line.clone());
                }
                crate::ui::handle_open_exr_from_path(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console_model.clone(), full_exr_cache.clone(), path);
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
                    crate::ui::handle_open_exr_from_path(
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
                                crate::ui::load_thumbnails_for_directory(ui.as_weak(), dir, console_model.clone());
                            }
                        }
                        Err(e) => {
                            ui.report_error_with_status(&console_model, "delete", "Delete error", format!("{} → {}", display, e));
                        }
                    }
                }
            }
        }
    });
}

/// Main UI callbacks setup - coordinates all other setup functions
pub fn setup_ui_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
    full_exr_cache: FullExrCache,
    ui_state: SharedUiState,
) -> Rc<VecModel<SharedString>> {
    let console_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    ui.set_console_text(SharedString::from(""));

    setup_menu_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone(), full_exr_cache.clone(), ui_state.clone());
    setup_image_control_callbacks(ui, image_cache.clone(), current_file_path.clone(), console_model.clone());
    setup_panel_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone(), full_exr_cache.clone());

    console_model
}