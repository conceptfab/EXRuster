use crate::ui::progress::patterns;
use crate::ui::push_console;
use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{lock_or_recover, ConsoleModel};
use crate::utils::{normalize_channel_name, UiErrorReporter};
use crate::AppWindow;
use slint::{ComponentHandle, Weak};
use std::collections::HashSet;
use std::fmt::Write;

/// Refreshes the layer model UI with current expand/collapse state (optimized)
fn refresh_layer_model(ui_handle: Weak<AppWindow>, app_state: SharedAppState) {
    if let Some(ui) = ui_handle.upgrade() {
        // Only refresh if we have layers to show
        let layers_info_vec = {
            if let Ok(state) = app_state.read() {
                state
                    .image_cache
                    .as_ref()
                    .map(|c| c.layers_info.clone())
                    .unwrap_or_default()
            } else {
                vec![]
            }
        };

        if !layers_info_vec.is_empty() {
            // Quick rebuild - this is unavoidable with current architecture
            let (layers_model, layers_colors, layers_font_sizes) =
                crate::ui::file_handlers::create_layers_model(&layers_info_vec, &ui, &app_state);
            ui.set_layers_model(layers_model);
            ui.set_layers_colors(layers_colors);
            ui.set_layers_font_sizes(layers_font_sizes);
        }
    }
}

