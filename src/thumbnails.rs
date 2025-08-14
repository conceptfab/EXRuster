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

// Dodaj import dla GPU context i processor
use crate::gpu_context::GpuContext;
use crate::gpu_thumbnails::GpuThumbnailProcessor;

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

/// GPU-accelerated version: generuje miniaturki z wykorzystaniem GPU jeśli dostępne
#[allow(dead_code)]
pub fn generate_exr_thumbnails_in_dir_gpu(
    directory: &Path,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
    gpu_context: Option<&GpuContext>,
) -> anyhow::Result<Vec<ExrThumbnailInfo>> {
    let files = list_exr_files(directory)?;
    let total_files = files.len();
    
    // Natychmiastowe powiadomienie o liczbie plików
    if let Some(p) = progress { 
        if total_files > 0 {
            p.set(0.0, Some(&format!("Found {} files, processing...", total_files))); 
        } else {
            p.finish(Some("No EXR files found"));
            return Ok(Vec::new());
        }
    }

    if total_files == 0 {
        if let Some(p) = progress { p.finish(Some("No EXR files")); }
        return Ok(Vec::new());
    }

    // Sprawdź czy GPU jest dostępne i czy warto go użyć
    let use_gpu = gpu_context.is_some() && should_use_gpu_for_thumbnails(&files);
    
    if use_gpu {
        if let Some(p) = progress { 
            p.set(0.1, Some("GPU acceleration enabled for thumbnails")); 
        }
        generate_thumbnails_gpu(files, thumb_height, exposure, gamma, tonemap_mode, progress, gpu_context.unwrap())
    } else {
        if let Some(p) = progress { 
            p.set(0.1, Some("Using CPU for thumbnail generation")); 
        }
        generate_thumbnails_cpu(files, thumb_height, exposure, gamma, tonemap_mode, progress)
    }
}

/// Sprawdza czy warto użyć GPU dla generowania miniaturek
#[allow(dead_code)]
fn should_use_gpu_for_thumbnails(_files: &[PathBuf]) -> bool {
    // GPU ma być używane zawsze gdy jest dostępne!
    // Usuwamy ograniczenia rozmiaru plików - GPU acceleration dla wszystkich plików EXR
    true
}

/// Generuje miniaturki używając GPU acceleration
#[allow(dead_code)]
fn generate_thumbnails_gpu(
    files: Vec<PathBuf>,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
    gpu_context: &GpuContext,
) -> anyhow::Result<Vec<ExrThumbnailInfo>> {
    let total_files = files.len();
    
    if let Some(p) = progress { 
        p.set(0.15, Some("GPU: Preparing batch processing...")); 
    }
    
    // 1) Przygotuj dane dla GPU - wczytaj wszystkie pliki do pamięci
    let mut thumbnail_works = Vec::new();
    
    for (idx, path) in files.iter().enumerate() {
        if let Some(p) = progress {
            let frac = 0.15 + (idx as f32 / total_files as f32) * 0.3; // 15% - 45%
            p.set(frac, Some(&format!("GPU: Loading {}/{} {}", idx + 1, total_files, path.file_name().and_then(|n| n.to_str()).unwrap_or("?"))));
        }
        
        // Wczytaj dane pliku EXR
        match load_exr_data_for_gpu(path, thumb_height) {
            Ok(exr_data) => {
                thumbnail_works.push(exr_data);
            }
            Err(e) => {
                // Log błąd ale kontynuuj z innymi plikami
                eprintln!("GPU: Failed to load {}: {}", path.display(), e);
            }
        }
    }
    
    if thumbnail_works.is_empty() {
        if let Some(p) = progress { 
            p.set(0.5, Some("GPU: No files loaded, falling back to CPU")); 
        }
        return generate_thumbnails_cpu(files, thumb_height, exposure, gamma, tonemap_mode, progress);
    }
    
    if let Some(p) = progress { 
        p.set(0.5, Some(&format!("GPU: Processing {} files in batch...", thumbnail_works.len()))); 
    }
    
    // 2) Przetwórz wszystkie miniaturki na GPU w jednym batch
    match process_thumbnails_batch_gpu(
        &mut thumbnail_works, 
        exposure, 
        gamma, 
        tonemap_mode, 
        gpu_context,
        progress
    ) {
        Ok(processed_works) => {
            if let Some(p) = progress { 
                p.set(0.9, Some("GPU: Converting to UI format...")); 
            }
            
            // 3) Konwertuj wyniki GPU do formatu UI
            let thumbnails = convert_gpu_works_to_ui(processed_works);
            
            if let Some(p) = progress { 
                p.finish(Some(&format!("GPU: {} thumbnails processed successfully", thumbnails.len()))); 
            }
            
            Ok(thumbnails)
        }
        Err(e) => {
            if let Some(p) = progress { 
                p.set(0.6, Some("GPU failed, falling back to CPU...")); 
            }
            // Fallback do CPU w przypadku błędu GPU
            eprintln!("GPU processing failed: {}, falling back to CPU", e);
            generate_thumbnails_cpu(files, thumb_height, exposure, gamma, tonemap_mode, progress)
        }
    }
}

