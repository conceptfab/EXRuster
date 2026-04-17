use crate::ui::{push_console, SharedAppState};
use crate::utils::error_handling::UiErrorReporter;
use crate::AppWindow;
use slint::{ComponentHandle, Model, SharedString, VecModel, Weak};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Helper struct to reduce Arc cloning and simplify callback setup
#[derive(Clone)]
struct CallbackHelper {
    ui_weak: Weak<AppWindow>,
    app_state: SharedAppState,
    console_model: Rc<VecModel<SharedString>>,
}

impl CallbackHelper {
    fn new(ui: &AppWindow, app_state: SharedAppState, console_model: Rc<VecModel<SharedString>>) -> Self {
        Self {
            ui_weak: ui.as_weak(),
            app_state,
            console_model,
        }
    }

    /// Execute export operation with error handling
    fn execute_export<F>(&self, export_fn: F)
    where
        F: FnOnce(Weak<AppWindow>, SharedAppState, Rc<VecModel<SharedString>>),
    {
        export_fn(self.ui_weak.clone(), self.app_state.clone(), self.console_model.clone());
    }

    /// Handle tone mapping mode change
    fn handle_tonemap_change(&self, mode: i32) {
        if let Some(ui) = self.ui_weak.upgrade() {
            if let Ok(state) = self.app_state.read() {
                if let Some(ref cache) = state.image_cache {
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let image = crate::ui::update_preview_image(
                        &ui, cache, exposure, gamma, mode, &self.console_model,
                    );
                    ui.set_exr_image(image);
                    push_console(
                        &ui,
                        &self.console_model,
                        format!("[preview] updated → tonemap mode: {}", mode),
                    );
                    ui.set_status_text(
                        format!(
                            "Tonemap: {}",
                            match mode {
                                0 => "ACES",
                                1 => "Reinhard", 
                                2 => "Linear",
                                3 => "Filmic",
                                4 => "Hable",
                                5 => "Local",
                                _ => "Unknown",
                            }
                        ).into()
                    );
                }
            }
        }
    }

}

/// Setup menu-related callbacks (file operations, console management, histogram, layers)
pub fn setup_menu_callbacks(
    ui: &AppWindow,
    app_state: SharedAppState,
    console_model: Rc<VecModel<SharedString>>,
) {
    let helper = CallbackHelper::new(ui, app_state.clone(), console_model.clone());
    
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
        let app_state = Arc::clone(&app_state);
        let console = console_model.clone();
        move || {
            crate::ui::handle_open_exr(ui_handle.clone(), app_state.clone(), console.clone());
        }
    });

    // Callback dla żądania histogramu
    ui.on_histogram_requested({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                crate::ui::file_handlers::apply_histogram_to_ui(&ui, &app_state);
                push_console(&ui, &console, "[histogram] updated".to_string());
                ui.set_status_text("Histogram updated".into());
            }
        }
    });

    {
        ui.on_layer_tree_clicked({
            let ui_handle = ui.as_weak();
            let app_state = Arc::clone(&app_state);
            let console = console_model.clone();
            move |clicked_item: slint::SharedString| {
                crate::ui::handle_layer_tree_click(
                    ui_handle.clone(),
                    app_state.clone(),
                    clicked_item.to_string(),
                    console.clone(),
                );
            }
        });
    }

    // Export callbacks - using helper to reduce Arc cloning
    ui.on_export_beauty({
        let helper = helper.clone();
        move || helper.execute_export(crate::ui::export_handlers::export_beauty)
    });

    ui.on_export_all({
        let helper = helper.clone();
        move || helper.execute_export(crate::ui::export_handlers::export_all)
    });

    ui.on_export_scene({
        let helper = helper.clone();
        move || helper.execute_export(crate::ui::export_handlers::export_scene)
    });

    ui.on_export_objects({
        let helper = helper.clone();
        move || helper.execute_export(crate::ui::export_handlers::export_objects)
    });

    ui.on_export_cryptomatte({
        let helper = helper.clone();
        move || helper.execute_export(crate::ui::export_handlers::export_cryptomatte)
    });

    ui.on_export_lights({
        let helper = helper.clone();
        move || helper.execute_export(crate::ui::export_handlers::export_lights)
    });
}