pub fn handle_layer_tree_click(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    clicked_item: String,
    console: ConsoleModel,
) {
    let trimmed = clicked_item.trim();

    // GRUPA - sprawdź czy zawiera strzałkę grupy
    if (trimmed.contains("▼ 📂") || trimmed.contains("▶ 📂")) && !trimmed.contains("📁") {
        if let Some(ui) = ui_handle.upgrade() {
            let group_name = trimmed
                .trim_start_matches("▼ 📂")
                .trim_start_matches("▶ 📂")
                .trim()
                .to_string();

            // Toggle group expansion state
            if let Ok(mut state) = app_state.write() {
                state.ui_state.toggle_group_expansion(&group_name);
            }

            // Refresh the layer model
            refresh_layer_model(ui_handle.clone(), app_state.clone());

            push_console(
                &ui,
                &console,
                format!("[expand] toggled group: {}", group_name),
            );
        }
    }
    // WARSTWA - kliknięcie w warstwę (📁) - zawsze load composite
    else if trimmed.starts_with("📁") {
        if let Some(ui) = ui_handle.upgrade() {
            let layer_name = trimmed.trim_start_matches("📁 ").trim().to_string();

            let real_layer_name = {
                let map = lock_or_recover(&crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER);
                map.get(&layer_name)
                    .cloned()
                    .unwrap_or_else(|| layer_name.clone())
            };

            let mut status_msg = String::with_capacity(64);
            use std::fmt::Write;
            write!(&mut status_msg, "Loading layer: {}", layer_name).unwrap();

            // Use a separate buffer for console messages to avoid conflicting borrows
            let mut console_buffer = String::with_capacity(128);
            write!(
                &mut console_buffer,
                "[layer] clicked: {} (real='{}')",
                layer_name, real_layer_name
            )
            .unwrap();
            push_console(&ui, &console, console_buffer.clone());

            if let Ok(mut state) = app_state.write() {
                if let Some(ref path) = state.current_file_path.clone() {
                    if let Some(ref mut cache) = state.image_cache {
                        let _prog = patterns::processing(ui.as_weak(), "Loading layer");
                        match cache.load_layer(&path, &real_layer_name, Some(_prog.inner())) {
                            Ok(()) => {
                                let exposure = ui.get_exposure_value();
                                let gamma = ui.get_gamma_value();
                                let tonemap_mode = ui.get_tonemap_mode() as i32;
                                let image =
                                    cache.process_to_composite(exposure, gamma, tonemap_mode, true);
                                ui.set_exr_image(image);
                                console_buffer.clear();
                                write!(
                                    &mut console_buffer,
                                    "[layer] {} → mode: RGB (composite)",
                                    real_layer_name
                                )
                                .unwrap();
                                push_console(&ui, &console, console_buffer.clone());

                                console_buffer.clear();
                                write!(
                                    &mut console_buffer,
                                    "[preview] updated → mode: RGB (composite), layer: {}",
                                    real_layer_name
                                )
                                .unwrap();
                                push_console(&ui, &console, console_buffer.clone());
                                let channels = cache
                                    .layers_info
                                    .iter()
                                    .find(|l| l.name == real_layer_name)
                                    .map(|l| {
                                        l.channels
                                            .iter()
                                            .map(|c| c.name.clone())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_else(|| "?".into());
                                status_msg.clear();
                                write!(
                                    &mut status_msg,
                                    "Layer: {} | mode: RGB | channels: {}",
                                    real_layer_name, channels
                                )
                                .unwrap();
                                ui.set_status_text(status_msg.into());
                                console_buffer.clear();
                                write!(&mut console_buffer, "  📁 {}", layer_name).unwrap();
                                ui.set_selected_layer_item(console_buffer.into());
                            }
                            Err(e) => {
                                status_msg.clear();
                                write!(&mut status_msg, "Error loading layer {}", real_layer_name)
                                    .unwrap();
                                ui.report_error_with_status(&console, "layer", &status_msg, e);
                            }
                        }
                    } else {
                        ui.report_error(&console, "file", "No image cache loaded");
                    }
                } else {
                    ui.report_error(&console, "file", "No file loaded");
                }
            }
        }
    }
    // KANAŁY - pozostała logika bez zmian
    else {
        let is_dot = trimmed.starts_with("• ");
        let is_rgba_emoji = trimmed.starts_with("🔴")
            || trimmed.starts_with("🟢")
            || trimmed.starts_with("🔵")
            || trimmed.starts_with("⚪");
        if !(is_dot || is_rgba_emoji) {
            return;
        }

        if let Some(ui) = ui_handle.upgrade() {
            if let Ok(mut state) = app_state.write() {
                if let Some(ref path) = state.current_file_path.clone() {
                    if let Some(ref mut cache) = state.image_cache {
                        // Pre-allocate string buffers for the channel processing section
                        let mut status_msg = String::with_capacity(128);
                        let mut console_buffer = String::with_capacity(128);

                        let (active_layer, channel_short) = {
                            let s = trimmed;
                            if let Some(at_pos) = s.rfind('@') {
                                let layer_display = s[at_pos + 1..].trim().to_string();
                                let layer = {
                                    let map = lock_or_recover(
                                        &crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER,
                                    );
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
                                    let map =
                                        lock_or_recover(&crate::ui::file_handlers::ITEM_TO_LAYER);
                                    map.get(&key)
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

                        let _prog = patterns::processing(ui.as_weak(), "Loading channel");
                        match cache.load_channel(
                            &path,
                            &active_layer,
                            &channel_short,
                            Some(_prog.inner()),
                        ) {
                            Ok(()) => {
                                let _exposure = ui.get_exposure_value();
                                let _gamma = ui.get_gamma_value();

                                let upper = channel_short.to_ascii_uppercase();
                                if upper == "Z" || upper.contains("DEPTH") {
                                    let image = cache.process_depth_image_with_progress(
                                        true,
                                        Some(_prog.inner()),
                                    );
                                    ui.set_exr_image(image);

                                    status_msg.clear();
                                    write!(&mut status_msg, "Layer: {} | Channel: {} | mode: Depth (auto-normalized, inverted)", active_layer, channel_short).unwrap();
                                    ui.set_status_text(status_msg.clone().into());

                                    console_buffer.clear();
                                    write!(
                                        &mut console_buffer,
                                        "[channel] {}@{} → mode: Depth (auto-normalized, inverted)",
                                        channel_short, active_layer
                                    )
                                    .unwrap();
                                    push_console(&ui, &console, console_buffer.clone());

                                    console_buffer.clear();
                                    write!(&mut console_buffer, "[preview] updated → mode: Depth (auto-normalized, inverted), {}::{}", active_layer, channel_short).unwrap();
                                    push_console(&ui, &console, console_buffer.clone());
                                } else {
                                    let image = cache.process_depth_image_with_progress(
                                        false,
                                        Some(_prog.inner()),
                                    );
                                    ui.set_exr_image(image);

                                    status_msg.clear();
                                    write!(&mut status_msg, "Layer: {} | Channel: {} | mode: Grayscale (auto-normalized)", active_layer, channel_short).unwrap();
                                    ui.set_status_text(status_msg.clone().into());

                                    console_buffer.clear();
                                    write!(
                                        &mut console_buffer,
                                        "[channel] {}@{} → mode: Grayscale (auto-normalized)",
                                        channel_short, active_layer
                                    )
                                    .unwrap();
                                    push_console(&ui, &console, console_buffer.clone());

                                    console_buffer.clear();
                                    write!(&mut console_buffer, "[preview] updated → mode: Grayscale (auto-normalized), {}::{}", active_layer, channel_short).unwrap();
                                    push_console(&ui, &console, console_buffer.clone());
                                }
                                console_buffer.clear();
                                write!(
                                    &mut console_buffer,
                                    "[selection] trying to select: '{}'",
                                    &clicked_item
                                )
                                .unwrap();
                                push_console(&ui, &console, console_buffer.clone());
                                ui.set_selected_layer_item(clicked_item.into());
                            }
                            Err(e) => {
                                status_msg.clear();
                                write!(&mut status_msg, "Error loading channel {}", channel_short)
                                    .unwrap();
                                console_buffer.clear();
                                write!(
                                    &mut console_buffer,
                                    "{}@{}: {}",
                                    channel_short, active_layer, e
                                )
                                .unwrap();
                                ui.report_error_with_status(
                                    &console,
                                    "channel",
                                    &status_msg,
                                    console_buffer.clone(),
                                );
                            }
                        }
                    } else {
                        ui.report_error(&console, "file", "No image cache loaded");
                    }
                } else {
                    ui.report_error(&console, "file", "No file loaded");
                }
            }
        }
    }
}

/// FUNKCJA: Zwijanie/rozwijanie WSZYSTKICH grup na raz za pomocą strzałek góra/dół
pub fn toggle_all_layer_groups(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
    expand: bool, // true = rozwiń wszystkie, false = zwiń wszystkie
) {
    if let Some(ui) = ui_handle.upgrade() {
        // Pobierz nazwy grup z image_cache (nie z ui_state, bo może być puste na starcie)
        let group_names = {
            if let Ok(state) = app_state.read() {
                if let Some(cache) = state.image_cache.as_ref() {
                    use crate::processing::channel_classification::determine_channel_group_with_config;
                    use crate::utils::channel_config::{get_fallback_config, load_channel_config};
                    // Use cached config or load once
                    let config = state.channel_config.clone().unwrap_or_else(|| {
                        load_channel_config().unwrap_or_else(|_| get_fallback_config())
                    });
                    let mut groups = HashSet::new();
                    for layer in &cache.layers_info {
                        let name_for_classification = if layer.name.is_empty() {
                            "Beauty"
                        } else {
                            &layer.name
                        };
                        let group_name =
                            determine_channel_group_with_config(name_for_classification, &config);
                        groups.insert(group_name);
                    }
                    groups.into_iter().collect::<Vec<String>>()
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        };

        if group_names.is_empty() {
            push_console(
                &ui,
                &console,
                "[toggle] no groups found in image cache".to_string(),
            );
            return;
        }

        let action = if expand { "expanded" } else { "collapsed" };

        // Ustaw stan wszystkich grup na raz
        if let Ok(mut state) = app_state.write() {
            for group_name in &group_names {
                state.ui_state.set_group_expansion(group_name, expand);
            }
        }

        // Odśwież model warstw
        refresh_layer_model(ui_handle.clone(), app_state.clone());

        push_console(
            &ui,
            &console,
            format!(
                "[toggle] {} ALL groups: {} (arrow navigation)",
                action,
                group_names.join(", ")
            ),
        );
        ui.set_status_text(
            format!(
                "{} all groups",
                if expand { "Expanded" } else { "Collapsed" }
            )
            .into(),
        );
    }
}
