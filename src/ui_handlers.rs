use slint::{Weak, ComponentHandle, Timer, TimerMode, ModelRc, VecModel, SharedString, Color};
use slint::invoke_from_event_loop;
use std::sync::{Arc, Mutex, MutexGuard};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use crate::image_cache::ImageCache;
use crate::file_operations::{open_file_dialog, get_file_name};
use std::rc::Rc;
// removed unused: use exr::prelude as exr;
use crate::exr_metadata;
use crate::progress::{ProgressSink, UiProgress};
use crate::utils::{get_channel_info, normalize_channel_name, human_size};
use std::fs::File;
use tiff::encoder::{TiffEncoder, colortype::{RGBA32Float, RGB32Float, Gray32Float}};
use tiff::tags::Tag;
use image::{ImageBuffer, Rgb};
use glam::Vec3;
use crate::image_processing::tone_map_and_gamma;

// Import komponentów Slint
use crate::AppWindow;
use crate::ThumbItem;

pub type ImageCacheType = Arc<Mutex<Option<ImageCache>>>;
pub type CurrentFilePathType = Arc<Mutex<Option<PathBuf>>>;
pub type ConsoleModel = Rc<VecModel<SharedString>>;
pub type GpuContextType = Arc<Mutex<Option<crate::gpu_context::GpuContext>>>;
use crate::full_exr_cache::{FullExrCacheData, FullLayer, build_full_exr_cache};
pub type FullExrCache = Arc<Mutex<Option<std::sync::Arc<FullExrCacheData>>>>;

/// Dodaje linię do modelu konsoli i aktualizuje tekst w `TextEdit` (console-text)
pub fn push_console(ui: &crate::AppWindow, console: &ConsoleModel, line: String) {
    console.push(line.clone().into());
    let mut joined = ui.get_console_text().to_string();
    if !joined.is_empty() { joined.push('\n'); }
    joined.push_str(&line);
    ui.set_console_text(joined.into());
}

static LAST_PREVIEW_LOG: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);



#[inline]
pub(crate) fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// Uproszczone: usunięty stan drzewa i globalny TREE_STATE
// Mapowanie linii modelu na nazwę warstwy (aby kanał wiedział, do której warstwy należy)
static ITEM_TO_LAYER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

// Mapowanie wyświetlanej nazwy warstwy → rzeczywista nazwa z pliku (np. "Beauty" → "")
static DISPLAY_TO_REAL_LAYER: std::sync::LazyLock<std::sync::Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

// Globalny dostęp do kontekstu GPU
static GPU_CONTEXT: std::sync::LazyLock<std::sync::Mutex<Option<Arc<Mutex<Option<crate::gpu_context::GpuContext>>>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

// Globalny stan akceleracji GPU
static GPU_ACCELERATION_ENABLED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

// Normalizacja nazw kanałów do skrótu R/G/B/A
#[inline]
fn normalize_channel_display_to_short(channel_display: &str) -> String { normalize_channel_name(channel_display) }

