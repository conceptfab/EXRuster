use slint::{Weak, ComponentHandle, VecModel, SharedString};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use crate::io::image_cache::ImageCache;
use crate::ui::state::SharedUiState;
use crate::ui::{push_console, lock_or_recover};
use crate::utils::{get_channel_info, normalize_channel_name, UiErrorReporter, patterns};
use crate::AppWindow;

pub type ImageCacheType = Arc<Mutex<Option<ImageCache>>>;
pub type CurrentFilePathType = Arc<Mutex<Option<PathBuf>>>;
pub type ConsoleModel = std::rc::Rc<VecModel<SharedString>>;

pub fn handle_layer_tree_click(
    ui_handle: Weak<AppWindow>,
    image_cache: ImageCacheType,
    clicked_item: String,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
    ui_state: SharedUiState,
) {
    if clicked_item.starts_with("📁") {
        if let Some(ui) = ui_handle.upgrade() {
            let display_layer_name = clicked_item.trim_start_matches("📁").trim().to_string();
            let layer_name = {
                let state_guard = lock_or_recover(&ui_state);
                state_guard.get_real_layer_for_display(&display_layer_name)
                    .cloned()
                    .unwrap_or_else(|| display_layer_name.clone())
            };
            
            let mut status_msg = String::new();
            status_msg.push_str(&format!("Loading layer: {}", display_layer_name));
            push_console(&ui, &console, format!("[layer] clicked: {} (real='{}')", display_layer_name, layer_name));
            
            let file_path = {
                let path_guard = lock_or_recover(&current_file_path);
                path_guard.clone()
            };
            
            if let Some(path) = file_path {
                let mut cache_guard = lock_or_recover(&image_cache);
                if let Some(ref mut cache) = *cache_guard {
                    let _prog = patterns::processing(ui.as_weak(), "Loading layer");
                    match cache.load_layer(&path, &layer_name, Some(_prog.inner())) {
                        Ok(()) => {
                            let exposure = ui.get_exposure_value();
                            let gamma = ui.get_gamma_value();
                            let tonemap_mode = ui.get_tonemap_mode() as i32;
                            let image = cache.process_to_composite(exposure, gamma, tonemap_mode, true);
                            ui.set_exr_image(image);
                            push_console(&ui, &console, format!("[layer] {} → mode: RGB (composite)", layer_name));
                            push_console(&ui, &console, format!("[preview] updated → mode: RGB (composite), layer: {}", layer_name));
                            let channels = cache.layers_info
                                .iter()
                                .find(|l| l.name == layer_name)
                                .map(|l| l.channels.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(", "))
                                .unwrap_or_else(|| "?".into());
                            status_msg = format!("Layer: {} | mode: RGB | channels: {}", layer_name, channels);
                            ui.set_status_text(status_msg.into());
                            ui.set_selected_layer_item(format!("📁 {}", display_layer_name).into());
                        }
                        Err(e) => {
                            ui.report_error_with_status(&console, "layer", &format!("Error loading layer {}", layer_name), e);
                            // Progress automatically resets on scope exit
                        }
                    }
                }
            } else {
                ui.report_error(&console, "file", "No file loaded");
            }
        }
    }
    else {
        let trimmed = clicked_item.trim();
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
                            let state_guard = lock_or_recover(&ui_state);
                            state_guard.get_real_layer_for_display(&layer_display)
                                .cloned()
                                .unwrap_or(layer_display)
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
                            let state_guard = lock_or_recover(&ui_state);
                            state_guard.get_layer_for_item(&key)
                                .cloned()
                                .unwrap_or_else(|| cache.current_layer_name.clone())
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
                        let display_layer = {
                            let state_guard = lock_or_recover(&ui_state);
                            state_guard.get_display_for_real_layer(&active_layer)
                                .unwrap_or(active_layer.clone())
                        };
                        let (_, emoji, display_name) = get_channel_info(&channel_short, &ui);
                        let label = if emoji == "•" { "    • ".to_string() } else { format!("    {} {}", emoji, display_name) };
                        let selected = if label == "    • " {
                            format!("{} @{}", channel_short, display_layer)
                        } else {
                            format!("{} @{}", label, display_layer)
                        };
                        ui.set_selected_layer_item(selected.into());
                    }
                    Err(e) => {
                        ui.report_error_with_status(&console, "channel", &format!("Error loading channel {}", channel_short), format!("{}@{}: {}", channel_short, active_layer, e));
                        // Progress automatically resets on scope exit
                    }
                }
            }
        }
    }
}