/// Setup image control callbacks (exposure, gamma, tonemap mode)
pub fn setup_image_control_callbacks(
    ui: &AppWindow,
    app_state: SharedAppState,
    console_model: Rc<VecModel<SharedString>>,
) {
    let helper = CallbackHelper::new(ui, app_state.clone(), console_model.clone());
    let ui_weak_for_throttle = ui.as_weak();
    let app_state_for_throttle = app_state.clone();
    let console_for_throttle = console_model.clone();

    let throttled_updater = crate::ui::ThrottledUpdate::new(move |exp, gamma| {
        if let Some(_ui) = ui_weak_for_throttle.upgrade() {
            crate::ui::handle_parameter_changed_throttled(
                ui_weak_for_throttle.clone(),
                app_state_for_throttle.clone(),
                console_for_throttle.clone(),
                exp,
                gamma,
            );
        }
    });
    let throttled_update = Arc::new(Mutex::new(throttled_updater));

    ui.on_exposure_changed({
        let throttled_update = Arc::clone(&throttled_update);

        move |exposure: f32| {
            let updater = throttled_update.lock().unwrap();
            updater.update_exposure(exposure);
        }
    });

    ui.on_gamma_changed({
        let throttled_update = Arc::clone(&throttled_update);

        move |gamma: f32| {
            let updater = throttled_update.lock().unwrap();
            updater.update_gamma(gamma);
        }
    });

    // Tonemap mode changed - using extracted helper method
    ui.on_tonemap_mode_changed({
        let helper = helper.clone();
        move |mode: i32| helper.handle_tonemap_change(mode)
    });

    // Re-render podgląd przy zmianie geometrii obszaru podglądu (debounce 200ms)
    let debounced_geometry = {
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console = console_model.clone();
        let debounced = crate::ui::image_controls::DebouncedGeometry::new(move || {
            if let Some(ui) = ui_handle.upgrade() {
                if let Ok(state) = app_state.read() {
                    if let Some(ref cache) = state.image_cache {
                        let exposure = ui.get_exposure_value();
                        let gamma = ui.get_gamma_value();
                        let mode = ui.get_tonemap_mode();
                        let image =
                            crate::ui::update_preview_image(&ui, cache, exposure, gamma, mode, &console);
                        ui.set_exr_image(image);
                    }
                }
            }
        });
        Arc::new(Mutex::new(debounced))
    };
    ui.on_preview_geometry_changed({
        let debounced = Arc::clone(&debounced_geometry);
        move |_w, _h| {
            debounced.lock().unwrap().trigger();
        }
    });
}