pub fn handle_layer_tree_click(
    ui_handle: Weak<AppWindow>,
    image_cache: ImageCacheType,
    clicked_item: String,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    // Sprawdź czy kliknięto na warstwę (zaczyna się od 📁)
    if clicked_item.starts_with("📁") {
                if let Some(ui) = ui_handle.upgrade() {
            // Wyodrębnij wyświetlaną nazwę warstwy (usuń emoji i spacje)
            let display_layer_name = clicked_item.trim_start_matches("📁").trim().to_string();
            // Zmapuj na rzeczywistą nazwę z pliku (np. "Beauty" → "")
            let layer_name = {
                let map = lock_or_recover(&DISPLAY_TO_REAL_LAYER);
                map.get(&display_layer_name).cloned().unwrap_or_else(|| display_layer_name.clone())
            };
            
            let mut status_msg = String::new();
            status_msg.push_str(&format!("Loading layer: {}", display_layer_name));
            push_console(&ui, &console, format!("[layer] clicked: {} (real='{}')", display_layer_name, layer_name));
            
            // Pobierz ścieżkę do pliku
            let file_path = {
                let path_guard = lock_or_recover(&current_file_path);
                path_guard.clone()
            };
            
            if let Some(path) = file_path {
                // Załaduj nową warstwę
                let mut cache_guard = lock_or_recover(&image_cache);
                if let Some(ref mut cache) = *cache_guard {
                    let prog = UiProgress::new(ui.as_weak());
                    prog.start_indeterminate(Some("Loading layer..."));
                    match cache.load_layer(&path, &layer_name, Some(&prog)) {
                        Ok(()) => {
                            // Pobierz aktualne wartości ekspozycji i gammy
                            let exposure = ui.get_exposure_value();
                            let gamma = ui.get_gamma_value();
                            // Warstwa → kompozyt RGB (z duplikowaniem brakujących kanałów)
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
                            prog.finish(Some("Layer loaded"));
                            // Zaznacz w liście wybraną warstwę
                            ui.set_selected_layer_item(format!("📁 {}", display_layer_name).into());
                        }
                        Err(e) => {
                            ui.set_status_text(format!("Error loading layer {}: {}", layer_name, e).into());
                            push_console(&ui, &console, format!("[error] loading layer {}: {}", layer_name, e));
                            prog.reset();
                        }
                    }
                }
            } else {
                ui.set_status_text("Error: No file loaded".into());
                push_console(&ui, &console, "[error] no file loaded".to_string());
            }
        }
    }
    // Sprawdź klik kanału (wiersz zaczyna się od „• ” lub emoji koloru)
    else {
        // próbujemy dopasować „    • X” lub „    🔴 R/🟢 G/🔵 B/⚪ A”
        let trimmed = clicked_item.trim();
        let is_dot = trimmed.starts_with("• ");
        let is_rgba_emoji = trimmed.starts_with("🔴") || trimmed.starts_with("🟢") || trimmed.starts_with("🔵") || trimmed.starts_with("⚪");
        if !(is_dot || is_rgba_emoji) { return; }

        // Ustal aktywną warstwę i skrót kanału z klikniętej linii (preferuj sufiks '@Warstwa' jeżeli jest obecny)
        if let Some(ui) = ui_handle.upgrade() {
            let file_path = {
                let path_guard = lock_or_recover(&current_file_path);
                path_guard.clone()
            };
            if file_path.is_none() { return; }

            let mut cache_guard = lock_or_recover(&image_cache);
            if let Some(ref mut cache) = *cache_guard {
                // Preferuj parsowanie z sufiksu '@Warstwa' aby uniknąć kolizji duplikatów w mapie
                let (active_layer, channel_short) = {
                    let s = trimmed;
                    if let Some(at_pos) = s.rfind('@') {
                        let layer_display = s[at_pos + 1..].trim().to_string();
                        // Zmapuj wyświetlaną nazwę warstwy na rzeczywistą (np. "Beauty" → "")
                        let layer = {
                            let map = lock_or_recover(&DISPLAY_TO_REAL_LAYER);
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
                        // Fallback: użyj mapy i dotychczasowego parsowania
                        let active_layer = {
                            let key = clicked_item.trim_end().to_string();
                            let map = lock_or_recover(&ITEM_TO_LAYER);
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
                // Jeżeli kliknięto na przyjazną nazwę (Red/Green/Blue/Alpha), zamień na skrót R/G/B/A
                let channel_short = normalize_channel_display_to_short(&channel_short);
                // NIE normalizujemy nazw — używamy 1:1 z pliku; jedynie tryb Depth rozpoznamy później po wzorcu

                let path = match file_path {
                    Some(p) => p,
                    None => {
                        ui.set_status_text("Błąd: brak ścieżki do pliku".into());
                        push_console(&ui, &console, "[error] brak ścieżki do pliku".to_string());
                        return;
                    }
                };
                // Brak specjalnego traktowania Cryptomatte – kanały jak w każdej warstwie

                let prog = UiProgress::new(ui.as_weak());
                prog.start_indeterminate(Some("Loading channel..."));
                match cache.load_channel(&path, &active_layer, &channel_short, Some(&prog)) {
                    Ok(()) => {
                        let _exposure = ui.get_exposure_value();
                        let _gamma = ui.get_gamma_value();

                        // Specjalny przypadek Depth: jeżeli nazwa kanału to Z/Depth, użyj process_depth_image z invertem= true (near jasne)
                        let upper = channel_short.to_ascii_uppercase();
                        if upper == "Z" || upper.contains("DEPTH") {
                            let image = cache.process_depth_image_with_progress(true, Some(&prog));
                            ui.set_exr_image(image);
                            ui.set_status_text(format!("Layer: {} | Channel: {} | mode: Depth (auto-normalized, inverted)", active_layer, channel_short).into());
                            push_console(&ui, &console, format!("[channel] {}@{} → mode: Depth (auto-normalized, inverted)", channel_short, active_layer));
                            push_console(&ui, &console, format!("[preview] updated → mode: Depth (auto-normalized, inverted), {}::{}", active_layer, channel_short));
                        } else {
                            // Kanał → auto-normalizowany grayscale (percentyle)
                            let image = cache.process_depth_image_with_progress(false, Some(&prog));
                            ui.set_exr_image(image);
                            ui.set_status_text(format!("Layer: {} | Channel: {} | mode: Grayscale (auto-normalized)", active_layer, channel_short).into());
                            push_console(&ui, &console, format!("[channel] {}@{} → mode: Grayscale (auto-normalized)", channel_short, active_layer));
                            push_console(&ui, &console, format!("[preview] updated → mode: Grayscale (auto-normalized), {}::{}", active_layer, channel_short));
                        }
                        prog.finish(Some("Channel loaded"));
                        // Ustaw podświetlenie wybranego wiersza na liście
                        let display_layer = {
                            let map = lock_or_recover(&DISPLAY_TO_REAL_LAYER);
                            // Odwrotne mapowanie: znajdź klucz po wartości jeśli to możliwe
                            map.iter().find_map(|(k, v)| if v == &active_layer { Some(k.clone()) } else { None }).unwrap_or(active_layer.clone())
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
                        ui.set_status_text(format!("Error loading channel {}: {}", channel_short, e).into());
                        push_console(&ui, &console, format!("[error] loading channel {}@{}: {}", channel_short, active_layer, e));
                        prog.reset();
                    }
                }
            }
        }
    }
}

// Dodaj throttling timer dla smooth updates
pub struct ThrottledUpdate {
    _timer: Timer,
    pending_exposure: Arc<Mutex<Option<f32>>>,
    pending_gamma: Arc<Mutex<Option<f32>>>,
}

impl ThrottledUpdate {
    pub fn new<F>(mut callback: F) -> Self 
    where 
        F: FnMut(Option<f32>, Option<f32>) + 'static
    {
        let pending_exposure = Arc::new(Mutex::new(None));
        let pending_gamma = Arc::new(Mutex::new(None));
        
        let pending_exp_clone = pending_exposure.clone();
        let pending_gamma_clone = pending_gamma.clone();
        
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let exp = lock_or_recover(&pending_exp_clone).take();
            let gamma = lock_or_recover(&pending_gamma_clone).take();
            
            // Wywołaj callback nawet jeśli tylko jeden parametr się zmienił
            if exp.is_some() || gamma.is_some() {
                callback(exp, gamma);
            }
        });
        
        Self { _timer: timer, pending_exposure, pending_gamma }
    }
    
    pub fn update_exposure(&self, value: f32) {
        *lock_or_recover(&self.pending_exposure) = Some(value);
    }
    
    pub fn update_gamma(&self, value: f32) {
        *lock_or_recover(&self.pending_gamma) = Some(value);
    }
}

/// Obsługuje callback wyjścia z aplikacji
pub fn handle_exit(ui_handle: Weak<AppWindow>) {
    if let Some(ui) = ui_handle.upgrade() {
        let _ = ui.window().hide();
    }
}

/// Wspólna funkcja do wczytywania miniatur dla wskazanego katalogu i aktualizacji UI.
/// Używana zarówno przy starcie aplikacji (po argumencie pliku), jak i po wyborze folderu z UI.
pub fn load_thumbnails_for_directory(
    ui_handle: Weak<AppWindow>,
    directory: &Path,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        push_console(&ui, &console, format!("[folder] loading thumbnails: {}", directory.display()));
        ui.set_status_text(format!("Loading thumbnails: {}", directory.display()).into());
        let exposure = ui.get_exposure_value();
        let gamma = ui.get_gamma_value();
        let t0 = Instant::now();
        let prog = UiProgress::new(ui.as_weak());
        let tonemap_mode = ui.get_tonemap_mode() as i32;
        
        // Sprawdź czy GPU acceleration jest dostępne
        // TODO: Naprawić problem z lifetime'ami dla GPU context
        let gpu_context = None; // Na razie wyłączone
        
        match crate::thumbnails::generate_exr_thumbnails_in_dir_gpu(
            directory, 150, exposure, gamma, tonemap_mode, Some(&prog), gpu_context
        ) {
            Ok(mut thumbs) => {
                prog.set(0.95, Some("Sorting thumbnails..."));
                thumbs.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
                let items: Vec<ThumbItem> = thumbs
                    .into_iter()
                    .map(|t| ThumbItem {
                        img: t.image,
                        name: t.file_name.into(),
                        size: human_size(t.file_size_bytes).into(),
                        layers: format!("{} layers", t.num_layers).into(),
                        path: t.path.display().to_string().into(),
                        width: t.width as i32,
                        height: t.height as i32,
                    })
                    .collect();
                let count = items.len();
                ui.set_thumbnails(ModelRc::new(VecModel::from(items)));
                ui.set_bottom_panel_visible(true);
                let ms = t0.elapsed().as_millis();
                ui.set_status_text("Thumbnails loaded".into());
                prog.finish(Some("Thumbnails loaded"));
                push_console(&ui, &console, format!("[folder] {} EXR files | thumbnails in {} ms", count, ms));
            }
            Err(e) => {
                ui.set_status_text(format!("Error loading thumbnails: {}", e).into());
                push_console(&ui, &console, format!("[error][folder] {}", e));
                prog.reset();
            }
        }
    }
}

/// Obsługuje callback otwierania pliku EXR
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

/// Identyczna procedura jak w `handle_open_exr`, ale dla już znanej ścieżki
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

        // Zbuduj i wyświetl metadane w zakładce Meta
        match exr_metadata::read_and_group_metadata(&path) {
            Ok(meta) => {
                // Tekstowa wersja (zostawiona jako fallback)
                let lines = exr_metadata::build_ui_lines(&meta);
                let text = lines.join("\n");
                ui.set_meta_text(text.into());
                // Tabelaryczna wersja 2 kolumny
                let rows = exr_metadata::build_ui_rows(&meta);
                let (keys, vals): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
                ui.set_meta_table_keys(ModelRc::new(VecModel::from(keys.into_iter().map(SharedString::from).collect::<Vec<_>>())));
                ui.set_meta_table_values(ModelRc::new(VecModel::from(vals.into_iter().map(SharedString::from).collect::<Vec<_>>())));
                push_console(&ui, &console, format!("[meta] layers: {}", meta.layers.len()));
                prog.set(0.15, Some("Metadata loaded"));
            }
            Err(e) => {
                ui.set_meta_text(format!("Błąd odczytu metadanych: {}", e).into());
                push_console(&ui, &console, format!("[error][meta] {}", e));
                prog.reset();
            }
        }

        // Zapisz ścieżkę do pliku
        { *lock_or_recover(&current_file_path) = Some(path.clone()); }

        // Asynchroniczne wczytanie: wybór ścieżki FULL vs LIGHT
        let file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let force_light = std::env::var("EXRUSTER_LIGHT_OPEN").ok().as_deref() == Some("1");
        let use_light = force_light || file_size_bytes > 700 * 1024 * 1024; // >700MB ⇒ light

        prog.set(0.22, Some(if use_light { "Reading EXR (light)..." } else { "Reading EXR (full)..." }));
        ui.set_progress_value(-1.0);

        // Pobierz aktualne parametry przetwarzania
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
                // Odczytaj tylko najlepszą warstwę i zbuduj minimalny cache
                let light_res = (|| -> anyhow::Result<std::sync::Arc<FullExrCacheData>> {
                    let layers = crate::image_cache::extract_layers_info(&path_c)?;
                    let best = crate::image_cache::find_best_layer(&layers);
                    let lc = crate::image_cache::load_all_channels_for_layer(&path_c, &best, None)?;
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
                        let cache_res = crate::image_cache::ImageCache::new_with_full_cache(&path_c, full.clone());
                        match cache_res {
                            Ok(cache) => {
                                let _ = invoke_from_event_loop(move || {
                                    if let Some(ui2) = ui_weak.upgrade() {
                                        { let mut g = lock_or_recover(&full_exr_cache_c); *g = Some(full.clone()); }
                                        { let mut cg = lock_or_recover(&image_cache_c); *cg = Some(cache); }
                                        // Ustaw kontekst GPU w ImageCache
                                        set_gpu_context_in_cache_global(&image_cache_c);
                                        // Generuj obraz na wątku UI
                                        let img = {
                                            let guard = lock_or_recover(&image_cache_c);
                                            if let Some(ref c) = *guard { 
                                                process_image_with_gpu_fallback(c, exposure0, gamma0, tonemap_mode0)
                                            } else { 
                                                ui2.get_exr_image() 
                                            }
                                        };
                                        ui2.set_exr_image(img);

                                        // Zaktualizuj listę warstw
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
            // FULL ścieżka (dotychczasowa)
            rayon::spawn(move || {
                let t_start = Instant::now();
                let full_res = build_full_exr_cache(&path_c, None).map(std::sync::Arc::new);
                match full_res {
                    Ok(full) => {
                        let t_new = Instant::now();
                        let cache_res = crate::image_cache::ImageCache::new_with_full_cache(&path_c, full.clone());
                        match cache_res {
                            Ok(cache) => {
                                let _ = invoke_from_event_loop(move || {
                                    if let Some(ui2) = ui_weak.upgrade() {
                                        { let mut g = lock_or_recover(&full_exr_cache_c); *g = Some(full.clone()); }
                                        { let mut cg = lock_or_recover(&image_cache_c); *cg = Some(cache); }
                                        // Ustaw kontekst GPU w ImageCache
                                        set_gpu_context_in_cache_global(&image_cache_c);
                                        // Wygeneruj obraz na wątku UI (Image nie jest Send)
                                        let (img, layers_info_len, layers_info_vec) = {
                                            let guard = lock_or_recover(&image_cache_c);
                                            if let Some(ref c) = *guard {
                                                let li = c.layers_info.clone();
                                                (process_image_with_gpu_fallback(c, exposure0, gamma0, tonemap_mode0), li.len(), li)
                                            } else {
                                                (ui2.get_exr_image(), 0usize, Vec::new())
                                            }
                                        };
                                        ui2.set_exr_image(img);
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
}

// Ulepszona funkcja obsługi ekspozycji I gamma z throttling
pub fn handle_parameter_changed_throttled(
    ui_handle: Weak<AppWindow>,
    image_cache: ImageCacheType,
    console: ConsoleModel,
    exposure: Option<f32>,
    gamma: Option<f32>,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let cache_guard = lock_or_recover(&image_cache);
        if let Some(ref cache) = *cache_guard {
            // Pobierz aktualne wartości jeśli nie zostały przekazane
            let final_exposure = exposure.unwrap_or_else(|| ui.get_exposure_value());
            let final_gamma = gamma.unwrap_or_else(|| ui.get_gamma_value());
            
            // Użyj thumbnail dla real-time preview jeśli obraz jest duży, ale nie schodź poniżej 1:1 względem widżetu
            // Uwzględnij HiDPI i image-fit: contain (dopasowanie aspektu)
            let tonemap_mode = ui.get_tonemap_mode() as i32;
            let preview_w = ui.get_preview_area_width() as f32;
            let preview_h = ui.get_preview_area_height() as f32;
            let dpr = ui.window().scale_factor() as f32;
            let img_w = cache.width as f32;
            let img_h = cache.height as f32;
            let container_ratio = if preview_h > 0.0 { preview_w / preview_h } else { 1.0 };
            let image_ratio = if img_h > 0.0 { img_w / img_h } else { 1.0 };
            // Dłuzszy bok obrazu po dopasowaniu do kontenera (contain)
            let display_long_side_logical = if container_ratio > image_ratio { preview_h * image_ratio } else { preview_w };
            let target = (display_long_side_logical * dpr).round().max(1.0) as u32;
            let image = if cache.raw_pixels.len() > 2_000_000 {
                cache.process_to_thumbnail(final_exposure, final_gamma, tonemap_mode, target)
            } else {
                process_image_with_gpu_fallback(cache, final_exposure, final_gamma, tonemap_mode)
            };
            
            ui.set_exr_image(image);
            // Throttled log do konsoli: co najmniej 300 ms odstępu, z diagnostyką DPI i dopasowania
            let mut last = lock_or_recover(&LAST_PREVIEW_LOG);
            let now = Instant::now();
            if last.map(|t| now.duration_since(t).as_millis() >= 300).unwrap_or(true) {
                let display_w_logical = if container_ratio > image_ratio { preview_h * image_ratio } else { preview_w };
                let display_h_logical = if container_ratio > image_ratio { preview_h } else { preview_w / image_ratio };
                let win_w = ui.get_window_width() as u32;
                let win_h = ui.get_window_height() as u32;
                let win_w_px = (win_w as f32 * dpr).round() as u32;
                let win_h_px = (win_h as f32 * dpr).round() as u32;
                push_console(&ui, &console,
                    format!("[preview] params: exp={:.2}, gamma={:.2} | window={}x{} (≈{}x{} px @{}x) | view={}x{} @{}x | img={}x{} | display≈{}x{} px target={} px",
                        final_exposure, final_gamma,
                        win_w, win_h, win_w_px, win_h_px, dpr,
                        preview_w as u32, preview_h as u32, dpr,
                        img_w as u32, img_h as u32,
                        (display_w_logical * dpr).round() as u32,
                        (display_h_logical * dpr).round() as u32,
                        target));
                *last = Some(now);
            }
            
            // Aktualizuj status bar z informacją o zmienionym parametrze
            if exposure.is_some() && gamma.is_some() {
                ui.set_status_text(format!("🔄 Exposure: {:.2} EV, Gamma: {:.2}", final_exposure, final_gamma).into());
            } else if exposure.is_some() {
                ui.set_status_text(format!("🔄 Exposure: {:.2} EV", final_exposure).into());
            } else if gamma.is_some() {
                ui.set_status_text(format!("🔄 Gamma: {:.2}", final_gamma).into());
            }
        }
    }
}

// usunięto nieużywaną funkcję create_layers_model

pub fn create_layers_model(
    layers_info: &[crate::image_cache::LayerInfo],
    ui: &AppWindow,
) -> (ModelRc<slint::SharedString>, ModelRc<slint::Color>, ModelRc<i32>) {
    // UPROSZCZONE DRZEWO: Warstwa → faktyczne kanały (bez grup). RGBA tylko jeśli istnieją w pliku.
    let mut items: Vec<SharedString> = Vec::new();
    let mut colors: Vec<Color> = Vec::new();
    let mut font_sizes: Vec<i32> = Vec::new();
    // Wyczyść mapę
    lock_or_recover(&ITEM_TO_LAYER).clear();
    lock_or_recover(&DISPLAY_TO_REAL_LAYER).clear();
    for layer in layers_info {
        // Przyjazna nazwa dla pustej warstwy RGBA
        let display_name = if layer.name.is_empty() { "Beauty".to_string() } else { layer.name.clone() };
        // Zapisz mapowanie wyświetlanej nazwy na rzeczywistą
        {
            let mut map = lock_or_recover(&DISPLAY_TO_REAL_LAYER);
            map.insert(display_name.clone(), layer.name.clone());
        }
        // Wiersz nagłówka warstwy
        items.push(format!("📁 {}", display_name).into());
        colors.push(ui.get_layers_color_default());
        font_sizes.push(12);

        // Zbierz listę rzeczywistych kanałów (krótkie nazwy)
        let mut short_channels: Vec<String> = layer
            .channels
            .iter()
            .map(|c| c.name.split('.').last().unwrap_or(&c.name).to_string())
            .collect();

        // Zachowaj kolejność: R, G, B, A (jeśli są), potem reszta alfabetycznie
        // Uwzględnij synonimy: Red/Green/Blue/Alpha (case-insensitive)
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
            // Emoji dla RGBA, kropka dla pozostałych, oraz sufiks @<warstwa> dla jednoznaczności
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

// === Export Handlers ===

fn with_current_path(current_file_path: &CurrentFilePathType) -> Option<PathBuf> {
    let guard = super::ui_handlers::lock_or_recover(current_file_path);
    guard.clone()
}

pub fn handle_export_convert(
    ui_handle: Weak<AppWindow>,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console: ConsoleModel,
    full_exr_cache: FullExrCache,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let Some(_path) = with_current_path(&current_file_path) else {
            ui.set_status_text("Error: No file loaded".into());
            return;
        };
        if let Some(dst) = crate::file_operations::save_file_dialog(
            "Zapisz TIFF",
            "export.tiff",
            &[("TIFF", &["tif", "tiff"])],
        ) {
            push_console(&ui, &console, format!("[export] convert → {}", dst.display()));
            let prog = UiProgress::new(ui.as_weak());
            let guard = lock_or_recover(&image_cache);
            if let Some(ref cache) = *guard {
                // Zlicz liczbę stron do zapisania, aby móc raportować postęp
                let mut total_pages: usize = 0;
                for layer in &cache.layers_info {
                    let mut has_r = false;
                    let mut has_g = false;
                    let mut has_b = false;
                    let mut has_a = false;
                    for ch in &layer.channels {
                        let short = ch.name.split('.').last().unwrap_or(&ch.name).to_ascii_uppercase();
                        match short.as_str() {
                            "R" | "RED" => has_r = true,
                            "G" | "GREEN" => has_g = true,
                            "B" | "BLUE" => has_b = true,
                            "A" | "ALPHA" => has_a = true,
                            _ => {}
                        }
                    }
                    if has_r && has_g && has_b {
                        total_pages += 1; // RGB lub RGBA
                    } else if (has_r as u8 + has_g as u8 + has_b as u8) == 1 && !has_a {
                        total_pages += 1; // pojedynczy kanał R/G/B
                    } else if layer.channels.len() == 1 {
                        total_pages += 1; // dokładnie jeden kanał o innej nazwie
                    }
                }

                if total_pages == 0 {
                    ui.set_status_text("Export error: no pages to export".into());
                    return;
                }

                let mut completed_pages: usize = 0;
                prog.set(0.0, Some("Exporting TIFF..."));
                // Utwórz encoder TIFF
                let mut output_file = match File::create(&dst) {
                    Ok(f) => f,
                    Err(e) => {
                        ui.set_status_text(format!("Export error: {}", e).into());
                        prog.reset();
                        return;
                    }
                };
                let mut tiff_encoder = match TiffEncoder::new(&mut output_file) {
                    Ok(enc) => enc,
                    Err(e) => {
                        ui.set_status_text(format!("Export error (encoder): {}", e).into());
                        prog.reset();
                        return;
                    }
                };

                // Użyj globalnego cache pełnych danych EXR
                let full = {
                    let g = lock_or_recover(&full_exr_cache);
                    if let Some(ref img) = *g { img.clone() } else {
                        ui.set_status_text("Export error: EXR cache is empty".into());
                        prog.reset();
                        return;
                    }
                };

                let mut pages_written: u16 = 0;
                for layer in &cache.layers_info {
                    // Ustal dostępność kanałów w tej warstwie (po krótkich nazwach R/G/B/A)
                    let mut has_r = false;
                    let mut has_g = false;
                    let mut has_b = false;
                    let mut has_a = false;
                    for ch in &layer.channels {
                        let short = ch.name.split('.').last().unwrap_or(&ch.name).to_ascii_uppercase();
                        match short.as_str() {
                            "R" | "RED" => has_r = true,
                            "G" | "GREEN" => has_g = true,
                            "B" | "BLUE" => has_b = true,
                            "A" | "ALPHA" => has_a = true,
                            _ => {}
                        }
                    }

                    let display_name = if layer.name.is_empty() { "Beauty".to_string() } else { layer.name.clone() };

                    // Znajdź dopasowaną warstwę w pełnym cache
                    let wanted_lower = layer.name.to_lowercase();
                    let phys_layer = if let Some(pl) = crate::full_exr_cache::find_layer_by_name(&full, &wanted_lower) { pl } else {
                        push_console(&ui, &console, format!("[export] skip '{}' (layer not found)", display_name));
                        continue;
                    };
                    let width = phys_layer.width as u32;
                    let height = phys_layer.height as u32;
                    let pixel_count = (width as usize) * (height as usize);

                    // Zmapuj indeksy kanałów (planarny układ w FullLayer)
                    let mut r_idx: Option<usize> = None;
                    let mut g_idx: Option<usize> = None;
                    let mut b_idx: Option<usize> = None;
                    let mut a_idx: Option<usize> = None;
                    let group_indices: Vec<usize> = (0..phys_layer.channel_names.len()).collect();
                    for (idx, short) in phys_layer.channel_names.iter().enumerate() {
                        let su = short.to_ascii_uppercase();
                        match su.as_str() {
                            "R" | "RED" => r_idx = Some(idx),
                            "G" | "GREEN" => g_idx = Some(idx),
                            "B" | "BLUE" => b_idx = Some(idx),
                            "A" | "ALPHA" => a_idx = Some(idx),
                            _ => {
                                if r_idx.is_none() && su.starts_with('R') { r_idx = Some(idx); }
                                else if g_idx.is_none() && su.starts_with('G') { g_idx = Some(idx); }
                                else if b_idx.is_none() && su.starts_with('B') { b_idx = Some(idx); }
                            }
                        }
                    }

                    // Przypadek: pełny RGB(A)
                    if has_r && has_g && has_b {
                        if r_idx.is_none() { r_idx = group_indices.get(0).cloned(); }
                        if g_idx.is_none() { g_idx = group_indices.get(1).cloned().or(r_idx); }
                        if b_idx.is_none() { b_idx = group_indices.get(2).cloned().or(g_idx).or(r_idx); }

                        let (Some(ri), Some(gi), Some(bi)) = (r_idx, g_idx, b_idx) else {
                            push_console(&ui, &console, format!("[export] skip '{}' (missing RGB)", display_name));
                            continue;
                        };

                        if has_a {
                            let mut buf: Vec<f32> = vec![0.0; pixel_count * 4];
                            for i in 0..pixel_count {
                                let r = phys_layer.channel_data[ri * pixel_count + i];
                                let g = phys_layer.channel_data[gi * pixel_count + i];
                                let b = phys_layer.channel_data[bi * pixel_count + i];
                                let a = a_idx.map(|ci| phys_layer.channel_data[ci * pixel_count + i]).unwrap_or(1.0);
                                let base = i * 4;
                                buf[base + 0] = r;
                                buf[base + 1] = g;
                                buf[base + 2] = b;
                                buf[base + 3] = a;
                            }
                            if let Err(e) = write_tiff_page_rgba_f32(&mut tiff_encoder, width, height, &display_name, &buf) {
                                ui.set_status_text(format!("Export error (TIFF RGBA): {}", e).into());
                                prog.reset();
                                return;
                            }
                        } else {
                            let mut buf: Vec<f32> = vec![0.0; pixel_count * 3];
                            for i in 0..pixel_count {
                                let r = phys_layer.channel_data[ri * pixel_count + i];
                                let g = phys_layer.channel_data[gi * pixel_count + i];
                                let b = phys_layer.channel_data[bi * pixel_count + i];
                                let base = i * 3;
                                buf[base + 0] = r;
                                buf[base + 1] = g;
                                buf[base + 2] = b;
                            }
                            if let Err(e) = write_tiff_page_rgb_f32(&mut tiff_encoder, width, height, &display_name, &buf) {
                                ui.set_status_text(format!("Export error (TIFF RGB): {}", e).into());
                                prog.reset();
                                return;
                            }
                        }
                        pages_written += 1;
                        push_console(&ui, &console, format!("[export] page: {} ({}x{}, {})", display_name, width, height, if has_a { "RGBAf32" } else { "RGBf32" }));
                        completed_pages += 1;
                        prog.set(
                            (completed_pages as f32) / (total_pages as f32),
                            Some(&format!("Exporting TIFF ({}/{}) — {}", completed_pages, total_pages, display_name))
                        );
                    }
                    // Przypadek: pojedynczy kanał RGB (np. tylko R) → Gray f32
                    else if (has_r as u8 + has_g as u8 + has_b as u8) == 1 && !has_a {
                        let wanted = if has_r { "R" } else if has_g { "G" } else { "B" };
                        // znajdź indeks kanału
                        let mut ci: Option<usize> = None;
                        for (idx, short) in phys_layer.channel_names.iter().enumerate() {
                            if short.eq_ignore_ascii_case(wanted) || short.to_ascii_uppercase().starts_with(&wanted.to_ascii_uppercase()) {
                                ci = Some(idx);
                                break;
                            }
                        }
                        let Some(ci) = ci else {
                            push_console(&ui, &console, format!("[export] skip '{}' (channel not found)", display_name));
                            continue;
                        };
                        let mut buf: Vec<f32> = vec![0.0; pixel_count];
                        for i in 0..pixel_count { buf[i] = phys_layer.channel_data[ci * pixel_count + i]; }
                        if let Err(e) = write_tiff_page_gray_f32(&mut tiff_encoder, width, height, &display_name, &buf) {
                            ui.set_status_text(format!("Export error (TIFF Gray): {}", e).into());
                            prog.reset();
                            return;
                        }
                        pages_written += 1;
                        push_console(&ui, &console, format!("[export] page: {} ({}x{}, Grayf32)", display_name, width, height));
                        completed_pages += 1;
                        prog.set(
                            (completed_pages as f32) / (total_pages as f32),
                            Some(&format!("Exporting TIFF ({}/{}) — {}", completed_pages, total_pages, display_name))
                        );
                    }
                    // Przypadek: dokładnie jeden kanał o innej nazwie (np. Z/Depth/Mask) → Gray f32
                    else if layer.channels.len() == 1 {
                        let ch_name = &layer.channels[0].name;
                        // znajdź indeks kanału po krótkiej nazwie
                        let mut ci: Option<usize> = None;
                        for (idx, short) in phys_layer.channel_names.iter().enumerate() {
                            if short.as_str() == ch_name.as_str() {
                                ci = Some(idx);
                                break;
                            }
                        }
                        let Some(ci) = ci else {
                            push_console(&ui, &console, format!("[export] skip '{}' (channel not found)", display_name));
                            continue;
                        };
                        let mut buf: Vec<f32> = vec![0.0; pixel_count];
                        for i in 0..pixel_count { buf[i] = phys_layer.channel_data[ci * pixel_count + i]; }
                        if let Err(e) = write_tiff_page_gray_f32(&mut tiff_encoder, width, height, &display_name, &buf) {
                            ui.set_status_text(format!("Export error (TIFF Gray): {}", e).into());
                            prog.reset();
                            return;
                        }
                        pages_written += 1;
                        push_console(&ui, &console, format!("[export] page: {} ({}x{}, Grayf32)", display_name, width, height));
                        completed_pages += 1;
                        prog.set(
                            (completed_pages as f32) / (total_pages as f32),
                            Some(&format!("Exporting TIFF ({}/{}) — {}", completed_pages, total_pages, display_name))
                        );
                    }
                    // Inne/niestandardowe układy kanałów – pomiń
                    else {
                        push_console(&ui, &console, format!("[export] skip '{}' (unsupported channel layout)", display_name));
                        continue;
                    }
                }

                if pages_written == 0 {
                    ui.set_status_text("Export error: no pages written".into());
                    prog.reset();
                    return;
                }

                ui.set_status_text(format!("Exported TIFF ({} pages)", pages_written).into());
                prog.finish(Some("TIFF saved"));
            }
        }
    }
}

// Pomocnicze funkcje zapisu strony TIFF z tagiem nazwy warstwy (tiff 0.9 API)
fn write_tiff_page_rgba_f32(
    encoder: &mut TiffEncoder<&mut File>,
    width: u32,
    height: u32,
    layer_name: &str,
    data: &[f32],
) -> anyhow::Result<()> {
    let mut image_writer = encoder
        .new_image::<RGBA32Float>(width, height)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    // Tag opisu warstwy
    image_writer.encoder().write_tag(Tag::ImageDescription, layer_name).map_err(|e| anyhow::anyhow!("{}", e))?;
    image_writer
        .write_data(data)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

fn write_tiff_page_rgb_f32(
    encoder: &mut TiffEncoder<&mut File>,
    width: u32,
    height: u32,
    layer_name: &str,
    data: &[f32],
) -> anyhow::Result<()> {
    let mut image_writer = encoder
        .new_image::<RGB32Float>(width, height)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    image_writer.encoder().write_tag(Tag::ImageDescription, layer_name).map_err(|e| anyhow::anyhow!("{}", e))?;
    image_writer
        .write_data(data)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

fn write_tiff_page_gray_f32(
    encoder: &mut TiffEncoder<&mut File>,
    width: u32,
    height: u32,
    layer_name: &str,
    data: &[f32],
) -> anyhow::Result<()> {
    let mut image_writer = encoder
        .new_image::<Gray32Float>(width, height)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    image_writer.encoder().write_tag(Tag::ImageDescription, layer_name).map_err(|e| anyhow::anyhow!("{}", e))?;
    image_writer
        .write_data(data)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}

pub fn handle_export_beauty(
    ui_handle: Weak<AppWindow>,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let Some(path) = with_current_path(&current_file_path) else {
            ui.set_status_text("Error: No file loaded".into());
            return;
        };
        let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
        let suggested = format!("{}_beauty.png", file_stem);
        if let Some(dst) = crate::file_operations::save_file_dialog(
            "Zapisz PNG (Beauty)",
            &suggested,
            &[("PNG", &["png"])],
        ) {
            push_console(&ui, &console, format!("[export] beauty → {}", dst.display()));
            let prog = UiProgress::new(ui.as_weak());
            prog.start_indeterminate(Some("Exporting Beauty PNG..."));
            let guard = lock_or_recover(&image_cache);
            if let Some(ref cache) = *guard {
                let width = cache.width;
                let height = cache.height;
                // Zastosuj current exposure/tone-map/gamma i sRGB, zapis do 16-bit PNG
                let exposure = ui.get_exposure_value();
                let gamma = ui.get_gamma_value();
                let tonemap_mode: i32 = ui.get_tonemap_mode() as i32;
                let mut buf = ImageBuffer::<Rgb<u16>, Vec<u16>>::new(width, height);
                for (x, y, p) in buf.enumerate_pixels_mut() {
                    let idx = (y as usize) * (width as usize) + (x as usize);
                    if let Some(&(mut r, mut g, mut b, _a)) = cache.raw_pixels.get(idx) {
                        if let Some(mat) = cache.color_matrix() {
                            let v = mat * Vec3::new(r, g, b);
                            r = v.x; g = v.y; b = v.z;
                        }
                        let (r_out, g_out, b_out) = tone_map_and_gamma(r, g, b, exposure, gamma, tonemap_mode);
                        let r16 = (r_out * 65535.0).round().clamp(0.0, 65535.0) as u16;
                        let g16 = (g_out * 65535.0).round().clamp(0.0, 65535.0) as u16;
                        let b16 = (b_out * 65535.0).round().clamp(0.0, 65535.0) as u16;
                        *p = image::Rgb([r16, g16, b16]);
                    }
                }
                if let Err(e) = buf.save_with_format(&dst, image::ImageFormat::Png) {
                    ui.set_status_text(format!("Export error: {}", e).into());
                    prog.reset();
                    return;
                }
                ui.set_status_text("Exported Beauty PNG".into());
                prog.finish(Some("PNG saved"));
            }
        }
    }
}

pub fn handle_export_channels(
    ui_handle: Weak<AppWindow>,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console: ConsoleModel,
    full_exr_cache: FullExrCache,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let Some(path) = with_current_path(&current_file_path) else {
            ui.set_status_text("Error: No file loaded".into());
            return;
        };
        if let Some(dst_dir) = crate::file_operations::choose_export_directory() {
            push_console(&ui, &console, format!("[export] channels → {}", dst_dir.display()));
            let prog = UiProgress::new(ui.as_weak());
            let mut exported = 0usize;
            {
                let guard = lock_or_recover(&image_cache);
                if let Some(ref cache) = *guard {
                    // Pobierz pełny cache danych
                    let full = {
                        let g = lock_or_recover(&full_exr_cache);
                        if let Some(ref img) = *g { img.clone() } else {
                            ui.set_status_text("Export error: EXR cache is empty".into());
                            prog.reset();
                            return;
                        }
                    };

                    // Zlicz łączną liczbę kanałów do przetworzenia
                    let total_channels: usize = cache.layers_info.iter().map(|l| l.channels.len()).sum();
                    if total_channels == 0 {
                        ui.set_status_text("Export error: no channels to export".into());
                        return;
                    }
                    prog.set(0.0, Some("Exporting channels..."));

                    for layer in &cache.layers_info {
                        let display_layer = if layer.name.is_empty() { "Beauty".to_string() } else { layer.name.clone() };
                        // Znajdź odpowiadającą warstwę w pełnym cache
                        let wanted_lower = layer.name.to_lowercase();
                        let phys_layer = if let Some(pl) = crate::full_exr_cache::find_layer_by_name(&full, &wanted_lower) { pl } else {
                            push_console(&ui, &console, format!("[export] skip layer '{}' (layer not found)", display_layer));
                            continue;
                        };
                        let width = phys_layer.width as u32;
                        let height = phys_layer.height as u32;
                        let pixel_count = (width as usize) * (height as usize);

                    // Kanały dostępne w postaci planarnych danych

                    for ch in &layer.channels {
                            let ch_upper = ch.name.to_ascii_uppercase();
                        // znajdź indeks kanału po krótkiej nazwie
                        let mut channel_index: Option<usize> = None;
                        for (idx, short) in phys_layer.channel_names.iter().enumerate() {
                            let su = short.to_ascii_uppercase();
                            if su == ch_upper || su.starts_with(&ch_upper) { channel_index = Some(idx); break; }
                            if ch_upper == "Z" && (su == "Z" || su.contains("DEPTH") || su == "DISTANCE") { channel_index = Some(idx); break; }
                        }
                            let Some(ci) = channel_index else {
                                push_console(&ui, &console, format!("[export] skip channel '{}::{}' (not found)", display_layer, ch.name));
                                continue;
                            };

                            // Zbierz wartości kanału
                        let mut values: Vec<f32> = Vec::with_capacity(pixel_count);
                        for i in 0..pixel_count { values.push(phys_layer.channel_data[ci * pixel_count + i]); }

                            // Renderuj do 16-bit grayscale
                            let mut buf = ImageBuffer::<image::Luma<u16>, Vec<u16>>::new(width, height);
                            if !values.is_empty() {
                                let use_depth = ch_upper == "Z" || ch_upper.contains("DEPTH") || ch_upper == "DISTANCE";
                                if use_depth {
                                    use std::cmp::Ordering;
                                    let len = values.len();
                                    let p_lo_idx = ((len as f32) * 0.01).floor() as usize;
                                    let mut p_hi_idx = ((len as f32) * 0.99).ceil() as isize - 1;
                                    if p_hi_idx < 0 { p_hi_idx = 0; }
                                    let p_hi_idx = (p_hi_idx as usize).min(len - 1);
                                    let lo = {
                                        let (_, lo_ref, _) = values.select_nth_unstable_by(p_lo_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                                        *lo_ref
                                    };
                                    let mut hi = {
                                        let (_, hi_ref, _) = values.select_nth_unstable_by(p_hi_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                                        *hi_ref
                                    };
                                    let lo = lo;
                                    if hi <= lo { hi = lo + 1e-6; }
                                    for (x, y, p) in buf.enumerate_pixels_mut() {
                                        let idx = (y as usize) * (width as usize) + (x as usize);
                                        let v = values[idx];
                                        let mut n = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
                                        n = 1.0 - n; // inverted (near bright)
                                        let v16 = (n * 65535.0).round().clamp(0.0, 65535.0) as u16;
                                        *p = image::Luma([v16]);
                                    }
                                } else {
                                    let _exposure = ui.get_exposure_value();
                                    let _gamma = ui.get_gamma_value();
                                    let exp_mul = 2.0_f32.powf(_exposure);
                                    let inv_gamma = if _gamma > 0.0 { 1.0 / _gamma } else { 1.0 / 2.2 };
                                    for (x, y, p) in buf.enumerate_pixels_mut() {
                                        let idx = (y as usize) * (width as usize) + (x as usize);
                                        let mut v = values[idx] * exp_mul;
                                        v = v.clamp(0.0, 1.0).powf(inv_gamma);
                                        let v16 = (v * 65535.0).round().clamp(0.0, 65535.0) as u16;
                                        *p = image::Luma([v16]);
                                    }
                                }
                            }

                            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("export");
                            let filename = format!("{}_{}_{}.png", file_stem, display_layer, ch.name);
                            let out_path = dst_dir.join(filename);
                            if let Err(e) = buf.save_with_format(&out_path, image::ImageFormat::Png) {
                                ui.set_status_text(format!("Export error: {}", e).into());
                                prog.reset();
                                return;
                            }
                            exported += 1;
                            prog.set(
                                (exported as f32) / (total_channels as f32),
                                Some(&format!("Exporting channels ({}/{}) — {}::{}", exported, total_channels, display_layer, ch.name))
                            );
                        }
                    }
                }
            }
            ui.set_status_text(format!("Exported {} channel images", exported).into());
            prog.finish(Some("Channels saved"));
        }
    }
}

/// Aktualizuje status GPU w interfejsie użytkownika
pub fn update_gpu_status(ui: &AppWindow, gpu_context: &GpuContextType) {
    if let Ok(guard) = gpu_context.lock() {
        if let Some(ref context) = *guard {
            let adapter_info = context.get_adapter_info();
            let status_text = format!("GPU: {} - dostępny", adapter_info.name);
            ui.set_gpu_status_text(status_text.into());
        } else {
            ui.set_gpu_status_text("GPU: niedostępny (tryb CPU)".into());
        }
    } else {
        ui.set_gpu_status_text("GPU: błąd dostępu".into());
    }
}

/// Sprawdza czy GPU jest dostępne i aktualizuje status
pub fn check_gpu_availability(ui: &AppWindow, gpu_context: &GpuContextType) -> bool {
    if let Ok(guard) = gpu_context.lock() {
        if let Some(ref context) = *guard {
            if context.is_available() {
                let adapter_info = context.get_adapter_info();
                ui.set_gpu_status_text(format!("GPU: {} - aktywny", adapter_info.name).into());
                return true;
            } else {
                ui.set_gpu_status_text("GPU: błąd urządzenia".into());
                return false;
            }
        } else {
            ui.set_gpu_status_text("GPU: niedostępny (tryb CPU)".into());
            return false;
        }
    } else {
        ui.set_gpu_status_text("GPU: błąd dostępu".into());
        return false;
    }
}

/// Ustawia globalny kontekst GPU
pub fn set_global_gpu_context(gpu_context: Arc<Mutex<Option<crate::gpu_context::GpuContext>>>) {
    if let Ok(mut guard) = GPU_CONTEXT.lock() {
        *guard = Some(gpu_context);
    }
}

/// Ustawia globalny stan akceleracji GPU
pub fn set_global_gpu_acceleration(enabled: bool) {
    if let Ok(mut guard) = GPU_ACCELERATION_ENABLED.lock() {
        *guard = enabled;
    }
}

/// Pobiera globalny stan akceleracji GPU
pub fn get_global_gpu_acceleration() -> bool {
    if let Ok(guard) = GPU_ACCELERATION_ENABLED.lock() {
        *guard
    } else {
        false
    }
}

/// Ustawia kontekst GPU w ImageCache po jego utworzeniu
pub fn set_gpu_context_in_cache(image_cache: &ImageCacheType, gpu_context: &GpuContextType) {
    if let Ok(mut guard) = image_cache.lock() {
        if let Some(ref mut cache) = *guard {
            cache.set_gpu_context(gpu_context.clone());
        }
    }
}

/// Ustawia kontekst GPU w ImageCache używając globalnego kontekstu
pub fn set_gpu_context_in_cache_global(image_cache: &ImageCacheType) {
    if let Ok(guard) = GPU_CONTEXT.lock() {
        if let Some(ref gpu_ctx) = *guard {
            set_gpu_context_in_cache(image_cache, gpu_ctx);
        }
    }
}

/// Przetwarza obraz używając GPU lub CPU w zależności od ustawień
pub fn process_image_with_gpu_fallback(
    cache: &ImageCache,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> slint::Image {
    let gpu_enabled = get_global_gpu_acceleration();
    
    if gpu_enabled {
        // Spróbuj użyć GPU z lepszą obsługą błędów
        match cache.process_to_image_gpu(exposure, gamma, tonemap_mode) {
            Ok(image) => {
                println!("Obraz przetworzony pomyślnie na GPU");
                image
            },
            Err(e) => {
                // Fallback do CPU w przypadku błędu GPU
                eprintln!("GPU processing failed: {}, falling back to CPU", e);
                println!("Przełączam na tryb CPU...");
                
                // Automatycznie wyłącz GPU acceleration po błędzie
                set_global_gpu_acceleration(false);
                
                // Użyj CPU jako fallback
                cache.process_to_image(exposure, gamma, tonemap_mode)
            }
        }
    } else {
        // Użyj CPU
        cache.process_to_image(exposure, gamma, tonemap_mode)
    }
}

