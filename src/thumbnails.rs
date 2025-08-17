use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use rayon::prelude::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, Duration};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::image_cache::extract_layers_info;
use crate::progress::ProgressSink;
use std::sync::{Mutex, OnceLock};
use lru::LruCache;

// Dodaj importy dla nowego systemu
use exr::prelude as exr;
use image;

/// Statistics for timing operations
struct TimingStats {
    total_load_time: AtomicU64,    // Total time for loading/creating thumbnails (in nanoseconds)
    total_save_time: AtomicU64,    // Total time for saving thumbnails (in nanoseconds)
}

impl TimingStats {
    fn new() -> Self {
        Self {
            total_load_time: AtomicU64::new(0),
            total_save_time: AtomicU64::new(0),
        }
    }

    fn add_load_time(&self, duration: Duration) {
        self.total_load_time.fetch_add(duration.as_nanos() as u64, AtomicOrdering::SeqCst);
    }



    fn get_load_time(&self) -> Duration {
        Duration::from_nanos(self.total_load_time.load(AtomicOrdering::SeqCst))
    }

    fn get_save_time(&self) -> Duration {
        Duration::from_nanos(self.total_save_time.load(AtomicOrdering::SeqCst))
    }

    fn get_total_time(&self) -> Duration {
        self.get_load_time() + self.get_save_time()
    }
}

/// Color processing configuration
struct ColorConfig {
    gamma: f32,
    exposure: f32,
    tonemap_mode: i32,
}

impl ColorConfig {
    fn new(gamma: f32, exposure: f32, tonemap_mode: i32) -> Self {
        Self {
            gamma,
            exposure,
            tonemap_mode,
        }
    }
}





/// Generuje miniaturki używając CPU (nowa, wydajna implementacja) - zwraca ExrThumbWork
pub fn generate_thumbnails_cpu_raw(
    files: Vec<PathBuf>,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<Vec<ExrThumbWork>> {
    let total_files = files.len();
    let timing_stats = TimingStats::new();
    let color_config = ColorConfig::new(
        gamma,
        exposure,
        tonemap_mode
    );

    // 1) Równolegle generuj dane miniaturek w typie bezpiecznym dla wątków
    let completed = AtomicUsize::new(0);
    let works: Vec<ExrThumbWork> = files
        .into_par_iter()
        .filter_map(|path| {
            // Spróbuj z cache LRU
            let cached_opt = {
                if let Ok(mut guard) = get_thumb_cache().lock() {
                    c_get(&mut *guard, &path, thumb_height, exposure, gamma, tonemap_mode)
                } else {
                    None
                }
            };
            if let Some(cached) = cached_opt {
                let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(p) = progress {
                    let frac = (n as f32) / (total_files as f32);
                    p.set(frac, Some(&format!("Cached: {}/{} {}", n, total_files, path.file_name().and_then(|n| n.to_str()).unwrap_or("?"))));
                }
                return Some(cached);
            }

            let res = generate_single_exr_thumbnail_work_new(&path, thumb_height, &color_config, &timing_stats)
                .map(|work| {
                    // Zapisz do cache
                    put_thumb_cache(&work, thumb_height, exposure, gamma, tonemap_mode);
                    work
                });
            let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(p) = progress {
                let frac = (n as f32) / (total_files as f32);
                p.set(frac, Some(&format!("Processed: {}/{} {}", n, total_files, path.file_name().and_then(|n| n.to_str()).unwrap_or("?"))));
            }
            match res {
                Ok(work) => Some(work),
                Err(_e) => None,
            }
        })
        .collect();

    if let Some(p) = progress { 
        p.finish(Some(&format!("Thumbnails loaded: {} files processed", works.len()))); 
    }
    
    let load_time = timing_stats.get_load_time();
    let save_time = timing_stats.get_save_time();
    let processing_time = timing_stats.get_total_time();
    println!("Thumbnail generation timing: Load: {:.2}ms, Save: {:.2}ms, Total: {:.2}ms", 
             load_time.as_millis(), save_time.as_millis(), processing_time.as_millis());
    
    Ok(works)
}



pub fn list_exr_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("Nie można odczytać katalogu: {}", dir.display()))?;

    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("exr") {
                    out.push(path);
                }
            }
        }
    }
    Ok(out)
}

