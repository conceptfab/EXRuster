use crate::io::exr_metadata;
use crate::io::file_operations::{get_file_name, open_file_dialog};
use crate::io::full_exr_cache::build_full_exr_cache;
use crate::io::image_cache::{ImageCache, LayerInfo};
use crate::io::lazy_exr_loader::LazyExrLoader;
use crate::log_warn;
use crate::ui::progress::{schedule_progress_finish_with_reset, UiProgress};
use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{lock_or_recover, push_console, ConsoleModel};
use crate::{
    utils::{get_channel_info, UiErrorReporter},
    AppWindow,
};
use anyhow::{Context, Result};
use slint::{
    invoke_from_event_loop, Color, ComponentHandle, ModelRc, SharedString, VecModel, Weak,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// File size threshold for light mode loading (>700 MB)
const LIGHT_MODE_FILE_SIZE_THRESHOLD: u64 = 700 * 1024 * 1024;

/// Shared histogram computation — updates histogram in cache and applies to UI
pub fn apply_histogram_to_ui(ui: &AppWindow, app_state: &SharedAppState) {
    if let Ok(mut state) = app_state.write() {
        if let Some(ref mut cache) = state.image_cache {
            if let Ok(()) = cache.update_histogram() {
                if let Some(hist_data) = cache.get_histogram_data() {
                    hist_data.apply_to_ui(ui);
                    ui.set_histogram_total_pixels(hist_data.total_pixels as i32);
                    let p1 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.01);
                    let p50 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.50);
                    let p99 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.99);
                    ui.set_histogram_p1(p1);
                    ui.set_histogram_p50(p50);
                    ui.set_histogram_p99(p99);
                }
            }
        }
    }
}

// Global static variables for layer mapping (to be moved to state in future refactoring)
pub static ITEM_TO_LAYER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub static DISPLAY_TO_REAL_LAYER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Handles EXR file opening callback
pub fn handle_open_exr(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        push_console(&ui, &console, "[file] opening EXR file".to_string());
        ui.set_status_text("Opening EXR file...".into());
        ui.set_progress_value(-1.0);

        if let Some(path) = open_file_dialog() {
            handle_open_exr_from_path(ui_handle, app_state, console, path);
        } else {
            ui.set_status_text("File selection canceled".into());
            ui.set_progress_value(0.0);
            push_console(&ui, &console, "[file] selection canceled".to_string());
        }
    }
}

