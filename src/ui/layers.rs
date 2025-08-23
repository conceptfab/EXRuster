use slint::{Weak, ComponentHandle, VecModel, SharedString};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::collections::HashSet;
use crate::io::image_cache::ImageCache;
use crate::ui::state::{SharedUiState, UiState};
use crate::ui::{push_console, lock_or_recover};
use crate::utils::{normalize_channel_name, UiErrorReporter};
use crate::ui::progress::patterns;
use crate::AppWindow;

pub type ImageCacheType = Arc<Mutex<Option<ImageCache>>>;
pub type CurrentFilePathType = Arc<Mutex<Option<PathBuf>>>;
pub type ConsoleModel = std::rc::Rc<VecModel<SharedString>>;

/// Refreshes the layer model UI with current expand/collapse state (optimized)
fn refresh_layer_model(
    ui_handle: Weak<AppWindow>,
    image_cache: ImageCacheType,
    ui_state: SharedUiState,
) {
    if let Some(ui) = ui_handle.upgrade() {
        // Only refresh if we have layers to show
        let layers_info_vec = {
            let guard = lock_or_recover(&image_cache);
            guard.as_ref().map(|c| c.layers_info.clone()).unwrap_or_default()
        };
        
        if !layers_info_vec.is_empty() {
            // Quick rebuild - this is unavoidable with current architecture
            let (layers_model, layers_colors, layers_font_sizes) = 
                crate::ui::file_handlers::create_layers_model(&layers_info_vec, &ui, &ui_state);
            ui.set_layers_model(layers_model);
            ui.set_layers_colors(layers_colors);
            ui.set_layers_font_sizes(layers_font_sizes);
        }
    }
}

pub fn handle_layer_tree_click(
    ui_handle: Weak<AppWindow>,
    image_cache: ImageCacheType,
    clicked_item: String,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
    ui_state: SharedUiState,
) {
    
    let trimmed = clicked_item.trim();
    
    // GRUPA - sprawdź czy zawiera strzałkę grupy 
    if (trimmed.contains("▼ 📂") || trimmed.contains("▶ 📂")) && !trimmed.contains("📁") {
        if let Some(ui) = ui_handle.upgrade() {
            let group_name = trimmed.trim_start_matches("▼ 📂").trim_start_matches("▶ 📂").trim().to_string();
            
            // Toggle group expansion state
            {
                let mut state_guard: std::sync::MutexGuard<'_, UiState> = lock_or_recover(&ui_state);
                state_guard.toggle_group_expansion(&group_name);
            }
            
            // Refresh the layer model
            refresh_layer_model(ui_handle.clone(), image_cache.clone(), ui_state.clone());
            
            push_console(&ui, &console, format!("[expand] toggled group: {}", group_name));
        }
    } 
    // WARSTWA - kliknięcie w warstwę (📁) - zawsze load composite
    else if trimmed.starts_with("📁") {
        if let Some(ui) = ui_handle.upgrade() {
            let layer_name = trimmed.trim_start_matches("📁 ").trim().to_string();
            
            let real_layer_name = {
                let map = lock_or_recover(&crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER);
                map.get(&layer_name).cloned().unwrap_or_else(|| layer_name.clone())
            };
        
            let mut status_msg = String::new();
            status_msg.push_str(&format!("Loading layer: {}", layer_name));
            push_console(&ui, &console, format!("[layer] clicked: {} (real='{}')", layer_name, real_layer_name));
            
            let file_path = {
                let path_guard = lock_or_recover(&current_file_path);
                path_guard.clone()
            };
            
            if let Some(path) = file_path {
                let mut cache_guard = lock_or_recover(&image_cache);
                if let Some(ref mut cache) = *cache_guard {
                    let _prog = patterns::processing(ui.as_weak(), "Loading layer");
                    match cache.load_layer(&path, &real_layer_name, Some(_prog.inner())) {
                        Ok(()) => {
                            let exposure = ui.get_exposure_value();
                            let gamma = ui.get_gamma_value();
                            let tonemap_mode = ui.get_tonemap_mode() as i32;
                            let image = cache.process_to_composite(exposure, gamma, tonemap_mode, true);
                            ui.set_exr_image(image);
                            push_console(&ui, &console, format!("[layer] {} → mode: RGB (composite)", real_layer_name));
                            push_console(&ui, &console, format!("[preview] updated → mode: RGB (composite), layer: {}", real_layer_name));
                            let channels = cache.layers_info
                                .iter()
                                .find(|l| l.name == real_layer_name)
                                .map(|l| l.channels.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(", "))
                                .unwrap_or_else(|| "?".into());
                            status_msg = format!("Layer: {} | mode: RGB | channels: {}", real_layer_name, channels);
                            ui.set_status_text(status_msg.into());
                            ui.set_selected_layer_item(format!("  📁 {}", layer_name).into());
                        }
                        Err(e) => {
                            ui.report_error_with_status(&console, "layer", &format!("Error loading layer {}", real_layer_name), e);
                        }
                    }
                }
            } else {
                ui.report_error(&console, "file", "No file loaded");
            }
        }
    } 
    // KANAŁY - pozostała logika bez zmian
    else {
        let is_dot = trimmed.starts_with("• ");
        let is_rgba_emoji = trimmed.starts_with("🔴") || trimmed.starts_with("🟢") || trimmed.starts_with("🔵") || trimmed.starts_with("⚪");
        if !(is_dot || is_rgba_emoji) { return; }

        if let Some(ui) = ui_handle.upgrade() {
            let file_path = {
                let path_guard = lock_or_recover(&current_file_path);
                path_guard.clone()
            };
            if file_path.is_none() { return; }

            let mut cache_guard = lock_or_recover(&image_cache);
            if let Some(ref mut cache) = *cache_guard {
                let (active_layer, channel_short) = {
                    let s = trimmed;
                    if let Some(at_pos) = s.rfind('@') {
                        let layer_display = s[at_pos + 1..].trim().to_string();
                        let layer = {
                            let map = lock_or_recover(&crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER);
                            map.get(&layer_display).cloned().unwrap_or(layer_display)
                        };
                        let left = s[..at_pos].trim();
                        let ch_short = if is_dot {
                            left.trim_start_matches('•').trim().to_string()
                        } else {
                            left.split_whitespace().nth(1).unwrap_or("").to_string()
                        };
                        (layer, ch_short)
                    } else {
                        let active_layer = {
                            let key = clicked_item.trim_end().to_string();
                            let map = lock_or_recover(&crate::ui::file_handlers::ITEM_TO_LAYER);
                            map.get(&key).cloned().unwrap_or_else(|| cache.current_layer_name.clone())
                        };
                        let ch_short = if is_dot {
                            trimmed.trim_start_matches("• ").trim().to_string()
                        } else {
                            trimmed.split_whitespace().nth(1).unwrap_or("").to_string()
                        };
                        (active_layer, ch_short)
                    }
                };
                let channel_short = normalize_channel_name(&channel_short);

                let path = match file_path {
                    Some(p) => p,
                    None => {
                        ui.report_error(&console, "file", "brak ścieżki do pliku");
                        return;
                    }
                };

                let _prog = patterns::processing(ui.as_weak(), "Loading channel");
                match cache.load_channel(&path, &active_layer, &channel_short, Some(_prog.inner())) {
                    Ok(()) => {
                        let _exposure = ui.get_exposure_value();
                        let _gamma = ui.get_gamma_value();

                        let upper = channel_short.to_ascii_uppercase();
                        if upper == "Z" || upper.contains("DEPTH") {
                            let image = cache.process_depth_image_with_progress(true, Some(_prog.inner()));
                            ui.set_exr_image(image);
                            ui.set_status_text(format!("Layer: {} | Channel: {} | mode: Depth (auto-normalized, inverted)", active_layer, channel_short).into());
                            push_console(&ui, &console, format!("[channel] {}@{} → mode: Depth (auto-normalized, inverted)", channel_short, active_layer));
                            push_console(&ui, &console, format!("[preview] updated → mode: Depth (auto-normalized, inverted), {}::{}", active_layer, channel_short));
                        } else {
                            let image = cache.process_depth_image_with_progress(false, Some(_prog.inner()));
                            ui.set_exr_image(image);
                            ui.set_status_text(format!("Layer: {} | Channel: {} | mode: Grayscale (auto-normalized)", active_layer, channel_short).into());
                            push_console(&ui, &console, format!("[channel] {}@{} → mode: Grayscale (auto-normalized)", channel_short, active_layer));
                            push_console(&ui, &console, format!("[preview] updated → mode: Grayscale (auto-normalized), {}::{}", active_layer, channel_short));
                        }
                        push_console(&ui, &console, format!("[selection] trying to select: '{}'", &clicked_item));
                        ui.set_selected_layer_item(clicked_item.into());
                    }
                    Err(e) => {
                        ui.report_error_with_status(&console, "channel", &format!("Error loading channel {}", channel_short), format!("{}@{}: {}", channel_short, active_layer, e));
                    }
                }
            }
        }
    }
}

