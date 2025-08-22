use slint::{Weak, ComponentHandle, ModelRc, VecModel, SharedString, Color, invoke_from_event_loop};
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::collections::HashMap;
use crate::io::file_operations::{open_file_dialog, get_file_name};
use crate::io::exr_metadata;
use crate::io::image_cache::{ImageCache, LayerInfo};
use crate::io::full_exr_cache::{build_full_exr_cache, FullExrCacheData, FullLayer};
use crate::ui::ui_handlers::{push_console, lock_or_recover, ConsoleModel, ImageCacheType, CurrentFilePathType, FullExrCache};
use crate::{AppWindow, utils::{get_channel_info, UiErrorReporter, WeakProgressExt, patterns}};
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
        let _prog = ui.as_weak().scoped_progress()
            .start_indeterminate(Some("Opening EXR file..."));
        push_console(&ui, &console, "[file] opening EXR file".to_string());

        if let Some(path) = open_file_dialog() {
            handle_open_exr_from_path(ui_handle, current_file_path, image_cache, console, full_exr_cache, path);
        } else {
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
        let prog = patterns::file_operation(ui.as_weak(), "Loading", &path.display().to_string())
            .set(0.05, None);
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
                                                                // Apply histogram data to UI using the new unified method
                                                                hist_data.apply_to_ui(&ui2);
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
                                                                // Apply histogram data to UI using the new unified method
                                                                hist_data.apply_to_ui(&ui2);
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
                ui.report_error_with_status(&console, "meta", "Błąd odczytu metadanych", e);
                // Progress automatically resets on scope exit
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

/// Creates layers model for UI from LayerInfo with dictionary-based channel grouping
pub fn create_layers_model(
    layers_info: &[LayerInfo],
    ui: &AppWindow,
) -> (ModelRc<SharedString>, ModelRc<Color>, ModelRc<i32>) {
    use crate::utils::channel_config::{load_channel_config, get_fallback_config};
    use crate::processing::channel_classification::determine_channel_group_with_config;
    use std::collections::HashMap;
    
    // Ładuj konfigurację grupowania kanałów z obsługą błędów
    let config = match load_channel_config() {
        Ok(cfg) => {
            println!("[config] Successfully loaded channel groups from {}", crate::utils::channel_config::CHANNEL_CONFIG_PATH);
            cfg
        },
        Err(e) => {
            eprintln!("Warning: Failed to load channel config: {}. Using fallback.", e);
            // Zapisz komunikat do konsoli UI jeśli dostępna
            let fallback = get_fallback_config();
            println!("[config] Using fallback configuration with {} groups", fallback.groups.len());
            fallback
        }
    };
    
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

        // Grupuj kanały według słownika
        let mut channel_groups: HashMap<String, Vec<String>> = HashMap::new();
        
        for channel in &layer.channels {
            let short_name = channel.name.split('.').last().unwrap_or(&channel.name).to_string();
            let group_name = determine_channel_group_with_config(&channel.name, &config);
            channel_groups.entry(group_name).or_insert_with(Vec::new).push(short_name);
        }
        
        // Sortuj grupy według priorytetu z konfiguracji
        let mut sorted_groups: Vec<(String, Vec<String>)> = channel_groups.into_iter().collect();
        sorted_groups.sort_by(|a, b| {
            let a_priority = config.group_priority_order.iter().position(|x| {
                config.groups.get(x).map(|g| &g.name) == Some(&a.0)
            }).unwrap_or(999);
            let b_priority = config.group_priority_order.iter().position(|x| {
                config.groups.get(x).map(|g| &g.name) == Some(&b.0)
            }).unwrap_or(999);
            a_priority.cmp(&b_priority)
        });
        
        for (group_name, mut channels) in sorted_groups {
            // Grupa nagłówek jeśli nie jest to Base z pojedynczymi RGBA
            let is_simple_rgba = group_name == "Base" && channels.len() <= 4 && 
                channels.iter().all(|c| matches!(c.to_uppercase().as_str(), "R" | "G" | "B" | "A"));
            
            if !is_simple_rgba {
                items.push(format!("  📂 {}", group_name).into());
                colors.push(ui.get_layers_color_group()); // Nowy kolor dla grup
                font_sizes.push(11);
            }
            
            // Sortuj kanały w grupie
            if group_name == "Base" {
                // Specjalna kolejność dla RGBA
                let mut ordered: Vec<String> = Vec::new();
                let wanted_order = ["R", "G", "B", "A"];
                for wanted in wanted_order {
                    if let Some(pos) = channels.iter().position(|c| c.to_uppercase() == wanted) {
                        ordered.push(channels.remove(pos));
                    }
                }
                channels.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
                ordered.extend(channels);
                channels = ordered;
            } else {
                channels.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            }
            
            let indent = if is_simple_rgba { "    " } else { "      " };
            
            for ch in channels {
                // Emoji dla RGBA, dot dla innych
                let (_color, emoji, display_ch) = get_channel_info(&ch, ui);
                let base = format!("{}{} {}", indent, emoji, display_ch);
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
    }

    (
        ModelRc::new(VecModel::from(items)),
        ModelRc::new(VecModel::from(colors)),
        ModelRc::new(VecModel::from(font_sizes)),
    )
}