/// Identical procedure as in `handle_open_exr`, but for already known path
pub fn handle_open_exr_from_path(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
    path: PathBuf,
) {
    if let Some(ui) = ui_handle.upgrade() {
        ui.set_status_text(format!("Loading: {}", path.display()).into());
        ui.set_progress_value(0.05);
        push_console(
            &ui,
            &console,
            format!(
                "{{\"event\":\"file.open\",\"path\":\"{}\"}}",
                path.display()
            ),
        );

        let is_hdr = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.eq_ignore_ascii_case("hdr"))
            .unwrap_or(false);
        if is_hdr {
            handle_open_hdr_from_path(ui_handle, app_state, console, path);
            return;
        }

        // Load EXR file metadata and update UI
        match load_metadata(&ui, &path, &console) {
            Ok(()) => {
                // Save file path
                if let Ok(mut state) = app_state.write() {
                    state.current_file_path = Some(path.clone());
                    ui.set_current_file_path(path.display().to_string().into());
                }

                // Asynchronous loading: FULL vs LAZY path selection
                let file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let force_lazy = std::env::var("EXRUSTER_LAZY_OPEN").ok().as_deref() == Some("1");
                let use_lazy = force_lazy || file_size_bytes > LIGHT_MODE_FILE_SIZE_THRESHOLD;

                ui.set_progress_value(0.22);
                ui.set_status_text(
                    (if use_lazy {
                        "Reading EXR (lazy)..."
                    } else {
                        "Reading EXR (full)..."
                    })
                    .into(),
                );
                ui.set_progress_value(-1.0);

                // Get current processing parameters
                let exposure0 = ui.get_exposure_value();
                let gamma0 = ui.get_gamma_value();
                let tonemap_mode0 = ui.get_tonemap_mode();

                let ui_weak = ui.as_weak();
                let app_state_c = app_state.clone();
                let path_c = path.clone();

                if use_lazy {
                    let progress = std::sync::Arc::new(UiProgress::new(ui.as_weak()));
                    rayon::spawn(move || {
                        let t_start = Instant::now();
                        // Initialize LazyExrLoader (raportuje postęp odczytu na podstawie bajtów)
                        let lazy_res =
                            LazyExrLoader::new(path_c.clone(), 20, Some(progress)).map(std::sync::Arc::new);

                        match lazy_res {
                            Ok(lazy_loader) => {
                                let cache_res =
                                    ImageCache::new_with_lazy_loader(&path_c, lazy_loader.clone());
                                match cache_res {
                                    Ok(cache) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                // Single write lock: update state, generate image, get layers
                                                let (img, layers_info_vec) = {
                                                    if let Ok(mut state) = app_state_c.write() {
                                                        state.full_exr_cache = None;
                                                        state.image_cache = Some(cache);
                                                        let li = state
                                                            .image_cache
                                                            .as_ref()
                                                            .map(|c| c.layers_info.clone())
                                                            .unwrap_or_default();
                                                        let img = state
                                                            .image_cache
                                                            .as_ref()
                                                            .map(|c| {
                                                                c.process_to_image(
                                                                    exposure0,
                                                                    gamma0,
                                                                    tonemap_mode0,
                                                                )
                                                            })
                                                            .unwrap_or_else(|| ui2.get_exr_image());
                                                        (img, li)
                                                    } else {
                                                        (ui2.get_exr_image(), vec![])
                                                    }
                                                };
                                                ui2.set_exr_image(img);

                                                apply_histogram_to_ui(&ui2, &app_state_c);
                                                if !layers_info_vec.is_empty() {
                                                    // Create a temporary SharedUiState wrapper for compatibility
                                                    let (
                                                        layers_model,
                                                        layers_colors,
                                                        layers_kinds,
                                                        layers_font_sizes,
                                                    ) = create_layers_model(
                                                        &layers_info_vec,
                                                        &ui2,
                                                        &app_state_c,
                                                    );
                                                    ui2.set_layers_model(layers_model);
                                                    ui2.set_layers_colors(layers_colors);
                                                    ui2.set_layers_kinds(layers_kinds);
                                                    ui2.set_layers_font_sizes(layers_font_sizes);
                                                }

                                                let mut log = ui2.get_console_text().to_string();
                                                if !log.is_empty() {
                                                    log.push('\n');
                                                }
                                                log.push_str(&format!(
                                                    "[lazy] metadata ready in {} ms",
                                                    t_start.elapsed().as_millis()
                                                ));
                                                ui2.set_console_text(log.into());
                                                ui2.set_status_text("Loaded (lazy)".into());
                                                schedule_progress_finish_with_reset(ui2.as_weak());
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                ui2.set_status_text(
                                                    format!(
                                                        "Read error '{}': {}",
                                                        get_file_name(&path_c),
                                                        e
                                                    )
                                                    .into(),
                                                );
                                                let mut log = ui2.get_console_text().to_string();
                                                if !log.is_empty() {
                                                    log.push('\n');
                                                }
                                                log.push_str(&format!("[error] lazy open: {}", e));
                                                ui2.set_console_text(log.into());
                                                ui2.set_progress_value(0.0);
                                            }
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = invoke_from_event_loop(move || {
                                    if let Some(ui2) = ui_weak.upgrade() {
                                        ui2.set_status_text(
                                            format!(
                                                "Read error '{}': {}",
                                                get_file_name(&path_c),
                                                e
                                            )
                                            .into(),
                                        );
                                        let mut log = ui2.get_console_text().to_string();
                                        if !log.is_empty() {
                                            log.push('\n');
                                        }
                                        log.push_str(&format!("[error] lazy open: {}", e));
                                        ui2.set_console_text(log.into());
                                        ui2.set_progress_value(0.0);
                                    }
                                });
                            }
                        }
                    });
                } else {
                    // FULL path (existing)
                    let progress = std::sync::Arc::new(UiProgress::new(ui.as_weak()));
                    rayon::spawn(move || {
                        let t_start = Instant::now();
                        let full_res =
                            build_full_exr_cache(&path_c, Some(progress.as_ref())).map(std::sync::Arc::new);
                        match full_res {
                            Ok(full) => {
                                let t_new = Instant::now();
                                let cache_res =
                                    ImageCache::new_with_full_cache(&path_c, full.clone());
                                match cache_res {
                                    Ok(cache) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                // Single write lock: update state, generate image, get layers
                                                let (img, layers_info_len, layers_info_vec) = {
                                                    if let Ok(mut state) = app_state_c.write() {
                                                        state.full_exr_cache = Some(full.clone());
                                                        state.image_cache = Some(cache);
                                                        let li = state
                                                            .image_cache
                                                            .as_ref()
                                                            .map(|c| c.layers_info.clone())
                                                            .unwrap_or_default();
                                                        let img = state
                                                            .image_cache
                                                            .as_ref()
                                                            .map(|c| {
                                                                c.process_to_image(
                                                                    exposure0,
                                                                    gamma0,
                                                                    tonemap_mode0,
                                                                )
                                                            })
                                                            .unwrap_or_else(|| ui2.get_exr_image());
                                                        (img, li.len(), li)
                                                    } else {
                                                        (ui2.get_exr_image(), 0usize, Vec::new())
                                                    }
                                                };
                                                ui2.set_exr_image(img);

                                                apply_histogram_to_ui(&ui2, &app_state_c);

                                                if !layers_info_vec.is_empty() {
                                                    // Create a temporary SharedUiState wrapper for compatibility
                                                    let (
                                                        layers_model,
                                                        layers_colors,
                                                        layers_kinds,
                                                        layers_font_sizes,
                                                    ) = create_layers_model(
                                                        &layers_info_vec,
                                                        &ui2,
                                                        &app_state_c,
                                                    );
                                                    ui2.set_layers_model(layers_model);
                                                    ui2.set_layers_colors(layers_colors);
                                                    ui2.set_layers_kinds(layers_kinds);
                                                    ui2.set_layers_font_sizes(layers_font_sizes);
                                                }
                                                let mut log = ui2.get_console_text().to_string();
                                                let mut append = |line: String| {
                                                    if !log.is_empty() {
                                                        log.push('\n');
                                                    }
                                                    log.push_str(&line);
                                                };
                                                append(format!(
                                                    "[cache] cache created ({} ms)",
                                                    t_new.elapsed().as_millis()
                                                ));
                                                append(format!("[preview] image updated (exp: {:.2}, gamma: {:.2})", exposure0, gamma0));
                                                append(format!(
                                                    "[layers] count: {}",
                                                    layers_info_len
                                                ));
                                                ui2.set_console_text(log.into());
                                                ui2.set_status_text(
                                                    format!(
                                                        "Loaded in {} ms",
                                                        t_start.elapsed().as_millis()
                                                    )
                                                    .into(),
                                                );
                                                schedule_progress_finish_with_reset(ui2.as_weak());
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                ui2.set_status_text(
                                                    format!(
                                                        "Read error '{}': {}",
                                                        get_file_name(&path_c),
                                                        e
                                                    )
                                                    .into(),
                                                );
                                                let mut log = ui2.get_console_text().to_string();
                                                if !log.is_empty() {
                                                    log.push('\n');
                                                }
                                                log.push_str(&format!(
                                                    "[error] reading file '{}': {}",
                                                    get_file_name(&path_c),
                                                    e
                                                ));
                                                ui2.set_console_text(log.into());
                                                ui2.set_progress_value(0.0);
                                            }
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = invoke_from_event_loop(move || {
                                    if let Some(ui2) = ui_weak.upgrade() {
                                        ui2.set_status_text(
                                            format!(
                                                "Read error '{}': {}",
                                                get_file_name(&path_c),
                                                e
                                            )
                                            .into(),
                                        );
                                        let mut log = ui2.get_console_text().to_string();
                                        if !log.is_empty() {
                                            log.push('\n');
                                        }
                                        log.push_str(&format!(
                                            "[error] reading file '{}': {}",
                                            get_file_name(&path_c),
                                            e
                                        ));
                                        ui2.set_console_text(log.into());
                                        ui2.set_progress_value(0.0);
                                    }
                                });
                            }
                        }
                    });
                }
            }
            Err(e) => {
                ui.report_error_with_status(&console, "meta", "Błąd odczytu metadanych", e);
                ui.set_progress_value(0.0);
            }
        }
    }
}

