use slint::{Weak, ComponentHandle, ModelRc, VecModel, SharedString, Color, invoke_from_event_loop};
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::collections::HashMap;
use crate::io::file_operations::{open_file_dialog, get_file_name};
use crate::io::exr_metadata;
use crate::io::image_cache::{ImageCache, LayerInfo};
use crate::io::full_exr_cache::{build_full_exr_cache, FullExrCacheData, FullLayer};
use crate::ui::progress::{UiProgress, ProgressSink};
use crate::ui::ui_handlers::{push_console, lock_or_recover, ConsoleModel, ImageCacheType, CurrentFilePathType, FullExrCache};
use crate::{AppWindow, utils::get_channel_info};
use anyhow::{Result, Context};

// Global static variables for layer mapping (to be moved to state in future refactoring)
static ITEM_TO_LAYER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

static DISPLAY_TO_REAL_LAYER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Handles EXR file opening callback
pub fn handle_open_exr(
    ui_handle: Weak<AppWindow>,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console: ConsoleModel,
    full_exr_cache: FullExrCache,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let prog = UiProgress::new(ui.as_weak());
        prog.start_indeterminate(Some("Opening EXR file..."));
        push_console(&ui, &console, "[file] opening EXR file".to_string());

        if let Some(path) = open_file_dialog() {
            handle_open_exr_from_path(ui_handle, current_file_path, image_cache, console, full_exr_cache, path);
        } else {
            prog.reset();
            ui.set_status_text("File selection canceled".into());
            push_console(&ui, &console, "[file] selection canceled".to_string());
        }
    }
}

