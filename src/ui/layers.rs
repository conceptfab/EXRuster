use crate::ui::progress::{ProgressSink, UiProgress};
use crate::ui::push_console;
use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{lock_or_recover, ConsoleModel};
use crate::utils::normalize_channel_name;
use crate::AppWindow;
use slint::{ComponentHandle, Weak};
use std::collections::HashSet;
use std::sync::Arc;

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
            let (layers_model, layers_colors, layers_kinds, layers_font_sizes) =
                crate::ui::file_handlers::create_layers_model(&layers_info_vec, &ui, &app_state);
            ui.set_layers_model(layers_model);
            ui.set_layers_colors(layers_colors);
            ui.set_layers_kinds(layers_kinds);
            ui.set_layers_font_sizes(layers_font_sizes);
        }
    }
}

pub fn handle_layer_tree_click(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    clicked_item: String,
    kind: i32,
    console: ConsoleModel,
) {
    let trimmed = clicked_item.trim();

    // GRUPA — kind 0 (rozwinięta) lub 1 (zwinięta)
    if kind == 0 || kind == 1 {
        if let Some(ui) = ui_handle.upgrade() {
            let group_name = trimmed.to_string();

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
    // WARSTWA — kind 2
    else if kind == 2 {
        if let Some(ui) = ui_handle.upgrade() {
            let layer_name = trimmed.to_string();

            let real_layer_name = {
                let map = lock_or_recover(&crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER);
                map.get(&layer_name)
                    .cloned()
                    .unwrap_or_else(|| layer_name.clone())
            };

            push_console(
                &ui,
                &console,
                format!(
                    "[layer] clicked: {} (real='{}')",
                    layer_name, real_layer_name
                ),
            );
            ui.set_status_text(format!("Loading layer: {}", layer_name).into());

            let exposure = ui.get_exposure_value();
            let gamma = ui.get_gamma_value();
            let tonemap_mode = ui.get_tonemap_mode();

            // Explicit progress rather than ScopedProgress: the RAII guard would
            // finish on drop at the end of *this* function, long before the
            // background load it is reporting on.
            let progress = Arc::new(UiProgress::new(ui.as_weak()));
            progress.start_indeterminate(Some("Loading layer"));

            let ui_weak = ui.as_weak();
            let app_state = app_state.clone();

            // Load + render off the UI thread. In lazy mode load_layer hits the
            // disk; previously that ran on the event loop, under the write lock.
            rayon::spawn(move || {
                let loaded = (|| -> anyhow::Result<_> {
                    let mut state = app_state
                        .write()
                        .map_err(|_| anyhow::anyhow!("app state lock poisoned"))?;
                    let path = state
                        .current_file_path
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("No file loaded"))?;
                    let cache = state
                        .image_cache
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("No image cache loaded"))?;

                    cache.load_layer(&path, &real_layer_name, Some(progress.as_ref()))?;

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
                    Ok((cache.snapshot(), channels))
                })();

                match loaded {
                    Ok((snap, channels)) => {
                        let buffer = crate::io::image_cache::render_to_buffer(
                            &snap.pixels,
                            snap.width,
                            snap.height,
                            exposure,
                            gamma,
                            tonemap_mode,
                            snap.color_matrix,
                        );
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_exr_image(slint::Image::from_rgba8(buffer));
                                ui.set_status_text(
                                    format!(
                                        "Layer: {} | mode: RGB | channels: {}",
                                        real_layer_name, channels
                                    )
                                    .into(),
                                );
                                ui.set_selected_layer_item(layer_name.into());
                                progress.finish(None);
                            }
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status_text(
                                    format!("Error loading layer {}: {}", real_layer_name, e).into(),
                                );
                                progress.reset();
                            }
                        });
                    }
                }
            });
        }
    }
    // KANAŁY — kind 3
    else if kind == 3 {
        let is_dot = trimmed.starts_with("• ");
        let is_rgba_emoji = trimmed.starts_with("🔴")
            || trimmed.starts_with("🟢")
            || trimmed.starts_with("🔵")
            || trimmed.starts_with("⚪");
        if !(is_dot || is_rgba_emoji) {
            return;
        }

        if let Some(ui) = ui_handle.upgrade() {
            // Parse the clicked label on the UI thread — it needs the current
            // layer name as a fallback, but only for a cheap read.
            let current_layer_fallback = app_state
                .read()
                .ok()
                .and_then(|s| s.image_cache.as_ref().map(|c| c.current_layer_name.clone()))
                .unwrap_or_default();

            let (active_layer, channel_short) = {
                let s = trimmed;
                if let Some(at_pos) = s.rfind('@') {
                    let layer_display = s[at_pos + 1..].trim().to_string();
                    let layer = {
                        let map =
                            lock_or_recover(&crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER);
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
                        map.get(&key)
                            .cloned()
                            .unwrap_or(current_layer_fallback)
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

            let upper = channel_short.to_ascii_uppercase();
            let is_depth = upper == "Z" || upper.contains("DEPTH");

            push_console(
                &ui,
                &console,
                format!("[channel] clicked: {}@{}", channel_short, active_layer),
            );

            let progress = Arc::new(UiProgress::new(ui.as_weak()));
            progress.start_indeterminate(Some("Loading channel"));

            let ui_weak = ui.as_weak();
            let app_state = app_state.clone();
            let selected_item = clicked_item.clone();

            rayon::spawn(move || {
                let rendered = (|| -> anyhow::Result<_> {
                    let mut state = app_state
                        .write()
                        .map_err(|_| anyhow::anyhow!("app state lock poisoned"))?;
                    let path = state
                        .current_file_path
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("No file loaded"))?;
                    let cache = state
                        .image_cache
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("No image cache loaded"))?;

                    cache.load_channel(
                        &path,
                        &active_layer,
                        &channel_short,
                        Some(progress.as_ref()),
                    )?;
                    // Depth is rendered inverted; both modes percentile-normalize.
                    Ok(cache.process_depth_to_buffer(is_depth, Some(progress.as_ref())))
                })();

                match rendered {
                    Ok(buffer) => {
                        let mode_label = if is_depth {
                            "Depth (auto-normalized, inverted)"
                        } else {
                            "Grayscale (auto-normalized)"
                        };
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_exr_image(slint::Image::from_rgba8(buffer));
                                ui.set_status_text(
                                    format!(
                                        "Layer: {} | Channel: {} | mode: {}",
                                        active_layer, channel_short, mode_label
                                    )
                                    .into(),
                                );
                                ui.set_selected_layer_item(selected_item.into());
                                progress.finish(None);
                            }
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status_text(
                                    format!("Error loading channel {}: {}", channel_short, e)
                                        .into(),
                                );
                                progress.reset();
                            }
                        });
                    }
                }
            });
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