/// Generuje miniaturki używając CPU (oryginalna implementacja)
fn generate_thumbnails_cpu(
    files: Vec<PathBuf>,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<Vec<ExrThumbnailInfo>> {
    let total_files = files.len();

    // 1) Równolegle generuj dane miniaturek w typie bezpiecznym dla wątków (bez slint::Image)
    let completed = AtomicUsize::new(0);
    let works: Vec<ExrThumbWork> = files
        .into_par_iter() // Użyj into_par_iter zamiast par_iter dla lepszej wydajności
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
                    // Dodaj więcej informacji w statusie
                    p.set(frac, Some(&format!("Cached: {}/{} {}", n, total_files, path.file_name().and_then(|n| n.to_str()).unwrap_or("?"))));
                }
                return Some(cached);
            }

            let res = generate_single_exr_thumbnail_work(&path, thumb_height, exposure, gamma, tonemap_mode)
                .map(|work| {
                    // Zapisz do cache
                    put_thumb_cache(&work, thumb_height, exposure, gamma, tonemap_mode);
                    work
                });
            let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(p) = progress {
                let frac = (n as f32) / (total_files as f32);
                // Dodaj więcej informacji w statusie
                p.set(frac, Some(&format!("Processed: {}/{} {}", n, total_files, path.file_name().and_then(|n| n.to_str()).unwrap_or("?"))));
            }
            match res {
                Ok(work) => Some(work),
                Err(_e) => None, // tu można logować błąd
            }
        })
        .collect();

    // 2) Na głównym wątku skonstruuj slint::Image (nie jest Send)
    let works_count = works.len(); // Zapisz długość przed przeniesieniem
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

    if let Some(p) = progress { 
        p.finish(Some(&format!("Thumbnails loaded: {} files processed", works_count))); 
    }
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