/// Identical procedure as in `handle_open_exr`, but for already known path
pub fn handle_open_exr_from_path(
    ui_handle: Weak<AppWindow>,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console: ConsoleModel,
    full_exr_cache: FullExrCache,
    path: PathBuf,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let prog = UiProgress::new(ui.as_weak());
        prog.set(0.05, Some(&format!("Loading: {}", path.display())));
        push_console(&ui, &console, format!("{{\"event\":\"file.open\",\"path\":\"{}\"}}", path.display()));

        // Load EXR file metadata and update UI
        match load_metadata(&ui, &path, &console) {
            Ok(()) => {
                // Save file path
                { *lock_or_recover(&current_file_path) = Some(path.clone()); }

                // Asynchronous loading: FULL vs LIGHT path selection
                let file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let force_light = std::env::var("EXRUSTER_LIGHT_OPEN").ok().as_deref() == Some("1");
                let use_light = force_light || file_size_bytes > 700 * 1024 * 1024; // >700MB ⇒ light

                prog.set(0.22, Some(if use_light { "Reading EXR (light)..." } else { "Reading EXR (full)..." }));
                ui.set_progress_value(-1.0);

                // Get current processing parameters
                let exposure0 = ui.get_exposure_value();
                let gamma0 = ui.get_gamma_value();
                let tonemap_mode0 = ui.get_tonemap_mode() as i32;

                let ui_weak = ui.as_weak();
                let image_cache_c = image_cache.clone();
                let full_exr_cache_c = full_exr_cache.clone();
                let path_c = path.clone();

                if use_light {
                    rayon::spawn(move || {
                        let t_start = Instant::now();
                        // Read only the best layer and build minimal cache
                        let light_res = (|| -> anyhow::Result<std::sync::Arc<FullExrCacheData>> {
                            let layers = crate::io::image_cache::extract_layers_info(&path_c)?;
                            let best = crate::io::image_cache::find_best_layer(&layers);
                            let lc = crate::io::image_cache::load_all_channels_for_layer(&path_c, &best, None)?;
                            let fl = FullLayer {
                                name: lc.layer_name.clone(),
                                width: lc.width,
                                height: lc.height,
                                channel_names: lc.channel_names.clone(),
                                channel_data: lc.channel_data.to_vec(),
                            };
                            Ok(std::sync::Arc::new(FullExrCacheData { layers: vec![fl] }))
                        })();

                        match light_res {
                            Ok(full) => {
                                let cache_res = ImageCache::new_with_full_cache(&path_c, full.clone());
                                match cache_res {
                                    Ok(cache) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                { let mut g = lock_or_recover(&full_exr_cache_c); *g = Some(full.clone()); }
                                                { let mut cg = lock_or_recover(&image_cache_c); *cg = Some(cache); }
                                                // Generate image on UI thread
                                                let img = {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut c) = *guard { 
                                                        c.process_to_image(exposure0, gamma0, tonemap_mode0)
                                                    } else { 
                                                        ui2.get_exr_image() 
                                                    }
                                                };
                                                ui2.set_exr_image(img);

                                                // Automatically calculate histogram for new image
                                                {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut cache) = *guard {
                                                        if let Ok(()) = cache.update_histogram() {
                                                            if let Some(hist_data) = cache.get_histogram_data() {
                                                                // Pass histogram data to UI
                                                                let red_bins: Vec<i32> = hist_data.red_bins.iter().map(|&x| x as i32).collect();
                                                                let green_bins: Vec<i32> = hist_data.green_bins.iter().map(|&x| x as i32).collect();
                                                                let blue_bins: Vec<i32> = hist_data.blue_bins.iter().map(|&x| x as i32).collect();
                                                                let lum_bins: Vec<i32> = hist_data.luminance_bins.iter().map(|&x| x as i32).collect();
                                                                
                                                                ui2.set_histogram_red_data(ModelRc::new(VecModel::from(red_bins)));
                                                                ui2.set_histogram_green_data(ModelRc::new(VecModel::from(green_bins)));
                                                                ui2.set_histogram_blue_data(ModelRc::new(VecModel::from(blue_bins)));
                                                                ui2.set_histogram_luminance_data(ModelRc::new(VecModel::from(lum_bins)));
                                                                
                                                                // Statistics
                                                                ui2.set_histogram_min_value(hist_data.min_value);
                                                                ui2.set_histogram_max_value(hist_data.max_value);
                                                                ui2.set_histogram_total_pixels(hist_data.total_pixels as i32);
                                                                
                                                                // Percentiles
                                                                let p1 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.01);
                                                                let p50 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.50);
                                                                let p99 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.99);
                                                                ui2.set_histogram_p1(p1);
                                                                ui2.set_histogram_p50(p50);
                                                                ui2.set_histogram_p99(p99);
                                                            }
                                                        }
                                                    }
                                                }

                                                // Update layers list
                                                let layers_info_vec = {
                                                    let guard = lock_or_recover(&image_cache_c);
                                                    guard.as_ref().map(|c| c.layers_info.clone()).unwrap_or_default()
                                                };
                                                if !layers_info_vec.is_empty() {
                                                    let (layers_model, layers_colors, layers_font_sizes) = create_layers_model(&layers_info_vec, &ui2);
                                                    ui2.set_layers_model(layers_model);
                                                    ui2.set_layers_colors(layers_colors);
                                                    ui2.set_layers_font_sizes(layers_font_sizes);
                                                }

                                                let mut log = ui2.get_console_text().to_string();
                                                if !log.is_empty() { log.push('\n'); }
                                                log.push_str(&format!("[light] image ready in {} ms", t_start.elapsed().as_millis()));
                                                ui2.set_console_text(log.into());
                                                ui2.set_status_text("Loaded (light)".into());
                                                ui2.set_progress_value(1.0);
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                ui2.set_status_text(format!("Read error '{}': {}", get_file_name(&path_c), e).into());
                                                let mut log = ui2.get_console_text().to_string();
                                                if !log.is_empty() { log.push('\n'); }
                                                log.push_str(&format!("[error] light open: {}", e));
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
                                        ui2.set_status_text(format!("Read error '{}': {}", get_file_name(&path_c), e).into());
                                        let mut log = ui2.get_console_text().to_string();
                                        if !log.is_empty() { log.push('\n'); }
                                        log.push_str(&format!("[error] light open: {}", e));
                                        ui2.set_console_text(log.into());
                                        ui2.set_progress_value(0.0);
                                    }
                                });
                            }
                        }
                    });
                } else {
                    // FULL path (existing)
                    rayon::spawn(move || {
                        let t_start = Instant::now();
                        let full_res = build_full_exr_cache(&path_c, None).map(std::sync::Arc::new);
                        match full_res {
                            Ok(full) => {
                                let t_new = Instant::now();
                                let cache_res = ImageCache::new_with_full_cache(&path_c, full.clone());
                                match cache_res {
                                    Ok(cache) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                { let mut g = lock_or_recover(&full_exr_cache_c); *g = Some(full.clone()); }
                                                { let mut cg = lock_or_recover(&image_cache_c); *cg = Some(cache); }
                                                // Generate image on UI thread (Image is not Send)
                                                let (img, layers_info_len, layers_info_vec) = {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut c) = *guard {
                                                        let li = c.layers_info.clone();
                                                        (c.process_to_image(exposure0, gamma0, tonemap_mode0), li.len(), li)
                                                    } else {
                                                        (ui2.get_exr_image(), 0usize, Vec::new())
                                                    }
                                                };
                                                ui2.set_exr_image(img);

                                                // Automatically calculate histogram for new image
                                                {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut cache) = *guard {
                                                        if let Ok(()) = cache.update_histogram() {
                                                            if let Some(hist_data) = cache.get_histogram_data() {
                                                                // Pass histogram data to UI
                                                                let red_bins: Vec<i32> = hist_data.red_bins.iter().map(|&x| x as i32).collect();
                                                                let green_bins: Vec<i32> = hist_data.green_bins.iter().map(|&x| x as i32).collect();
                                                                let blue_bins: Vec<i32> = hist_data.blue_bins.iter().map(|&x| x as i32).collect();
                                                                let lum_bins: Vec<i32> = hist_data.luminance_bins.iter().map(|&x| x as i32).collect();
                                                                
                                                                ui2.set_histogram_red_data(ModelRc::new(VecModel::from(red_bins)));
                                                                ui2.set_histogram_blue_data(ModelRc::new(VecModel::from(blue_bins)));
                                                                ui2.set_histogram_green_data(ModelRc::new(VecModel::from(green_bins)));
                                                                ui2.set_histogram_luminance_data(ModelRc::new(VecModel::from(lum_bins)));
                                                                
                                                                // Statistics
                                                                ui2.set_histogram_min_value(hist_data.min_value);
                                                                ui2.set_histogram_max_value(hist_data.max_value);
                                                                ui2.set_histogram_total_pixels(hist_data.total_pixels as i32);
                                                                
                                                                // Percentiles
                                                                let p1 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.01);
                                                                let p50 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.50);
                                                                let p99 = hist_data.get_percentile(crate::processing::histogram::HistogramChannel::Luminance, 0.99);
                                                                ui2.set_histogram_p1(p1);
                                                                ui2.set_histogram_p50(p50);
                                                                ui2.set_histogram_p99(p99);
                                                            }
                                                        }
                                                    }
                                                }

                                                if !layers_info_vec.is_empty() {
                                                    let (layers_model, layers_colors, layers_font_sizes) = create_layers_model(&layers_info_vec, &ui2);
                                                    ui2.set_layers_model(layers_model);
                                                    ui2.set_layers_colors(layers_colors);
                                                    ui2.set_layers_font_sizes(layers_font_sizes);
                                                }
                                                let mut log = ui2.get_console_text().to_string();
                                                let mut append = |line: String| { if !log.is_empty() { log.push('\n'); } log.push_str(&line); };
                                                append(format!("[cache] cache created ({} ms)", t_new.elapsed().as_millis()));
                                                append(format!("[preview] image updated (exp: {:.2}, gamma: {:.2})", exposure0, gamma0));
                                                append(format!("[layers] count: {}", layers_info_len));
                                                ui2.set_console_text(log.into());
                                                ui2.set_status_text(format!("Loaded in {} ms", t_start.elapsed().as_millis()).into());
                                                ui2.set_progress_value(1.0);
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                ui2.set_status_text(format!("Read error '{}': {}", get_file_name(&path_c), e).into());
                                                let mut log = ui2.get_console_text().to_string();
                                                if !log.is_empty() { log.push('\n'); }
                                                log.push_str(&format!("[error] reading file '{}': {}", get_file_name(&path_c), e));
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
                                        ui2.set_status_text(format!("Read error '{}': {}", get_file_name(&path_c), e).into());
                                        let mut log = ui2.get_console_text().to_string();
                                        if !log.is_empty() { log.push('\n'); }
                                        log.push_str(&format!("[error] reading file '{}': {}", get_file_name(&path_c), e));
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
                ui.set_status_text(format!("Błąd odczytu metadanych: {}", e).into());
                push_console(&ui, &console, format!("[error][meta] {}", e));
                prog.reset();
            }
        }
    }
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
    ui.set_meta_table_keys(ModelRc::new(VecModel::from(keys.into_iter().map(SharedString::from).collect::<Vec<_>>())));
    ui.set_meta_table_values(ModelRc::new(VecModel::from(vals.into_iter().map(SharedString::from).collect::<Vec<_>>())));
    push_console(ui, console, format!("[meta] layers: {}", meta.layers.len()));
    
    Ok(())
}

