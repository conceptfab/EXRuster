use slint::{Weak, ComponentHandle, Timer, TimerMode, ModelRc, VecModel, SharedString, Color};
use slint::invoke_from_event_loop;
use std::sync::{Arc, Mutex, MutexGuard};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use crate::io::image_cache::ImageCache;
use crate::io::file_operations::{open_file_dialog, get_file_name};
use std::rc::Rc;
use crate::io::exr_metadata;
// Removed unused import: bytemuck
use crate::ui::progress::{ProgressSink, UiProgress};
use crate::utils::{get_channel_info, human_size};
use anyhow::{Result, Context};
// Usunięte nieużywane importy związane z exportem
// Import komponentów Slint
use crate::AppWindow;
use crate::ThumbItem;

/// Centralny stan aplikacji - zastępuje globalne static variables
/// TODO: Implement full migration from global statics to dependency injection
#[allow(dead_code)]
pub struct AppState {
    pub item_to_layer: HashMap<String, String>,
    pub display_to_real_layer: HashMap<String, String>,
    pub current_file_path: Option<PathBuf>,
    pub last_preview_log: Option<Instant>,
}

#[allow(dead_code)]
impl AppState {
    pub fn new() -> Self {
        Self {
            item_to_layer: HashMap::new(),
            display_to_real_layer: HashMap::new(),
            current_file_path: None,
            last_preview_log: None,
        }
    }
    
    /// Synchronizuje stan z globalnymi zmiennymi (przejściowe rozwiązanie)
    pub fn sync_with_globals(&mut self) {
        // W przyszłości: migracja krok po kroku z globalnych na dependency injection
        // TODO: Implement proper sync with current_file_path global
    }
}

pub type ImageCacheType = Arc<Mutex<Option<ImageCache>>>;
pub type CurrentFilePathType = Arc<Mutex<Option<PathBuf>>>;
pub type ConsoleModel = Rc<VecModel<SharedString>>;
use crate::io::full_exr_cache::{FullExrCacheData, FullLayer, build_full_exr_cache};
pub type FullExrCache = Arc<Mutex<Option<std::sync::Arc<FullExrCacheData>>>>;

// Stała wysokości miniaturek - zmień tutaj aby dostosować rozdzielczość
const THUMBNAIL_HEIGHT: u32 = 130;

/// Dodaje linię do modelu konsoli i aktualizuje tekst w `TextEdit` (console-text)
pub fn push_console(ui: &crate::AppWindow, console: &ConsoleModel, line: String) {
    console.push(line.clone().into());
    let mut joined = ui.get_console_text().to_string();
    if !joined.is_empty() { joined.push('\n'); }
    joined.push_str(&line);
    ui.set_console_text(joined.into());
}

static LAST_PREVIEW_LOG: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);



/// Standardowy wzorzec dla bezpiecznego dostępu do Mutex z kontekstem błędu
/// TODO: Replace lock_or_recover usage with this function for better error handling
#[allow(dead_code)]
#[inline]
pub(crate) fn safe_lock<'a, T>(mutex: &'a Arc<Mutex<T>>, context: &'static str) -> Result<MutexGuard<'a, T>> {
    mutex.lock()
        .map_err(|_| anyhow::anyhow!("Mutex poisoned: {}", context))
}

/// Kompatybilność wsteczna - używa panic recovery
#[inline]
pub fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
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