#[derive(Clone)]
pub struct ExrThumbWork {
    pub path: PathBuf,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub num_layers: usize,
    pub pixels: Vec<u8>, // RGBA8 interleaved
}

/// NOWA, WYDAJNA FUNKCJA generowania miniaturki używająca nowoczesnego API exr
fn generate_single_exr_thumbnail_work_new(
    exr_path: &Path,
    thumb_height: u32,
    color_config: &ColorConfig,
    timing_stats: &TimingStats,
) -> anyhow::Result<ExrThumbWork> {
    let load_start = Instant::now();
    
    // Szybkie pobranie metadanych
    let layers_info = extract_layers_info(&exr_path.to_path_buf())
        .with_context(|| format!("Błąd odczytu meta EXR: {}", exr_path.display()))?;
    
    // Skopiuj wartości do closure aby uniknąć problemów z lifetime
    let exposure = color_config.exposure;
    let tonemap_mode = color_config.tonemap_mode;
    let gamma = color_config.gamma;
    
    // Użyj nowoczesnego API exr do wczytania danych
    let reader = exr::read_first_rgba_layer_from_file(
        exr_path,
        // Generuj bufor pikseli
        |resolution, _| exr::pixel_vec::PixelVec {
            resolution,
            pixels: vec![image::Rgba([0u8; 4]); resolution.width() * resolution.height()],
        },
        // Przetwarzaj piksele z nowoczesnym przetwarzaniem kolorów
        move |pixel_vec, position, (r, g, b, a): (f32, f32, f32, f32)| {
            let index = position.y() * pixel_vec.resolution.width() + position.x();
            
            // Zastosuj ekspozycję
            let exposure_mult = 2.0_f32.powf(exposure);
            let (r, g, b) = (r * exposure_mult, g * exposure_mult, b * exposure_mult);
            
            // Tone mapping
            let (r, g, b) = match tonemap_mode {
                0 => { // ACES
                    let aces_tonemap = |x: f32| {
                        let a = 2.51;
                        let b = 0.03;
                        let c = 2.43;
                        let d = 0.59;
                        let e = 0.14;
                        (x * (a * x + b) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
                    };
                    (aces_tonemap(r), aces_tonemap(g), aces_tonemap(b))
                },
                1 => { // Reinhard
                    let reinhard_tonemap = |x: f32| x / (1.0 + x);
                    (reinhard_tonemap(r), reinhard_tonemap(g), reinhard_tonemap(b))
                },
                2 => { // Linear
                    (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
                },
                _ => (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
            };

            // Gamma correction
            let gamma_correct = |x: f32| x.powf(1.0 / gamma);
            
            let processed = [
                (gamma_correct(r) * 255.0) as u8,
                (gamma_correct(g) * 255.0) as u8,
                (gamma_correct(b) * 255.0) as u8,
                (a.clamp(0.0, 1.0) * 255.0) as u8,
            ];
            
            pixel_vec.pixels[index] = image::Rgba(processed);
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to read EXR: {}", e))?;

    // Pobierz dane obrazu
    let image_data = reader.layer_data.channel_data.pixels;
    let (width, height) = (
        image_data.resolution.width() as u32,
        image_data.resolution.height() as u32,
    );

    // Oblicz wymiary miniaturki
    let thumb_width = (width as f32 / height as f32 * thumb_height as f32) as u32;

    // Stwórz obraz z pikseli
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        width,
        height,
        image_data.pixels.into_iter().flat_map(|rgba| rgba.0).collect::<Vec<u8>>(),
    )
    .ok_or_else(|| anyhow::anyhow!("Could not create image buffer"))?;

    // Resize używając szybszego filtra
    let thumbnail = image::imageops::resize(&img, thumb_width, thumb_height, image::imageops::FilterType::Triangle);

    let load_duration = load_start.elapsed();
    timing_stats.add_load_time(load_duration);

    // Konwertuj do formatu RGBA8
    let pixels = thumbnail.into_raw();
    
    let file_name = exr_path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    let file_size_bytes = fs::metadata(exr_path).map(|m| m.len()).unwrap_or(0);

    Ok(ExrThumbWork {
        path: exr_path.to_path_buf(),
        file_name,
        file_size_bytes,
        width: thumb_width,
        height: thumb_height,
        num_layers: layers_info.len(),
        pixels,
    })
}

// ================= LRU cache miniaturek =================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ThumbPresetKey {
    thumb_h: u32,
    tonemap_mode: i32,
    // Kwantyzujemy ekspozycję i gammę, by nie tworzyć nadmiaru wariantów
    exp_q: i16,
    gam_q: i16,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    path: PathBuf,
    modified: u64,
    preset: ThumbPresetKey,
}

// Przechowujemy wyłącznie gotowe piksele RGBA8 i podstawowe metadane
#[derive(Clone)]
pub struct ThumbValue {
    width: u32,
    height: u32,
    num_layers: usize,
    file_size_bytes: u64,
    file_name: String,
    pixels: Vec<u8>,
}

fn quantize(v: f32, step: f32, min: f32, max: f32) -> i16 {
    let clamped = v.clamp(min, max);
    ((clamped / step).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn make_preset(thumb_h: u32, exposure: f32, gamma: f32, tonemap_mode: i32) -> ThumbPresetKey {
    ThumbPresetKey {
        thumb_h,
        tonemap_mode,
        exp_q: quantize(exposure, 0.25, -16.0, 16.0),
        gam_q: quantize(gamma, 0.10, 0.5, 4.5),
    }
}

fn file_mtime_u64(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

static THUMB_CACHE: OnceLock<Mutex<LruCache<ThumbKey, ThumbValue>>> = OnceLock::new();

pub fn get_thumb_cache() -> &'static Mutex<LruCache<ThumbKey, ThumbValue>> {
    THUMB_CACHE.get_or_init(|| Mutex::new(LruCache::new(std::num::NonZeroUsize::new(256).unwrap())))
}

/// Czyści cache miniaturek (force regeneration)
pub fn clear_thumb_cache() {
    if let Ok(mut cache) = get_thumb_cache().lock() {
        cache.clear();
        println!("Thumbnail cache cleared - forcing regeneration");
    }
}

pub fn c_get(
    cache: &mut LruCache<ThumbKey, ThumbValue>,
    path: &Path,
    thumb_h: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> Option<ExrThumbWork> {
    let preset = make_preset(thumb_h, exposure, gamma, tonemap_mode);
    let key = ThumbKey { path: path.to_path_buf(), modified: file_mtime_u64(path), preset };
    cache.get(&key).map(|v| ExrThumbWork {
        path: key.path.clone(),
        file_name: v.file_name.clone(),
        file_size_bytes: v.file_size_bytes,
        width: v.width,
        height: v.height,
        num_layers: v.num_layers,
        pixels: v.pixels.clone(),
    })
}

pub fn put_thumb_cache(work: &ExrThumbWork, thumb_h: u32, exposure: f32,
                       gamma: f32, tonemap_mode: i32) {
    if let Ok(mut cache) = get_thumb_cache().lock() {
        let preset = make_preset(thumb_h, exposure, gamma, tonemap_mode);
        let key = ThumbKey { 
            path: work.path.clone(), 
            modified: file_mtime_u64(&work.path), 
            preset 
        };
        let value = ThumbValue {
            width: work.width,
            height: work.height,
            num_layers: work.num_layers,
            file_size_bytes: work.file_size_bytes,
            file_name: work.file_name.clone(),
            pixels: work.pixels.clone(),
        };
        cache.put(key, value);
    }
}