/// Creates layers model for UI from LayerInfo
pub fn create_layers_model(
    layers_info: &[LayerInfo],
    ui: &AppWindow,
) -> (ModelRc<SharedString>, ModelRc<Color>, ModelRc<i32>) {
    // SIMPLIFIED TREE: Layer → actual channels (no groups). RGBA only if they exist in file.
    let mut items: Vec<SharedString> = Vec::new();
    let mut colors: Vec<Color> = Vec::new();
    let mut font_sizes: Vec<i32> = Vec::new();
    // Clear map
    lock_or_recover(&ITEM_TO_LAYER).clear();
    lock_or_recover(&DISPLAY_TO_REAL_LAYER).clear();
    for layer in layers_info {
        // Friendly name for empty RGBA layer
        let display_name = if layer.name.is_empty() { "Beauty".to_string() } else { layer.name.clone() };
        // Save mapping of display name to actual
        {
            let mut map = lock_or_recover(&DISPLAY_TO_REAL_LAYER);
            map.insert(display_name.clone(), layer.name.clone());
        }
        // Layer header row
        items.push(format!("📁 {}", display_name).into());
        colors.push(ui.get_layers_color_default());
        font_sizes.push(12);

        // Collect list of actual channels (short names)
        let mut short_channels: Vec<String> = layer
            .channels
            .iter()
            .map(|c| c.name.split('.').last().unwrap_or(&c.name).to_string())
            .collect();

        // Preserve order: R, G, B, A (if present), then rest alphabetically
        // Include synonyms: Red/Green/Blue/Alpha (case-insensitive)
        let mut ordered: Vec<String> = Vec::new();
        let wanted_groups: [&[&str]; 4] = [
            &["R", "RED"],
            &["G", "GREEN"],
            &["B", "BLUE"],
            &["A", "ALPHA"],
        ];
        for aliases in wanted_groups {
            if let Some(pos) = short_channels.iter().position(|s| {
                let su = s.to_ascii_uppercase();
                aliases.iter().any(|a| su == *a || su.starts_with(*a))
            }) {
                ordered.push(short_channels.remove(pos));
            }
        }
        short_channels.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        ordered.extend(short_channels);

        for ch in ordered {
            // Emoji for RGBA, dot for others, plus suffix @<layer> for uniqueness
            let (_color, emoji, display_ch) = get_channel_info(&ch, ui);
            let base = format!("    {} {}", emoji, display_ch);
            let line = format!("{} @{}", base, display_name);
            ITEM_TO_LAYER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(line.clone(), layer.name.clone());
            items.push(line.clone().into());
            let (c, _emoji2, _display2) = get_channel_info(&ch, ui);
            colors.push(c);
            font_sizes.push(10);
        }
    }

    (
        ModelRc::new(VecModel::from(items)),
        ModelRc::new(VecModel::from(colors)),
        ModelRc::new(VecModel::from(font_sizes)),
    )
}