/// Setup panel callbacks (working folder, thumbnails, navigation)
pub fn setup_panel_callbacks(
    ui: &AppWindow,
    app_state: SharedAppState,
    console_model: Rc<VecModel<SharedString>>,
) {
    ui.on_key_pressed_debug({
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone();
        move |key: slint::SharedString| {
            if let Some(ui) = ui_handle.upgrade() {
                let k = if key.is_empty() {
                    SharedString::from("<empty>")
                } else {
                    key.clone()
                };
                ui.set_status_text(format!("key: {}", k).into());
                push_console(&ui, &console_model, format!("[key] {}", k));
            }
        }
    });
    ui.on_choose_working_folder({
        let app_state = Arc::clone(&app_state);
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                push_console(
                    &ui,
                    &console_model,
                    "[folder] choosing working folder...".to_string(),
                );

                if let Some(dir) = crate::io::file_operations::open_folder_dialog() {
                    if let Ok(mut state) = app_state.write() {
                        state.current_browsed_folder = Some(dir.clone());
                    }
                    crate::ui::load_thumbnails_for_directory(
                        ui.as_weak(),
                        &dir,
                        console_model.clone(),
                        130,
                    );
                } else {
                    push_console(
                        &ui,
                        &console_model,
                        "[folder] selection canceled".to_string(),
                    );
                }
            }
        }
    });

    ui.on_open_thumbnail({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move |path_str: slint::SharedString| {
            if let Some(_ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                {
                    let line =
                        SharedString::from(format!("[thumbnails] opening file {}", path.display()));
                    console_model.push(line.clone());
                }
                crate::ui::handle_open_exr_from_path(
                    ui_handle.clone(),
                    app_state.clone(),
                    console_model.clone(),
                    path,
                );
            }
        }
    });

    // Nawigacja miniatur klawiszami (delta: -1 wstecz, +1 dalej)
    ui.on_navigate_thumbnails({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move |delta: i32| {
            if delta == 0 {
                return;
            }
            if let Some(ui) = ui_handle.upgrade() {
                let model = ui.get_thumbnails();
                let count = model.row_count();
                if count == 0 {
                    return;
                }

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
                    if delta > 0 {
                        0
                    } else {
                        (count as i32) - 1
                    }
                };

                if let Some(item) = model.row_data(next_idx as usize) {
                    ui.set_opened_thumbnail_path(item.path.clone());
                    let path = std::path::PathBuf::from(item.path.as_str());
                    crate::ui::handle_open_exr_from_path(
                        ui.as_weak(),
                        app_state.clone(),
                        console_model.clone(),
                        path,
                    );
                }
            }
        }
    });

    // Zwijanie/rozwijanie WSZYSTKICH grup klawiszami (delta: -1 zwiń wszystkie, +1 rozwiń wszystkie)
    ui.on_navigate_layers({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move |delta: i32| {
            if delta == 0 {
                return;
            }
            if let Some(_ui) = ui_handle.upgrade() {
                // delta > 0 = strzałka w dół = rozwiń wszystkie grupy ✅
                // delta < 0 = strzałka w górę = zwiń wszystkie grupy ✅
                let expand_all = delta > 0;
                crate::ui::toggle_all_layer_groups(
                    ui_handle.clone(),
                    app_state.clone(),
                    console_model.clone(),
                    expand_all,
                );
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
                            push_console(
                                &ui,
                                &console_model,
                                format!("[delete] removed {}", display),
                            );
                            ui.set_status_text(format!("Deleted: {}", display).into());
                            // Po usunięciu odśwież miniatury dla katalogu pliku
                            if let Some(dir) = path.parent() {
                                crate::ui::load_thumbnails_for_directory(
                                    ui.as_weak(),
                                    dir,
                                    console_model.clone(),
                                    130,
                                );
                            }
                        }
                        Err(e) => {
                            ui.report_error_with_status(
                                &console_model,
                                "delete",
                                "Delete error",
                                format!("{} → {}", display, e),
                            );
                        }
                    }
                }
            }
        }
    });

    ui.on_copy_current_path({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move || {
            crate::ui::browser_handlers::handle_copy_current_path(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
            );
        }
    });

    ui.on_copy_current_file_to_location({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move || {
            crate::ui::browser_handlers::handle_copy_current_file_to(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
            );
        }
    });

    ui.on_thumbnail_size_changed({
        let ui_handle = ui.as_weak();
        let app_state = Arc::clone(&app_state);
        let console_model = console_model.clone();
        move |level: i32| {
            crate::ui::browser_handlers::handle_thumbnail_size_changed(
                ui_handle.clone(),
                app_state.clone(),
                console_model.clone(),
                level,
            );
        }
    });
}

/// Main UI callbacks setup - coordinates all other setup functions
pub fn setup_ui_callbacks(ui: &AppWindow, app_state: SharedAppState) -> Rc<VecModel<SharedString>> {
    let console_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    ui.set_console_text(SharedString::from(""));

    setup_menu_callbacks(ui, app_state.clone(), console_model.clone());
    setup_image_control_callbacks(ui, app_state.clone(), console_model.clone());
    setup_panel_callbacks(ui, app_state.clone(), console_model.clone());

    console_model
}