#[derive(Clone)]
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
        
        // Zoptymalizowane przetwarzanie z SIMD
        let total = (thumb_w as usize) * (thumb_h as usize);
        
        // Użyj większego bloku SIMD dla lepszej wydajności
        let par_pixels: Vec<u8> = (0..(total / 8)).into_par_iter().flat_map(|block| {
            let base = block * 8;
            let mut rr = [0.0f32; 8];
            let mut gg = [0.0f32; 8];
            let mut bb = [0.0f32; 8];
            let mut aa = [1.0f32; 8];
            for lane in 0..8 {
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
                f32x4::from_array([rr[0], rr[1], rr[2], rr[3]]), 
                f32x4::from_array([gg[0], gg[1], gg[2], gg[3]]), 
                f32x4::from_array([bb[0], bb[1], bb[2], bb[3]]), 
                exposure, gamma, tonemap_mode);
            let a8_1 = f32x4::from_array([aa[0], aa[1], aa[2], aa[3]]).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
            
            let (r8_2, g8_2, b8_2) = crate::image_processing::tone_map_and_gamma_simd(
                f32x4::from_array([rr[4], rr[5], rr[6], rr[7]]), 
                f32x4::from_array([gg[4], gg[5], gg[6], gg[7]]), 
                f32x4::from_array([bb[4], bb[5], bb[6], bb[7]]), 
                exposure, gamma, tonemap_mode);
            let a8_2 = f32x4::from_array([aa[4], aa[5], aa[6], aa[7]]).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
            
            let ra1: [f32; 4] = r8.into();
            let ga1: [f32; 4] = g8.into();
            let ba1: [f32; 4] = b8.into();
            let aa1: [f32; 4] = a8_1.into();
            
            let ra2: [f32; 4] = r8_2.into();
            let ga2: [f32; 4] = g8_2.into();
            let ba2: [f32; 4] = b8_2.into();
            let aa2: [f32; 4] = a8_2.into();
            
            let mut chunk = Vec::with_capacity(32);
            for i in 0..4 {
                chunk.push((ra1[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((ga1[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((ba1[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((aa1[i] * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            for i in 0..4 {
                chunk.push((ra2[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((ga2[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((ba2[i] * 255.0).round().clamp(0.0, 255.0) as u8);
                chunk.push((aa2[i] * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            chunk
        }).collect();

        pixels[..par_pixels.len()].copy_from_slice(&par_pixels);

        // Obsłuż pozostałe piksele (resztę z dzielenia)
        for i in (total / 8 * 8)..total {
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

/// Struktura danych EXR przygotowana dla GPU
#[allow(dead_code)]
struct ExrDataForGpu {
    path: PathBuf,
    file_name: String,
    file_size_bytes: u64,
    width: u32,
    height: u32,
    num_layers: usize,
    raw_pixels: Vec<f32>, // RGBA jako płaskie f32
    color_matrix: Option<[[f32; 3]; 3]>, // Macierz RGB→sRGB
    target_width: u32,
    target_height: u32,
}

/// Wczytuje dane EXR przygotowane dla GPU
#[allow(dead_code)]
fn load_exr_data_for_gpu(
    path: &Path, 
    thumb_height: u32
) -> anyhow::Result<ExrDataForGpu> {
    let path_buf = path.to_path_buf();
    
    // Wczytaj metadane
    let layers_info = extract_layers_info(&path_buf)
        .with_context(|| format!("Błąd odczytu EXR: {}", path.display()))?;
    
    // Znajdź najlepszą warstwę
    let best_layer_name = find_best_layer(&layers_info);
    
    // Wczytaj dane pikseli
    let (raw_pixels, width, height, _current_layer) = load_specific_layer(&path_buf, &best_layer_name, None)
        .with_context(|| format!("Błąd wczytania warstwy \"{}\": {}", best_layer_name, path.display()))?;
    
    // Macierz kolorów
    let color_matrix = compute_rgb_to_srgb_matrix_from_file_for_layer(&path_buf.as_path(), &best_layer_name)
        .ok()
        .map(|mat3| {
            // Konwertuj glam::Mat3 na [[f32; 3]; 3]
            [
                [mat3.x_axis.x, mat3.x_axis.y, mat3.x_axis.z],
                [mat3.y_axis.x, mat3.y_axis.y, mat3.y_axis.z],
                [mat3.z_axis.x, mat3.z_axis.y, mat3.z_axis.z],
            ]
        });
    
    // Oblicz wymiary miniatury
    let scale = thumb_height as f32 / height as f32;
    let thumb_h = thumb_height.max(1);
    let thumb_w = ((width as f32) * scale).max(1.0).round() as u32;
    
    // Konwertuj piksele do płaskiego formatu f32
    let flat_pixels: Vec<f32> = raw_pixels.iter()
        .flat_map(|(r, g, b, a)| vec![*r, *g, *b, *a])
        .collect();
    
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    let file_size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    
    Ok(ExrDataForGpu {
        path: path_buf,
        file_name,
        file_size_bytes,
        width,
        height,
        num_layers: layers_info.len(),
        raw_pixels: flat_pixels,
        color_matrix,
        target_width: thumb_w,
        target_height: thumb_h,
    })
}

/// Przetwarza wszystkie miniaturki na GPU w jednym batch
#[allow(dead_code)]
fn process_thumbnails_batch_gpu(
    thumbnail_works: &mut [ExrDataForGpu],
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    gpu_context: &GpuContext,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<Vec<ExrThumbWork>> {
    // Sprawdź czy GPU jest dostępne
    if let Some(p) = progress { 
        p.set(0.55, Some("GPU: Initializing GPU processor...")); 
    }
    
    // Utwórz GPU processor
    let gpu_processor = match GpuThumbnailProcessor::new(gpu_context.clone()) {
        Ok(processor) => processor,
        Err(e) => {
            eprintln!("GPU: Failed to create processor: {}", e);
            if let Some(p) = progress { 
                p.set(0.6, Some("GPU failed, falling back to CPU...")); 
            }
            return Err(anyhow::anyhow!("GPU processor creation failed: {}", e));
        }
    };
    
    if let Some(p) = progress { 
        p.set(0.6, Some(&format!("GPU: Processing {} files in batch...", thumbnail_works.len()))); 
    }
    
    let mut results = Vec::new();
    let total = thumbnail_works.len();
    
    for (idx, work) in thumbnail_works.iter().enumerate() {
        if let Some(p) = progress {
            let frac = 0.6 + (idx as f32 / total as f32) * 0.25; // 60% - 85%
            p.set(frac, Some(&format!("GPU: Processing {}/{} {}", idx + 1, total, work.file_name)));
        }
        
        // Przetwórz na GPU
        match gpu_processor.process_thumbnail(
            &work.raw_pixels,
            work.width,
            work.height,
            work.target_width,
            work.target_height,
            exposure,
            gamma,
            tonemap_mode as u32,
            work.color_matrix,
        ) {
            Ok(gpu_pixels) => {
                // Konwertuj wyniki GPU do formatu ExrThumbWork
                let pixels = convert_gpu_pixels_to_rgba8(&gpu_pixels);
                
                let processed_work = ExrThumbWork {
                    path: work.path.clone(),
                    file_name: work.file_name.clone(),
                    file_size_bytes: work.file_size_bytes,
                    width: work.target_width,
                    height: work.target_height,
                    num_layers: work.num_layers,
                    pixels,
                };
                
                results.push(processed_work);
            }
            Err(e) => {
                eprintln!("GPU: Failed to process {}: {}, falling back to CPU", work.file_name, e);
                
                // Fallback do CPU dla tego pliku
                let processed_work = process_single_thumbnail_cpu(work, exposure, gamma, tonemap_mode)?;
                results.push(processed_work);
            }
        }
    }
    
    Ok(results)
}

/// Konwertuje piksele GPU (u32) do formatu RGBA8 (Vec<u8>)
#[allow(dead_code)]
fn convert_gpu_pixels_to_rgba8(gpu_pixels: &[u32]) -> Vec<u8> {
    let mut rgba8_pixels = Vec::with_capacity(gpu_pixels.len() * 4);
    
    for &pixel in gpu_pixels {
        // Rozpakuj RGBA z u32 (format: AABBGGRR)
        let r = (pixel & 0xFF) as u8;
        let g = ((pixel >> 8) & 0xFF) as u8;
        let b = ((pixel >> 16) & 0xFF) as u8;
        let a = ((pixel >> 24) & 0xFF) as u8;
        
        rgba8_pixels.push(r);
        rgba8_pixels.push(g);
        rgba8_pixels.push(b);
        rgba8_pixels.push(a);
    }
    
    rgba8_pixels
}

/// Przetwarza pojedynczą miniaturkę na CPU (fallback dla GPU)
#[allow(dead_code)]
fn process_single_thumbnail_cpu(
    work: &ExrDataForGpu,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> anyhow::Result<ExrThumbWork> {
    let total_pixels = (work.target_width * work.target_height) as usize;
    let mut pixels = vec![0u8; total_pixels * 4];
    
    // Proste skalowanie nearest neighbor
    let scale_x = work.width as f32 / work.target_width as f32;
    let scale_y = work.height as f32 / work.target_height as f32;
    
    for y in 0..work.target_height {
        for x in 0..work.target_width {
            let src_x = ((x as f32 * scale_x) as u32).min(work.width.saturating_sub(1));
            let src_y = ((y as f32 * scale_y) as u32).min(work.height.saturating_sub(1));
            let src_idx = (src_y as usize * work.width as usize + src_x as usize) * 4;
            let dst_idx = (y as usize * work.target_width as usize + x as usize) * 4;
            
            if src_idx + 3 < work.raw_pixels.len() && dst_idx + 3 < pixels.len() {
                let mut r = work.raw_pixels[src_idx];
                let mut g = work.raw_pixels[src_idx + 1];
                let mut b = work.raw_pixels[src_idx + 2];
                let a = work.raw_pixels[src_idx + 3];
                
                // Zastosuj macierz kolorów
                if let Some(matrix) = work.color_matrix {
                    let new_r = matrix[0][0] * r + matrix[0][1] * g + matrix[0][2] * b;
                    let new_g = matrix[1][0] * r + matrix[1][1] * g + matrix[1][2] * b;
                    let new_b = matrix[2][0] * r + matrix[2][1] * g + matrix[2][2] * b;
                    r = new_r; g = new_g; b = new_b;
                }
                
                // Tone mapping i gamma
                let px = process_pixel(r, g, b, a, exposure, gamma, tonemap_mode);
                
                pixels[dst_idx] = px.r;
                pixels[dst_idx + 1] = px.g;
                pixels[dst_idx + 2] = px.b;
                pixels[dst_idx + 3] = px.a;
            }
        }
    }
    
    Ok(ExrThumbWork {
        path: work.path.clone(),
        file_name: work.file_name.clone(),
        file_size_bytes: work.file_size_bytes,
        width: work.target_width,
        height: work.target_height,
        num_layers: work.num_layers,
        pixels,
    })
}

/// Konwertuje wyniki GPU do formatu UI
#[allow(dead_code)]
fn convert_gpu_works_to_ui(works: Vec<ExrThumbWork>) -> Vec<ExrThumbnailInfo> {
    works.into_iter()
        .map(|w| {
            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w.width, w.height);
            let slice = buffer.make_mut_slice();
            
            // Skopiuj piksele do bufora Slint
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
        .collect()
}