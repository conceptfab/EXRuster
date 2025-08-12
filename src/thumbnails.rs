use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use rayon::prelude::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::cell::RefCell;
use std::rc::Rc;
use exr::prelude as exr;

use crate::image_processing::process_pixel;
use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;
use crate::image_cache::{extract_layers_info, find_best_layer, load_specific_layer};
use crate::progress::ProgressSink;
use crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer;
use glam::Vec3;
use std::sync::{Mutex, OnceLock};
use lru::LruCache;

/// Zwięzła reprezentacja miniaturki EXR do wyświetlenia w UI
pub struct ExrThumbnailInfo {
    pub path: PathBuf,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub num_layers: usize,
    pub width: u32,  // rzeczywista szerokość miniaturki po skalowaniu
    pub height: u32, // rzeczywista wysokość miniaturki (zawsze thumb_height)
    pub image: Image,
}

/// Główny interfejs: generuje miniaturki dla wszystkich plików .exr w katalogu (bez rekursji).
/// - Przetwarzanie odbywa się równolegle (Rayon)
/// - Miniaturki powstają z kompozytu kanałów R, G, B z "najlepszej" warstwy (wybór scentralizowany w `image_cache`)
/// - Transformacje zgodne z podglądem (ACES + gamma) przez `process_pixel`, z przekazanymi parametrami
pub fn generate_exr_thumbnails_in_dir(
    directory: &Path,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<Vec<ExrThumbnailInfo>> {
    let files = list_exr_files(directory)?;
    let total_files = files.len();
    if let Some(p) = progress { p.set(0.0, Some(&format!("Processing {} files...", total_files))); }

    if total_files == 0 {
        if let Some(p) = progress { p.finish(Some("No EXR files")); }
        return Ok(Vec::new());
    }

    // 1) Równolegle generuj dane miniaturek w typie bezpiecznym dla wątków (bez slint::Image)
    let completed = AtomicUsize::new(0);
    let works: Vec<ExrThumbWork> = files
        .par_iter()
        .filter_map(|path| {
            // Spróbuj z cache LRU
            let cached_opt = {
                if let Ok(mut guard) = get_thumb_cache().lock() {
                    c_get(&mut *guard, path, thumb_height, exposure, gamma, tonemap_mode)
                } else {
                    None
                }
            };
            if let Some(cached) = cached_opt {
                let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(p) = progress {
                    let frac = (n as f32) / (total_files as f32);
                    p.set(frac, Some(&format!("{} / {}", n, total_files)));
                }
                return Some(cached);
            }

            let res = generate_single_exr_thumbnail_work(path, thumb_height, exposure, gamma, tonemap_mode)
                .map(|work| {
                    // Zapisz do cache
                    put_thumb_cache(&work, thumb_height, exposure, gamma, tonemap_mode);
                    work
                });
            let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(p) = progress {
                let frac = (n as f32) / (total_files as f32);
                p.set(frac, Some(&format!("{} / {}", n, total_files)));
            }
            match res {
                Ok(work) => Some(work),
                Err(_e) => None, // tu można logować błąd
            }
        })
        .collect();

    // 2) Na głównym wątku skonstruuj slint::Image (nie jest Send)
    let thumbnails: Vec<ExrThumbnailInfo> = works
        .into_iter()
        .map(|w| {
            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w.width, w.height);
            let slice = buffer.make_mut_slice();
            // skopiuj surowe RGBA8 do bufora Slint
            for (dst, chunk) in slice.iter_mut().zip(w.pixels.chunks_exact(4)) {
                *dst = Rgba8Pixel { r: chunk[0], g: chunk[1], b: chunk[2], a: chunk[3] };
            }

            ExrThumbnailInfo {
                path: w.path,
                file_name: w.file_name,
                file_size_bytes: w.file_size_bytes,
                num_layers: w.num_layers,
                width: w.width,
                height: w.height,
                image: Image::from_rgba8(buffer),
            }
        })
        .collect();

    if let Some(p) = progress { p.finish(Some("Thumbnails ready")); }
    Ok(thumbnails)
}

fn list_exr_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
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

struct ExrThumbWork {
    path: PathBuf,
    file_name: String,
    file_size_bytes: u64,
    width: u32,
    height: u32,
    num_layers: usize,
    pixels: Vec<u8>, // RGBA8 interleaved
}