/// Handles opening a Radiance HDR (.hdr) file. Single layer, single-shot load.
pub fn handle_open_hdr_from_path(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
    path: PathBuf,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };

    if let Ok(mut state) = app_state.write() {
        state.current_file_path = Some(path.clone());
        ui.set_current_file_path(path.display().to_string().into());
    }
    ui.set_meta_text("".into());
    ui.set_meta_table_keys(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
    ui.set_meta_table_values(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
    ui.set_status_text("Reading HDR...".into());
    ui.set_progress_value(-1.0);

    let exposure0 = ui.get_exposure_value();
    let gamma0 = ui.get_gamma_value();
    let tonemap_mode0 = ui.get_tonemap_mode();
    let ui_weak = ui.as_weak();
    let app_state_c = app_state.clone();
    let path_c = path.clone();

    rayon::spawn(move || {
        let t_start = Instant::now();
        let cache_res = ImageCache::new_from_hdr(&path_c);
        match cache_res {
            Ok(cache) => {
                let _ = invoke_from_event_loop(move || {
                    if let Some(ui2) = ui_weak.upgrade() {
                        let (img, layers_info_vec) = {
                            if let Ok(mut state) = app_state_c.write() {
                                state.full_exr_cache = None;
                                state.image_cache = Some(cache);
                                let li = state
                                    .image_cache
                                    .as_ref()
                                    .map(|c| c.layers_info.clone())
                                    .unwrap_or_default();
                                let img = state
                                    .image_cache
                                    .as_ref()
                                    .map(|c| c.process_to_image(exposure0, gamma0, tonemap_mode0))
                                    .unwrap_or_else(|| ui2.get_exr_image());
                                (img, li)
                            } else {
                                (ui2.get_exr_image(), vec![])
                            }
                        };
                        ui2.set_exr_image(img);
                        apply_histogram_to_ui(&ui2, &app_state_c);
                        if !layers_info_vec.is_empty() {
                            let (layers_model, layers_colors, layers_kinds, layers_font_sizes) =
                                create_layers_model(&layers_info_vec, &ui2, &app_state_c);
                            ui2.set_layers_model(layers_model);
                            ui2.set_layers_colors(layers_colors);
                            ui2.set_layers_kinds(layers_kinds);
                            ui2.set_layers_font_sizes(layers_font_sizes);
                        }
                        let mut log = ui2.get_console_text().to_string();
                        if !log.is_empty() {
                            log.push('\n');
                        }
                        log.push_str(&format!(
                            "[hdr] loaded in {} ms",
                            t_start.elapsed().as_millis()
                        ));
                        ui2.set_console_text(log.into());
                        ui2.set_status_text("Loaded (HDR)".into());
                        schedule_progress_finish_with_reset(ui2.as_weak());
                    }
                });
            }
            Err(e) => {
                let _ = invoke_from_event_loop(move || {
                    if let Some(ui2) = ui_weak.upgrade() {
                        ui2.set_status_text(
                            format!("Read error '{}': {}", get_file_name(&path_c), e).into(),
                        );
                        let mut log = ui2.get_console_text().to_string();
                        if !log.is_empty() {
                            log.push('\n');
                        }
                        log.push_str(&format!("[error] hdr open: {}", e));
                        ui2.set_console_text(log.into());
                        ui2.set_progress_value(0.0);
                    }
                });
            }
        }
    });

    push_console(&ui, &console, "[file] opening HDR file".to_string());
}

/// Loads EXR file metadata and updates UI
pub fn load_metadata(
    ui: &AppWindow,
    path: &Path,
    console: &ConsoleModel,
) -> Result<(), anyhow::Error> {
    // Build and display metadata in Meta tab with better error handling
    let meta = exr_metadata::read_and_group_metadata(path)
        .with_context(|| format!("Failed to read EXR metadata from: {}", path.display()))?;

    // Text version (left as fallback)
    let lines = exr_metadata::build_ui_lines(&meta);
    let text = lines.join("\n");
    ui.set_meta_text(text.into());

    // Tabular version 2 columns
    let rows = exr_metadata::build_ui_rows(&meta);
    let (keys, vals): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    ui.set_meta_table_keys(ModelRc::new(VecModel::from(
        keys.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    )));
    ui.set_meta_table_values(ModelRc::new(VecModel::from(
        vals.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    )));
    push_console(ui, console, format!("[meta] layers: {}", meta.layers.len()));

    Ok(())
}

/// Creates layers model for UI from LayerInfo, grouped by channel types from config.
/// The new hierarchy is Group -> Layer -> Channel.
/// Row kind for the layers tree UI.
/// - 0 = group header (expanded)
/// - 1 = group header (collapsed)
/// - 2 = layer (under a group)
/// - 3 = channel (leaf)
pub fn create_layers_model(
    layers_info: &[LayerInfo],
    ui: &AppWindow,
    app_state: &crate::ui::state::SharedAppState,
) -> (
    ModelRc<SharedString>,
    ModelRc<Color>,
    ModelRc<i32>,
    ModelRc<i32>,
) {
    use crate::processing::channel_classification::determine_channel_group_with_config;
    use crate::utils::channel_config::{get_fallback_config, load_channel_config};
    use std::collections::HashMap;

    // Use cached config from AppState, or load and cache it
    let config = {
        let cached = app_state.read().ok().and_then(|s| s.channel_config.clone());
        if let Some(c) = cached {
            c
        } else {
            let c = load_channel_config().unwrap_or_else(|e| {
                log_warn!("Failed to load channel config for UI: {}. Using fallback.", e);
                get_fallback_config()
            });
            if let Ok(mut state) = app_state.write() {
                state.channel_config = Some(c.clone());
            }
            c
        }
    };

    let name_to_key: HashMap<&str, &str> = config
        .groups
        .iter()
        .map(|(key, def)| (def.name.as_str(), key.as_str()))
        .collect();

    // HashMap for O(1) priority lookup instead of O(n) iter().position() in sort
    let group_priority_map: HashMap<&str, usize> = config
        .group_priority_order
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();

    let mut items: Vec<SharedString> = Vec::new();
    let mut colors: Vec<Color> = Vec::new();
    let mut font_sizes: Vec<i32> = Vec::new();
    let mut kinds: Vec<i32> = Vec::new();

    lock_or_recover(&ITEM_TO_LAYER).clear();
    lock_or_recover(&DISPLAY_TO_REAL_LAYER).clear();

    // 1. Group layers by their group name
    let mut grouped_layers: HashMap<String, Vec<&LayerInfo>> = HashMap::new();
    for layer in layers_info {
        // Use layer.name for classification. If it's empty, it's the base "Beauty" layer.
        let name_for_classification = if layer.name.is_empty() {
            "Beauty"
        } else {
            &layer.name
        };
        let group_name = determine_channel_group_with_config(name_for_classification, &config);
        grouped_layers.entry(group_name).or_default().push(layer);
    }

    // 2. Sort the groups based on config priority (O(1) lookup via HashMap)
    let mut sorted_groups: Vec<_> = grouped_layers.into_iter().collect();
    sorted_groups.sort_by(|a, b| {
        let a_key = name_to_key.get(a.0.as_str()).unwrap_or(&"");
        let b_key = name_to_key.get(b.0.as_str()).unwrap_or(&"");
        let a_priority = *group_priority_map.get(a_key).unwrap_or(&999);
        let b_priority = *group_priority_map.get(b_key).unwrap_or(&999);
        a_priority.cmp(&b_priority)
    });

    // 3. Build the UI model from the new hierarchy
    // Reuse string buffer for formatting
    use std::fmt::Write;
    let mut format_buffer = String::with_capacity(128);

    for (group_name, layers) in sorted_groups {
        // Add group header with expand/collapse arrow
        let state_guard = match app_state.read() {
            Ok(guard) => guard,
            Err(_) => {
                log_warn!("Failed to acquire read lock on app_state");
                return (
                    ModelRc::new(VecModel::from(items)),
                    ModelRc::new(VecModel::from(colors)),
                    ModelRc::new(VecModel::from(kinds)),
                    ModelRc::new(VecModel::from(font_sizes)),
                );
            }
        };
        let is_expanded = state_guard.ui_state.is_group_expanded(&group_name);
        // Clean display — arrows and folder icon rendered by the UI layer from `kind`.
        items.push(SharedString::from(group_name.as_str()));
        colors.push(ui.get_layers_color_group());
        font_sizes.push(12);
        kinds.push(if is_expanded { 0 } else { 1 });
        drop(state_guard);

        // Show layers only if group is expanded
        if is_expanded {
            // Sort layers alphabetically within the group
            let mut sorted_layers = layers;
            sorted_layers.sort_by_key(|l| &l.name);

            for layer in sorted_layers {
                let display_name = if layer.name.is_empty() {
                    "Beauty".to_string()
                } else {
                    layer.name.clone()
                };
                {
                    let mut map = lock_or_recover(&DISPLAY_TO_REAL_LAYER);
                    map.insert(display_name.clone(), layer.name.clone());
                }

                // Layer row — indentation handled by UI based on kind.
                items.push(SharedString::from(display_name.as_str()));
                colors.push(ui.get_layers_color_default());
                font_sizes.push(11);
                kinds.push(2);

                // Add channels for the layer (always show all channels)
                {
                    let mut channels_to_sort = layer.channels.clone();
                    // Special sort for RGBA (eq_ignore_ascii_case avoids to_uppercase allocation)
                    let rgba_order = |name: &str| -> usize {
                        ["R", "G", "B", "A"]
                            .iter()
                            .position(|&s| name.eq_ignore_ascii_case(s))
                            .unwrap_or(99)
                    };
                    channels_to_sort.sort_by(|a, b| {
                        let a_pos = rgba_order(&a.name);
                        let b_pos = rgba_order(&b.name);
                        if a_pos != 99 || b_pos != 99 {
                            a_pos.cmp(&b_pos)
                        } else {
                            a.name.cmp(&b.name)
                        }
                    });

                    for ch_info in channels_to_sort {
                        let (_color, emoji, display_ch) = get_channel_info(&ch_info.name, ui);
                        // Clean channel line — indentation handled by UI based on kind.
                        format_buffer.clear();
                        write!(
                            &mut format_buffer,
                            "{} {} @{}",
                            emoji, display_ch, display_name
                        )
                        .unwrap();
                        let line = format_buffer.clone();

                        lock_or_recover(&ITEM_TO_LAYER).insert(line.clone(), layer.name.clone());

                        items.push(line.into());
                        colors.push(_color);
                        font_sizes.push(10);
                        kinds.push(3);
                    }
                }
            }
        }
    }

    (
        ModelRc::new(VecModel::from(items)),
        ModelRc::new(VecModel::from(colors)),
        ModelRc::new(VecModel::from(kinds)),
        ModelRc::new(VecModel::from(font_sizes)),
    )
}
