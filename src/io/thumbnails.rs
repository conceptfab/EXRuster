use anyhow::Context;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::log_info;
use crate::processing::tone_mapping::ToneMapMode;
use crate::ui::progress::ProgressSink;
use lru::LruCache;
use std::sync::{Arc, Mutex};
use std::sync::OnceLock;

// Dodaj importy dla nowego systemu
use exr::prelude as exr;
use image;

/// Statistics for timing operations
pub struct TimingStats {
    total_load_time: AtomicU64, // Total time for loading/creating thumbnails (in nanoseconds)
    total_save_time: AtomicU64, // Total time for saving thumbnails (in nanoseconds)
}

impl Clone for TimingStats {
    fn clone(&self) -> Self {
        Self {
            total_load_time: AtomicU64::new(self.total_load_time.load(AtomicOrdering::SeqCst)),
            total_save_time: AtomicU64::new(self.total_save_time.load(AtomicOrdering::SeqCst)),
        }
    }
}

impl TimingStats {
    fn new() -> Self {
        Self {
            total_load_time: AtomicU64::new(0),
            total_save_time: AtomicU64::new(0),
        }
    }

    fn add_load_time(&self, duration: Duration) {
        self.total_load_time
            .fetch_add(duration.as_nanos() as u64, AtomicOrdering::SeqCst);
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
#[derive(Clone)]
pub struct ColorConfig {
    gamma: f32,
    exposure: f32,
    tonemap_mode: ToneMapMode,
}

impl ColorConfig {
    fn new(gamma: f32, exposure: f32, tonemap_mode: i32) -> Self {
        Self {
            gamma,
            exposure,
            tonemap_mode: ToneMapMode::from(tonemap_mode),
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
    let color_config = ColorConfig::new(gamma, exposure, tonemap_mode);

    // A message forces an un-throttled event-loop dispatch (UiProgress::set treats
    // Some(msg) as "always deliver"). One per file would queue hundreds of closures
    // on a big folder, so only every Nth file carries text; the rest just move the bar.
    const PROGRESS_MESSAGE_EVERY: usize = 16;

    let report = |p: &dyn ProgressSink, done: usize, path: &Path, verb: &str| {
        let frac = (done as f32) / (total_files as f32);
        if done % PROGRESS_MESSAGE_EVERY == 0 || done == total_files {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            p.set(
                frac,
                Some(&format!("{}: {}/{} {}", verb, done, total_files, name)),
            );
        } else {
            p.set(frac, None);
        }
    };

    // 1) Równolegle generuj dane miniaturek w typie bezpiecznym dla wątków
    let completed = AtomicUsize::new(0);
    let works: Vec<ExrThumbWork> = files
        .into_par_iter()
        .filter_map(|path| {
            // Spróbuj z cache LRU
            let cached_opt = c_get(&path, thumb_height, exposure, gamma, tonemap_mode);
            if let Some(cached) = cached_opt {
                let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(p) = progress {
                    report(p, n, &path, "Cached");
                }
                return Some(cached);
            }

            let is_hdr = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.eq_ignore_ascii_case("hdr"))
                .unwrap_or(false);
            let res = (if is_hdr {
                generate_single_hdr_thumbnail_work(&path, thumb_height, &color_config, &timing_stats)
            } else {
                generate_single_exr_thumbnail_work_new(
                    &path,
                    thumb_height,
                    &color_config,
                    &timing_stats,
                )
            })
            .inspect(|work| {
                // Zapisz do cache
                put_thumb_cache(work, thumb_height, exposure, gamma, tonemap_mode);
            });
            let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(p) = progress {
                report(p, n, &path, "Processed");
            }
            res.ok()
        })
        .collect();

    if let Some(p) = progress {
        p.finish(Some(&format!(
            "Thumbnails loaded: {} files processed",
            works.len()
        )));
    }

    let load_time = timing_stats.get_load_time();
    let save_time = timing_stats.get_save_time();
    let processing_time = timing_stats.get_total_time();
    log_info!(
        "Thumbnail generation timing: Load: {:.2}ms, Save: {:.2}ms, Total: {:.2}ms",
        load_time.as_millis(),
        save_time.as_millis(),
        processing_time.as_millis()
    );

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
                if ext.eq_ignore_ascii_case("exr") || ext.eq_ignore_ascii_case("hdr") {
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
    pub pixels: Arc<[u8]>, // RGBA8 interleaved
}

/// Tone-map interleaved linear RGBA f32 to RGBA8.
///
/// Uses the shared pipeline (which reaches for the sRGB LUT at gamma ~2.2)
/// rather than raw `powf` per channel.
fn tonemap_thumb_rgba(rgba: &[f32], color_config: &ColorConfig) -> Arc<[u8]> {
    use crate::processing::tone_mapping::{tone_map_and_gamma, ToneMapParams};

    let params = ToneMapParams::new(
        color_config.exposure,
        color_config.gamma,
        color_config.tonemap_mode as i32,
    );

    let mut out = Vec::with_capacity(rgba.len());
    for chunk in rgba.chunks_exact(4) {
        let (r, g, b) = tone_map_and_gamma(
            chunk[0],
            chunk[1],
            chunk[2],
            params.exposure_multiplier,
            params.gamma_inv,
            params.use_srgb,
            params.mode,
        );
        out.push((r.clamp(0.0, 1.0) * 255.0) as u8);
        out.push((g.clamp(0.0, 1.0) * 255.0) as u8);
        out.push((b.clamp(0.0, 1.0) * 255.0) as u8);
        out.push((chunk[3].clamp(0.0, 1.0) * 255.0) as u8);
    }
    Arc::from(out.into_boxed_slice())
}

/// Generate one EXR thumbnail.
///
/// Reads linear f32, downscales, and only then tone-maps: the expensive
/// per-pixel math runs on ~390px of output rather than on every pixel of a
/// full-resolution frame (8.3M of them for 4K).
pub fn generate_single_exr_thumbnail_work_new(
    exr_path: &Path,
    thumb_height: u32,
    color_config: &ColorConfig,
    timing_stats: &TimingStats,
) -> anyhow::Result<ExrThumbWork> {
    let load_start = Instant::now();

    let reader = exr::read_first_rgba_layer_from_file(
        exr_path,
        |resolution, _| exr::pixel_vec::PixelVec {
            resolution,
            pixels: vec![[0.0f32; 4]; resolution.width() * resolution.height()],
        },
        |pixel_vec, position, (r, g, b, a): (f32, f32, f32, f32)| {
            let index = position.y() * pixel_vec.resolution.width() + position.x();
            pixel_vec.pixels[index] = [r, g, b, a];
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to read EXR: {}", e))?;

    // Count layers from file metadata headers (fast header-only read, no pixel data)
    let num_layers = ::exr::meta::MetaData::read_from_file(exr_path, false)
        .map(|meta| meta.headers.len())
        .unwrap_or(1);

    let image_data = reader.layer_data.channel_data.pixels;
    let (width, height) = (
        image_data.resolution.width() as u32,
        image_data.resolution.height() as u32,
    );
    let thumb_width = (width as f32 / height as f32 * thumb_height as f32) as u32;

    let raw: Vec<f32> = image_data.pixels.into_iter().flatten().collect();
    let img = image::ImageBuffer::<image::Rgba<f32>, _>::from_raw(width, height, raw)
        .ok_or_else(|| anyhow::anyhow!("Could not create image buffer"))?;

    let small = image::imageops::resize(
        &img,
        thumb_width,
        thumb_height,
        image::imageops::FilterType::Triangle,
    );
    let pixels = tonemap_thumb_rgba(small.as_raw(), color_config);

    timing_stats.add_load_time(load_start.elapsed());

    let raw_name = exr_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string());
    let file_name = crate::utils::normalize_display_name(&raw_name);
    let file_size_bytes = fs::metadata(exr_path).map(|m| m.len()).unwrap_or(0);

    Ok(ExrThumbWork {
        path: exr_path.to_path_buf(),
        file_name,
        file_size_bytes,
        width: thumb_width,
        height: thumb_height,
        num_layers,
        pixels,
    })
}

/// Generates a thumbnail for a Radiance HDR (.hdr) file.
pub fn generate_single_hdr_thumbnail_work(
    hdr_path: &Path,
    thumb_height: u32,
    color_config: &ColorConfig,
    timing_stats: &TimingStats,
) -> anyhow::Result<ExrThumbWork> {
    let load_start = Instant::now();

    let hdr = crate::io::hdr_loader::load_hdr(hdr_path)?;
    let width = hdr.width;
    let height = hdr.height;

    // Same order as the EXR path: downscale the linear data, then tone-map.
    let img = image::ImageBuffer::<image::Rgba<f32>, _>::from_raw(width, height, hdr.rgba)
        .ok_or_else(|| anyhow::anyhow!("Could not create HDR image buffer"))?;
    let thumb_width = (width as f32 / height as f32 * thumb_height as f32) as u32;
    let small = image::imageops::resize(
        &img,
        thumb_width,
        thumb_height,
        image::imageops::FilterType::Triangle,
    );
    let pixels = tonemap_thumb_rgba(small.as_raw(), color_config);

    timing_stats.add_load_time(load_start.elapsed());

    let raw_name = hdr_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string());
    let file_name = crate::utils::normalize_display_name(&raw_name);
    let file_size_bytes = fs::metadata(hdr_path).map(|m| m.len()).unwrap_or(0);

    Ok(ExrThumbWork {
        path: hdr_path.to_path_buf(),
        file_name,
        file_size_bytes,
        width: thumb_width,
        height: thumb_height,
        num_layers: 1,
        pixels,
    })
}

// ================= LRU cache miniaturek =================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ThumbPresetKey {
    thumb_h: u32,
    tonemap_mode: ToneMapMode,
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
    pixels: Arc<[u8]>,
}

fn quantize(v: f32, step: f32, min: f32, max: f32) -> i16 {
    let clamped = v.clamp(min, max);
    ((clamped / step).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn make_preset(thumb_h: u32, exposure: f32, gamma: f32, tonemap_mode: i32) -> ThumbPresetKey {
    ThumbPresetKey {
        thumb_h,
        tonemap_mode: ToneMapMode::from(tonemap_mode),
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

pub fn c_get(
    path: &Path,
    thumb_h: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> Option<ExrThumbWork> {
    let preset = make_preset(thumb_h, exposure, gamma, tonemap_mode);
    let key = ThumbKey {
        path: path.to_path_buf(),
        modified: file_mtime_u64(path),
        preset,
    };
    if let Ok(mut cache) = get_thumb_cache().lock() {
        cache.get(&key).map(|v| ExrThumbWork {
            path: key.path.clone(),
            file_name: v.file_name.clone(),
            file_size_bytes: v.file_size_bytes,
            width: v.width,
            height: v.height,
            num_layers: v.num_layers,
            pixels: v.pixels.clone(),
        })
    } else {
        None
    }
}

pub fn put_thumb_cache(
    work: &ExrThumbWork,
    thumb_h: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) {
    let preset = make_preset(thumb_h, exposure, gamma, tonemap_mode);
    let key = ThumbKey {
        path: work.path.clone(),
        modified: file_mtime_u64(&work.path),
        preset,
    };
    let value = ThumbValue {
        width: work.width,
        height: work.height,
        num_layers: work.num_layers,
        file_size_bytes: work.file_size_bytes,
        file_name: work.file_name.clone(),
        pixels: work.pixels.clone(),
    };
    if let Ok(mut cache) = get_thumb_cache().lock() {
        cache.put(key, value);
    }
}