fn generate_single_exr_thumbnail_work(
    path: &Path,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> anyhow::Result<ExrThumbWork> {
    use std::convert::Infallible;
    use ::exr::math::Vec2;

    let path_buf = path.to_path_buf();

    // Tylko meta: policz warstwy do prezentacji, bez wczytywania pikseli
    let layers_info = extract_layers_info(&path_buf)
        .with_context(|| format!("Błąd odczytu EXR: {}", path.display()))?;

    // Macierz primaries → sRGB z metadanych (globalnie / warstwa pusta)
    let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(&path_buf.as_path(), "").ok();

    // Współdzielony stan dla callbacków czytnika
    let dims = Rc::new(RefCell::new((0u32, 0u32, 0u32, 0u32))); // (w, h, tw, th)
    let strides = Rc::new(RefCell::new((1.0f32, 1.0f32))); // (sx, sy)
    let out_pixels = Rc::new(RefCell::new(Vec::<u8>::new()));

    let dims_c = dims.clone();
    let strides_c = strides.clone();
    let out_c1 = out_pixels.clone();

    // 1) Inicjalizacja po rozdzielczości
    let stream_result = exr::read_first_rgba_layer_from_file(
        &path_buf,
        move |resolution, _| -> Result<(), Infallible> {
            let width = resolution.width() as u32;
            let height = resolution.height() as u32;
            let thumb_h = thumb_height.max(1);
            let thumb_w = ((width as f32) * (thumb_h as f32) / (height as f32)).max(1.0).round() as u32;

            *dims_c.borrow_mut() = (width, height, thumb_w, thumb_h);
            let sx = (width as f32) / (thumb_w as f32);
            let sy = (height as f32) / (thumb_h as f32);
            *strides_c.borrow_mut() = (sx, sy);

            out_c1.borrow_mut().resize((thumb_w as usize) * (thumb_h as usize) * 4, 0u8);
            Ok(())
        },
        {
            let m = color_matrix_rgb_to_srgb;
            let out_c2 = out_pixels.clone();
            let dims_r = dims.clone();
            let strides_r = strides.clone();
            move |_, position: Vec2<usize>, (r0, g0, b0, a0): (f32, f32, f32, f32)| {
                let (_width, _height, thumb_w, thumb_h) = *dims_r.borrow();
                if thumb_w == 0 || thumb_h == 0 {
                    return;
                }
                let (sx, sy) = *strides_r.borrow();

                // Użyj poprawnych współrzędnych z biblioteki `exr`
                let src_x = position.x() as u32;
                let src_y = position.y() as u32;

                // Mapowanie do piksela docelowego (NN)
                let x_out = ((src_x as f32) / sx).floor() as u32;
                let y_out = ((src_y as f32) / sy).floor() as u32;
                if x_out >= thumb_w || y_out >= thumb_h {
                    return;
                }

                // Transformacja kolorów (opcjonalna) + tone-mapping
                let (mut r, mut g, mut b, a) = (r0, g0, b0, a0);
                if let Some(mat) = m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                let px = process_pixel(r, g, b, a, exposure, gamma, tonemap_mode);

                let out_index = ((y_out as usize) * (thumb_w as usize) + (x_out as usize)) * 4;
                {
                    let mut out_ref = out_c2.borrow_mut();
                    if out_index + 3 < out_ref.len() {
                        out_ref[out_index + 0] = px.r;
                        out_ref[out_index + 1] = px.g;
                        out_ref[out_index + 2] = px.b;
                        out_ref[out_index + 3] = 255;
                    }
                }
            }
        }
    );

    // Jeśli strumień się nie powiódł, fallback do heurystyki warstw
    if stream_result.is_err() {
        // Fallback: wczytaj warstwę w pełnej rozdzielczości, potem przeskaluj (stabilne API)
        let best_layer_name = find_best_layer(&layers_info);
        let (raw_pixels, width, height, _current_layer) = load_specific_layer(&path_buf, &best_layer_name, None)
            .with_context(|| format!("Błąd wczytania warstwy \"{}\": {}", best_layer_name, path.display()))?;

        let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(&path_buf.as_path(), &best_layer_name).ok();

        let scale = thumb_height as f32 / height as f32;
        let thumb_h = thumb_height.max(1);
        let thumb_w = ((width as f32) * scale).max(1.0).round() as u32;

        let mut pixels: Vec<u8> = vec![0; (thumb_w as usize) * (thumb_h as usize) * 4];
        let raw_width = width as usize;
        let m = color_matrix_rgb_to_srgb;
        let total = (thumb_w as usize) * (thumb_h as usize);
        let par_pixels: Vec<u8> = (0..(total / 4)).into_par_iter().flat_map(|block| {
            let base = block * 4;
            let mut rr = [0.0f32; 4];
            let mut gg = [0.0f32; 4];
            let mut bb = [0.0f32; 4];
            let mut aa = [1.0f32; 4];
            for lane in 0..4 {
                let i = base + lane;
                let x = (i as u32) % thumb_w;
                let y = (i as u32) / thumb_w;
                let src_x = ((x as f32 / scale) as u32).min(width.saturating_sub(1));
                let src_y = ((y as f32 / scale) as u32).min(height.saturating_sub(1));
                let src_idx = (src_y as usize) * raw_width + (src_x as usize);
                let (mut r, mut g, mut b, a) = raw_pixels[src_idx];
                if let Some(mat) = m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                rr[lane] = r; gg[lane] = g; bb[lane] = b; aa[lane] = a;
            }
            let (r8, g8, b8) = crate::image_processing::tone_map_and_gamma_simd(
                f32x4::from_array(rr), f32x4::from_array(gg), f32x4::from_array(bb), exposure, gamma, tonemap_mode);
            let a8 = f32x4::from_array(aa).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
            let ra: [f32; 4] = r8.into();
            let ga: [f32; 4] = g8.into();
            let ba: [f32; 4] = b8.into();
            let aa: [f32; 4] = a8.into();
            
            let mut chunk = Vec::with_capacity(16);
            for lane in 0..4 {
                chunk.push((ra[lane] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((ga[lane] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((ba[lane] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((aa[lane] * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            chunk
        }).collect();

        pixels[..par_pixels.len()].copy_from_slice(&par_pixels);

        for i in (total / 4 * 4)..total {
            let x = (i as u32) % thumb_w;
            let y = (i as u32) / thumb_w;
            let src_x = ((x as f32 / scale) as u32).min(width.saturating_sub(1));
            let src_y = ((y as f32 / scale) as u32).min(height.saturating_sub(1));
            let src_idx = (src_y as usize) * raw_width + (src_x as usize);
            let (mut r, mut g, mut b, a) = raw_pixels[src_idx];
            if let Some(mat) = m {
                let v = mat * Vec3::new(r, g, b);
                r = v.x; g = v.y; b = v.z;
            }
            let px = process_pixel(r, g, b, a, exposure, gamma, tonemap_mode);
            let out_index = i * 4;
            pixels[out_index + 0] = px.r;
            pixels[out_index + 1] = px.g;
            pixels[out_index + 2] = px.b;
            pixels[out_index + 3] = px.a;
        }

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let file_size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        return Ok(ExrThumbWork {
            path: path.to_path_buf(),
            file_name,
            file_size_bytes,
            width: thumb_w,
            height: thumb_h,
            num_layers: layers_info.len(),
            pixels,
        });
    }

    let (_width, _height, thumb_w, thumb_h) = *dims.borrow();
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    let file_size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // Skopiuj piksele do lokalnej zmiennej, aby zakończyć pożyczkę Ref zanim zwrócimy wynik
    let pixels_vec = {
        let borrow = out_pixels.borrow();
        borrow.clone()
    };

    Ok(ExrThumbWork {
        path: path.to_path_buf(),
        file_name,
        file_size_bytes,
        width: thumb_w,
        height: thumb_h,
        num_layers: layers_info.len(),
        pixels: pixels_vec,
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
struct ThumbKey {
    path: PathBuf,
    modified: u64,
    preset: ThumbPresetKey,
}

// Przechowujemy wyłącznie gotowe piksele RGBA8 i podstawowe metadane
#[derive(Clone)]
struct ThumbValue {
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

fn get_thumb_cache() -> &'static Mutex<LruCache<ThumbKey, ThumbValue>> {
    THUMB_CACHE.get_or_init(|| Mutex::new(LruCache::new(std::num::NonZeroUsize::new(256).unwrap())))
}

fn c_get(
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

fn put_thumb_cache(work: &ExrThumbWork, thumb_h: u32, exposure: f32, gamma: f32, tonemap_mode: i32) {
    let preset = make_preset(thumb_h, exposure, gamma, tonemap_mode);
    let key = ThumbKey { path: work.path.clone(), modified: file_mtime_u64(&work.path), preset };
    let val = ThumbValue {
        width: work.width,
        height: work.height,
        num_layers: work.num_layers,
        file_size_bytes: work.file_size_bytes,
        file_name: work.file_name.clone(),
        pixels: work.pixels.clone(),
    };
    if let Ok(mut c) = get_thumb_cache().lock() { c.put(key, val); }
}