// Removed old handle_layer_tree_click - now in layers.rs

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
/// Teraz działa asynchronicznie w osobnym wątku, aby nie blokować UI.
pub fn load_thumbnails_for_directory(
    ui_handle: Weak<AppWindow>,
    directory: &Path,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        push_console(&ui, &console, format!("[folder] loading thumbnails: {}", directory.display()));
        ui.set_status_text(format!("Loading thumbnails: {}", directory.display()).into());
        

        let prog = UiProgress::new(ui.as_weak());
        prog.start_indeterminate(Some("🔍 Scanning folder for EXR files..."));
        
        // Wyczyść cache aby wymusić regenerację miniaturek z nowymi parametrami
        crate::io::thumbnails::clear_thumb_cache();
        
        // Użyj stałych, zoptymalizowanych wartości dla miniaturek (nie z UI!)
        let exposure = 0.0;     // Neutralna ekspozycja dla miniaturek
        let gamma = 2.2;        // Standardowa gamma dla miniaturek  
        let tonemap_mode = 0;   // ACES tone mapping dla miniaturek
        
    
        
        // Generuj miniaturki w głównym wątku (bezpieczniejsze i z cache)
        let ui_weak = ui.as_weak();
        let directory_path = directory.to_path_buf();
        
        // Generuj miniaturki w tle z cache - używamy prostszego podejścia
        std::thread::spawn(move || {
            let t0 = Instant::now();
            
            // Generuj miniaturki w osobnym wątku używając istniejącej funkcji z cache
            let files = match crate::io::thumbnails::list_exr_files(&directory_path) {
                Ok(files) => files,
                Err(e) => {
                    let ui_weak_clone = ui_weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.set_status_text(format!("❌ Error scanning folder: {}", e).into());
                            prog.reset();
                        }
                    }).unwrap();
                    return;
                }
            };
            
            // Sprawdź czy folder nie jest pusty
            let total_files = files.len();
            if total_files == 0 {
                let ui_weak_clone = ui_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak_clone.upgrade() {
                        ui.set_status_text("⚠️ No EXR files found in selected folder".into());
                        prog.finish(Some("⚠️ No EXR files found"));
                    }
                }).unwrap();
                return;
            }
            
            // Użyj nowej, wydajnej funkcji do generowania miniaturek
            prog.set(0.1, Some(&format!("📁 Found {} EXR files, starting processing...", total_files)));
            
            // Generuj miniaturki w osobnym wątku - użyj GPU jeśli dostępny
            let thumbnail_works = match crate::io::thumbnails::generate_thumbnails_gpu_raw(
                files,
                THUMBNAIL_HEIGHT, 
                exposure, 
                gamma, 
                tonemap_mode,
                Some(&prog)
            ) {
                Ok(works) => works,
                Err(e) => {
                    eprintln!("Failed to generate thumbnails: {}", e);
                    let ui_weak_clone = ui_weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.set_status_text(format!("❌ Failed to generate thumbnails: {}", e).into());
                            prog.finish(Some("❌ Thumbnail generation failed"));
                        }
                    }).unwrap();
                    return;
                }
            };
            
            prog.set(0.9, Some("📊 Sorting thumbnails alphabetically..."));
            let mut sorted_works = thumbnail_works;
            sorted_works.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
            
            let count = sorted_works.len();
            let ms = t0.elapsed().as_millis();
            
            // Aktualizuj UI w głównym wątku przez invoke_from_event_loop
            let ui_weak_clone = ui_weak.clone();
            let count_clone = count;
            let ms_clone = ms;
            
            slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_clone.upgrade() {
                    prog.set(0.95, Some("🎨 Converting thumbnails to UI format..."));
                    
                    // Konwertuj miniaturki do formatu UI w głównym wątku
                    let items: Vec<ThumbItem> = sorted_works
                        .into_iter()
                        .map(|w| {
                            // Konwertuj surowe piksele RGBA8 do slint::Image
                            let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w.width, w.height);
                            let slice = buffer.make_mut_slice();
                            
                            // Skopiuj piksele RGBA8
                            for (dst, chunk) in slice.iter_mut().zip(w.pixels.chunks_exact(4)) {
                                *dst = slint::Rgba8Pixel { 
                                    r: chunk[0], 
                                    g: chunk[1], 
                                    b: chunk[2], 
                                    a: chunk[3] 
                                };
                            }
                            
                            let image = slint::Image::from_rgba8(buffer);
                            
                            ThumbItem {
                                img: image,
                                name: w.file_name.into(),
                                size: human_size(w.file_size_bytes).into(),
                                layers: format!("{} layers", w.num_layers).into(),
                                path: w.path.display().to_string().into(),
                                width: w.width as i32,
                                height: w.height as i32,
                            }
                        })
                        .collect();
                    
                    // Aktualizuj UI
                    ui.set_thumbnails(ModelRc::new(VecModel::from(items)));
                    ui.set_bottom_panel_visible(true);
                    
                    ui.set_status_text("Thumbnails loaded".into());
                    prog.finish(Some(&format!("✅ Successfully loaded {} thumbnails in {} ms", count_clone, ms_clone)));
                    
                    // Log do konsoli w osobnym wywołaniu - używamy prostego status text
                    ui.set_status_text(format!("[folder] {} EXR files | thumbnails in {} ms", count_clone, ms_clone).into());
                }
            }).unwrap();
        });
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

        // Ładuje metadane pliku EXR i aktualizuje UI
        match load_metadata(&ui, &path, &console) {
            Ok(()) => {
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
                                let cache_res = crate::io::image_cache::ImageCache::new_with_full_cache(&path_c, full.clone());
                                match cache_res {
                                    Ok(cache) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                { let mut g = lock_or_recover(&full_exr_cache_c); *g = Some(full.clone()); }
                                                { let mut cg = lock_or_recover(&image_cache_c); *cg = Some(cache); }
                                                // Generuj obraz na wątku UI
                                                let img = {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut c) = *guard { 
                                                        c.process_to_image(exposure0, gamma0, tonemap_mode0)
                                                    } else { 
                                                        ui2.get_exr_image() 
                                                    }
                                                };
                                                ui2.set_exr_image(img);

                                                // Automatycznie oblicz histogram dla nowego obrazu
                                                {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut cache) = *guard {
                                                        if let Ok(()) = cache.update_histogram() {
                                                            if let Some(hist_data) = cache.get_histogram_data() {
                                                                // Przekaż dane histogramu do UI
                                                                let red_bins: Vec<i32> = hist_data.red_bins.iter().map(|&x| x as i32).collect();
                                                                let green_bins: Vec<i32> = hist_data.green_bins.iter().map(|&x| x as i32).collect();
                                                                let blue_bins: Vec<i32> = hist_data.blue_bins.iter().map(|&x| x as i32).collect();
                                                                let lum_bins: Vec<i32> = hist_data.luminance_bins.iter().map(|&x| x as i32).collect();
                                                                
                                                                ui2.set_histogram_red_data(slint::ModelRc::new(slint::VecModel::from(red_bins)));
                                                                ui2.set_histogram_green_data(slint::ModelRc::new(slint::VecModel::from(green_bins)));
                                                                ui2.set_histogram_blue_data(slint::ModelRc::new(slint::VecModel::from(blue_bins)));
                                                                ui2.set_histogram_luminance_data(slint::ModelRc::new(slint::VecModel::from(lum_bins)));
                                                                
                                                                // Statystyki
                                                                ui2.set_histogram_min_value(hist_data.min_value);
                                                                ui2.set_histogram_max_value(hist_data.max_value);
                                                                ui2.set_histogram_total_pixels(hist_data.total_pixels as i32);
                                                                
                                                                // Percentyle
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
                                let cache_res = crate::io::image_cache::ImageCache::new_with_full_cache(&path_c, full.clone());
                                match cache_res {
                                    Ok(cache) => {
                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                { let mut g = lock_or_recover(&full_exr_cache_c); *g = Some(full.clone()); }
                                                { let mut cg = lock_or_recover(&image_cache_c); *cg = Some(cache); }
                                                // Wygeneruj obraz na wątku UI (Image nie jest Send)
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

                                                // Automatycznie oblicz histogram dla nowego obrazu
                                                {
                                                    let mut guard = lock_or_recover(&image_cache_c);
                                                    if let Some(ref mut cache) = *guard {
                                                        if let Ok(()) = cache.update_histogram() {
                                                            if let Some(hist_data) = cache.get_histogram_data() {
                                                                // Przekaż dane histogramu do UI
                                                                let red_bins: Vec<i32> = hist_data.red_bins.iter().map(|&x| x as i32).collect();
                                                                let green_bins: Vec<i32> = hist_data.green_bins.iter().map(|&x| x as i32).collect();
                                                                let blue_bins: Vec<i32> = hist_data.blue_bins.iter().map(|&x| x as i32).collect();
                                                                let lum_bins: Vec<i32> = hist_data.luminance_bins.iter().map(|&x| x as i32).collect();
                                                                
                                                                ui2.set_histogram_red_data(slint::ModelRc::new(slint::VecModel::from(red_bins)));
                                                                ui2.set_histogram_blue_data(slint::ModelRc::new(slint::VecModel::from(blue_bins)));
                                                                ui2.set_histogram_green_data(slint::ModelRc::new(slint::VecModel::from(green_bins)));
                                                                ui2.set_histogram_luminance_data(slint::ModelRc::new(slint::VecModel::from(lum_bins)));
                                                                
                                                                // Statystyki
                                                                ui2.set_histogram_min_value(hist_data.min_value);
                                                                ui2.set_histogram_max_value(hist_data.max_value);
                                                                ui2.set_histogram_total_pixels(hist_data.total_pixels as i32);
                                                                
                                                                // Percentyle
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

/// Ładuje metadane pliku EXR i aktualizuje UI
fn load_metadata(
    ui: &AppWindow,
    path: &Path,
    console: &ConsoleModel,
) -> Result<(), anyhow::Error> {
    // Zbuduj i wyświetl metadane w zakładce Meta z lepszą obsługą błędów
    let meta = exr_metadata::read_and_group_metadata(path)
        .with_context(|| format!("Failed to read EXR metadata from: {}", path.display()))?;
    
    // Tekstowa wersja (zostawiona jako fallback)
    let lines = exr_metadata::build_ui_lines(&meta);
    let text = lines.join("\n");
    ui.set_meta_text(text.into());
    
    // Tabelaryczna wersja 2 kolumny
    let rows = exr_metadata::build_ui_rows(&meta);
    let (keys, vals): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    ui.set_meta_table_keys(ModelRc::new(VecModel::from(keys.into_iter().map(SharedString::from).collect::<Vec<_>>())));
    ui.set_meta_table_values(ModelRc::new(VecModel::from(vals.into_iter().map(SharedString::from).collect::<Vec<_>>())));
    push_console(ui, console, format!("[meta] layers: {}", meta.layers.len()));
    
    Ok(())
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
            
            let tonemap_mode = ui.get_tonemap_mode() as i32;
            let image = update_preview_image(&ui, cache, final_exposure, final_gamma, tonemap_mode, &console);
            
            ui.set_exr_image(image);
            
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

/// Aktualizuje podgląd obrazu na podstawie aktualnych parametrów UI
pub fn update_preview_image(
    ui: &AppWindow,
    cache: &ImageCache,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    console: &ConsoleModel,
) -> slint::Image {
    // Użyj thumbnail dla real-time preview jeśli obraz jest duży, ale nie schodź poniżej 1:1 względem widżetu
    // Uwzględnij HiDPI i image-fit: contain (dopasowanie aspektu)
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
        cache.process_to_thumbnail(exposure, gamma, tonemap_mode, target)
    } else {
        cache.process_to_image(exposure, gamma, tonemap_mode)
    };
    
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
        push_console(ui, console,
            format!("[preview] params: exp={:.2}, gamma={:.2} | window={}x{} (≈{}x{} px @{}x) | view={}x{} @{}x | img={}x{} | display≈{}x{} px target={} px",
                exposure, gamma,
                win_w, win_h, win_w_px, win_h_px, dpr,
                preview_w as u32, preview_h as u32, dpr,
                img_w as u32, img_h as u32,
                (display_w_logical * dpr).round() as u32,
                (display_h_logical * dpr).round() as u32,
                target));
        *last = Some(now);
    }
    
    image
}

pub fn create_layers_model(
    layers_info: &[crate::io::image_cache::LayerInfo],
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
// Wszystkie funkcje exportu zostały usunięte

// Funkcja handle_export_convert została usunięta

// Wszystkie funkcje pomocnicze TIFF zostały usunięte

// Funkcja handle_export_beauty została usunięta

// Funkcja handle_export_channels została usunięta