/// FUNKCJA: Zwijanie/rozwijanie WSZYSTKICH grup na raz za pomocą strzałek góra/dół
pub fn toggle_all_layer_groups(
    ui_handle: Weak<AppWindow>,
    image_cache: ImageCacheType,
    console: ConsoleModel,
    ui_state: SharedUiState,
    expand: bool, // true = rozwiń wszystkie, false = zwiń wszystkie
) {
    if let Some(ui) = ui_handle.upgrade() {
        // Pobierz nazwy grup z image_cache (nie z ui_state, bo może być puste na starcie)
        let group_names = {
            let guard = lock_or_recover(&image_cache);
            if let Some(cache) = guard.as_ref() {
                // Pobierz wszystkie warstwy i określ ich grupy
                let mut groups = HashSet::new();
                for layer in &cache.layers_info {
                    let name_for_classification = if layer.name.is_empty() { "Beauty" } else { &layer.name };
                    use crate::processing::channel_classification::determine_channel_group_with_config;
                    use crate::utils::channel_config::{load_channel_config, get_fallback_config};
                    
                    let config = load_channel_config().unwrap_or_else(|_| get_fallback_config());
                    let group_name = determine_channel_group_with_config(name_for_classification, &config);
                    groups.insert(group_name);
                }
                groups.into_iter().collect::<Vec<String>>()
            } else {
                vec![]
            }
        };
        
        if group_names.is_empty() {
            push_console(&ui, &console, "[toggle] no groups found in image cache".to_string());
            return;
        }
        
        let action = if expand { "expanded" } else { "collapsed" };
        
        // Ustaw stan wszystkich grup na raz
        {
            let mut state_guard = lock_or_recover(&ui_state);
            for group_name in &group_names {
                state_guard.set_group_expansion(group_name, expand);
            }
        } // Zwolnij lock
        
        // Odśwież model warstw
        refresh_layer_model(ui_handle.clone(), image_cache.clone(), ui_state.clone());
        
        push_console(&ui, &console, format!("[toggle] {} ALL groups: {} (arrow navigation)", action, group_names.join(", ")));
        ui.set_status_text(format!("{} all groups", if expand { "Expanded" } else { "Collapsed" }).into());
    }
}
