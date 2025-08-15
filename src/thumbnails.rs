use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use rayon::prelude::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::sync::atomic::{AtomicUsize, Ordering};


use crate::image_processing::process_pixel;

use crate::image_cache::{extract_layers_info, find_best_layer, load_specific_layer};
use crate::progress::ProgressSink;
use crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer;
use glam::Vec3;
use std::sync::{Mutex, OnceLock};
use lru::LruCache;

// Dodaj import dla GPU context i processor
use crate::gpu_context::GpuContext;
use crate::gpu_thumbnails::GpuThumbnailProcessor;

/// Funkcja pomocnicza do precyzyjnej interpolacji bilinearnej
#[inline]
fn precise_bilinear_interpolation(
    r00: f32, r10: f32, r01: f32, r11: f32,
    g00: f32, g10: f32, g01: f32, g11: f32,
    b00: f32, b10: f32, b01: f32, b11: f32,
    a00: f32, a10: f32, a01: f32, a11: f32,
    fx: f32, fy: f32
) -> (f32, f32, f32, f32) {
    // Precyzyjna interpolacja bilinearna z clamp do [0,1]
    let r0 = (r00 * (1.0 - fx) + r10 * fx).clamp(0.0, 1.0);
    let r1 = (r01 * (1.0 - fx) + r11 * fx).clamp(0.0, 1.0);
    let r = (r0 * (1.0 - fy) + r1 * fy).clamp(0.0, 1.0);
    
    let g0 = (g00 * (1.0 - fx) + g10 * fx).clamp(0.0, 1.0);
    let g1 = (g01 * (1.0 - fx) + g11 * fx).clamp(0.0, 1.0);
    let g = (g0 * (1.0 - fy) + g1 * fy).clamp(0.0, 1.0);
    
    let b0 = (b00 * (1.0 - fx) + b10 * fx).clamp(0.0, 1.0);
    let b1 = (b01 * (1.0 - fx) + b11 * fx).clamp(0.0, 1.0);
    let b = (b0 * (1.0 - fy) + b1 * fy).clamp(0.0, 1.0);
    
    let a0 = (a00 * (1.0 - fx) + a10 * fx).clamp(0.0, 1.0);
    let a1 = (a01 * (1.0 - fx) + a11 * fx).clamp(0.0, 1.0);
    let a = (a0 * (1.0 - fy) + a1 * fy).clamp(0.0, 1.0);
    
    (r, g, b, a)
}

/// Zwięzła reprezentacja miniaturki EXR do wyświetlenia w UI
#[allow(dead_code)]
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
pub fn generate_thumbnails_cpu(
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

pub fn generate_single_exr_thumbnail_work(
    path: &Path,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> anyhow::Result<ExrThumbWork> {
    let path_buf = path.to_path_buf();
    
    // DEBUG: Loguj parametry generowania
    println!("Generating thumbnail for {}: exposure={}, gamma={}, tonemap={}", 
             path.file_name().and_then(|n| n.to_str()).unwrap_or("?"), 
             exposure, gamma, tonemap_mode);

    // Krok 1: Szybkie pobranie metadanych (liczba warstw, macierz kolorów)
    let layers_info = extract_layers_info(&path_buf)
        .with_context(|| format!("Błąd odczytu meta EXR: {}", path.display()))?;
    let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, "").ok();

    // Krok 2: Fallback do wolniejszej, ale bardziej niezawodnej metody
    let best_layer_name = find_best_layer(&layers_info);
    let (raw_pixels, width, height, _) = load_specific_layer(&path_buf, &best_layer_name, None)?;

    let scale = thumb_height as f32 / height as f32;
    let thumb_w = ((width as f32) * scale).max(1.0).round() as u32;
    let thumb_h = thumb_height;

    // Sprawdź czy wymiary są poprawne - POPRAWIONE!
    if thumb_w == 0 || thumb_h == 0 {
        return Err(anyhow::anyhow!("Invalid thumbnail dimensions: {}x{}", thumb_w, thumb_h));
    }
    
    // Sprawdź czy scale jest rozsądny
    if scale < 0.01 || scale > 100.0 {
        println!("WARNING: Unusual scale: {:.6} for {}x{} -> {}x{}", 
                 scale, width, height, thumb_w, thumb_h);
    }

    // Dodaj debugowanie współrzędnych - POPRAWIONE!
    println!("DEBUG: Original: {}x{}, Thumb: {}x{}, Scale: {:.6}", 
             width, height, thumb_w, thumb_h, scale);

    let mut pixels: Vec<u8> = vec![0; (thumb_w as usize) * (thumb_h as usize) * 4];
    let m = color_matrix_rgb_to_srgb;

    // ALTERNATYWNE PODEJŚCIE: użyj indeksów bezpośrednio zamiast chunks_mut
    // DODATKOWO: Użyj nearest neighbor zamiast bilinearnej interpolacji dla miniaturek
    let use_nearest_neighbor = true; // Toggle dla testowania
    
    for y in 0..thumb_h {
        for x in 0..thumb_w {
            let i = (y as usize) * (thumb_w as usize) + (x as usize);
            let buffer_idx = i * 4;
            
            // Dodaj debugowanie dla pierwszych kilku pikseli
            if i < 20 {
                println!("DEBUG: Pixel {}: pos=({},{}) -> buffer_idx={}", 
                         i, x, y, buffer_idx);
            }
            
            // Współrzędne źródłowe z częścią ułamkową - POPRAWIONE!
            // Używamy odwrotności scale do mapowania współrzędnych
            let src_x_f = (x as f32) * (width as f32) / (thumb_w as f32);
            let src_y_f = (y as f32) * (height as f32) / (thumb_h as f32);
        
            // Dodaj debugowanie dla pierwszych kilku pikseli
            if i < 10 {
                println!("DEBUG: Pixel {}: pos=({},{}) -> src=({:.2},{:.2})", 
                         i, x, y, src_x_f, src_y_f);
            }
            
            if use_nearest_neighbor {
                // NEAREST NEIGHBOR - może rozwiązać problem z przesuniętymi liniami!
                let src_x = src_x_f.round() as u32;
                let src_y = src_y_f.round() as u32;
                let src_x = src_x.min(width.saturating_sub(1));
                let src_y = src_y.min(height.saturating_sub(1));
                
                let idx = (src_y as usize) * (width as usize) + (src_x as usize);
                if idx < raw_pixels.len() {
                    let (r, g, b, a) = raw_pixels[idx];
                    
                    // Reszta kodu pozostaje bez zmian (macierz kolorów i process_pixel)
                    let mut final_r = r;
                    let mut final_g = g;
                    let mut final_b = b;
                    
                    if let Some(mat) = m {
                        let v = mat * Vec3::new(final_r, final_g, final_b);
                        final_r = v.x; final_g = v.y; final_b = v.z;
                    }
                    let px = process_pixel(final_r, final_g, final_b, a, exposure, gamma, tonemap_mode);
                    pixels[buffer_idx] = px.r; pixels[buffer_idx + 1] = px.g; pixels[buffer_idx + 2] = px.b; pixels[buffer_idx + 3] = px.a;
                }
            } else {
                // ORYGINALNA INTERPOLACJA BILINEARNA
                let src_x0 = src_x_f.floor() as u32;
                let src_y0 = src_y_f.floor() as u32;
                let src_x1 = (src_x0 + 1).min(width.saturating_sub(1));
                let src_y1 = (src_y0 + 1).min(height.saturating_sub(1));
                
                // Wagi interpolacji
                let fx = src_x_f.fract();
                let fy = src_y_f.fract();
                
                // Pobierz 4 sąsiednie piksele
                let idx00 = (src_y0 as usize) * (width as usize) + (src_x0 as usize);
                let idx10 = (src_y0 as usize) * (width as usize) + (src_x1 as usize);
                let idx01 = (src_y1 as usize) * (width as usize) + (src_x0 as usize);
                let idx11 = (src_y1 as usize) * (width as usize) + (src_x1 as usize);
                
                // Sprawdź czy indeksy są w zakresie
                if idx11 >= raw_pixels.len() {
                    println!("ERROR: Index out of bounds: idx11={}, len={}", idx11, raw_pixels.len());
                    continue; // Pomiń problematyczny piksel zamiast return
                }
                
                let (r00, g00, b00, a00) = raw_pixels[idx00];
                let (r10, g10, b10, a10) = raw_pixels[idx10];
                let (r01, g01, b01, a01) = raw_pixels[idx01];
                let (r11, g11, b11, a11) = raw_pixels[idx11];
                
                // Interpolacja bilinearna - POPRAWIONE!
                let (r, g, b, a) = precise_bilinear_interpolation(
                    r00, r10, r01, r11,
                    g00, g10, g01, g11,
                    b00, b10, b01, b11,
                    a00, a10, a01, a11,
                    fx, fy
                );
                
                // Reszta kodu pozostaje bez zmian (macierz kolorów i process_pixel)
                let mut final_r = r;
                let mut final_g = g;
                let mut final_b = b;
                
                if let Some(mat) = m {
                    let v = mat * Vec3::new(final_r, final_g, final_b);
                    final_r = v.x; final_g = v.y; final_b = v.z;
                }
                let px = process_pixel(final_r, final_g, final_b, a, exposure, gamma, tonemap_mode);
                pixels[buffer_idx] = px.r; pixels[buffer_idx + 1] = px.g; pixels[buffer_idx + 2] = px.b; pixels[buffer_idx + 3] = px.a;
            }
        }
    } // Zamykający nawias dla pętli for

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    let file_size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    Ok(ExrThumbWork {
        path: path_buf,
        file_name,
        file_size_bytes,
        width: thumb_w,
        height: thumb_h,
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

pub fn put_thumb_cache(work: &ExrThumbWork, thumb_h: u32, exposure: f32, gamma: f32, tonemap_mode: i32) {
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
    
    // Oblicz wymiary miniatury - POPRAWIONE!
    let scale = thumb_height as f32 / height as f32;
    let thumb_h = thumb_height;
    let thumb_w = ((width as f32) * scale).max(1.0).round() as u32;
    
    // Dodaj debugowanie współrzędnych
    println!("DEBUG: Original: {}x{}, Thumb: {}x{}, Scale: {:.6}", 
             width, height, thumb_w, thumb_h, scale);
    
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
        // Rozpakuj RGBA z u32 (format GPU shader: ABGR - A w najwyższych bitach)
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
            let src_x_f = x as f32 * scale_x;
            let src_y_f = y as f32 * scale_y;
            
            let src_x0 = src_x_f.floor() as u32;
            let src_y0 = src_y_f.floor() as u32;
            let src_x1 = (src_x0 + 1).min(work.width.saturating_sub(1));
            let src_y1 = (src_y0 + 1).min(work.height.saturating_sub(1));
            
            let fx = src_x_f.fract();
            let fy = src_y_f.fract();
            
            // Indeksy dla 4 pikseli
            let idx00 = (src_y0 as usize * work.width as usize + src_x0 as usize) * 4;
            let idx10 = (src_y0 as usize * work.width as usize + src_x1 as usize) * 4;
            let idx01 = (src_y1 as usize * work.width as usize + src_x0 as usize) * 4;
            let idx11 = (src_y1 as usize * work.width as usize + src_x1 as usize) * 4;
            
            if idx11 + 3 < work.raw_pixels.len() {
                // Pobierz 4 piksele i interpoluj
                let r00 = work.raw_pixels[idx00];
                let g00 = work.raw_pixels[idx00 + 1];
                let b00 = work.raw_pixels[idx00 + 2];
                let a00 = work.raw_pixels[idx00 + 3];
                
                let r10 = work.raw_pixels[idx10];
                let g10 = work.raw_pixels[idx10 + 1];
                let b10 = work.raw_pixels[idx10 + 2];
                let a10 = work.raw_pixels[idx10 + 3];
                
                let r01 = work.raw_pixels[idx01];
                let g01 = work.raw_pixels[idx01 + 1];
                let b01 = work.raw_pixels[idx01 + 2];
                let a01 = work.raw_pixels[idx01 + 3];
                
                let r11 = work.raw_pixels[idx11];
                let g11 = work.raw_pixels[idx11 + 1];
                let b11 = work.raw_pixels[idx11 + 2];
                let a11 = work.raw_pixels[idx11 + 3];
                
                // Bilinearna interpolacja
                let r = lerp2d(r00, r10, r01, r11, fx, fy);
                let g = lerp2d(g00, g10, g01, g11, fx, fy);
                let b = lerp2d(b00, b10, b01, b11, fx, fy);
                let a = lerp2d(a00, a10, a01, a11, fx, fy);
                
                let mut final_r = r;
                let mut final_g = g;
                let mut final_b = b;
                
                // Zastosuj macierz kolorów
                if let Some(matrix) = work.color_matrix {
                    let new_r = matrix[0][0] * final_r + matrix[0][1] * final_g + matrix[0][2] * final_b;
                    let new_g = matrix[1][0] * final_r + matrix[1][1] * final_g + matrix[1][2] * final_b;
                    let new_b = matrix[2][0] * final_r + matrix[2][1] * final_g + matrix[2][2] * final_b;
                    final_r = new_r; final_g = new_g; final_b = new_b;
                }
                
                // Tone mapping i gamma
                let px = process_pixel(final_r, final_g, final_b, a, exposure, gamma, tonemap_mode);
                
                let dst_idx = (y as usize * work.target_width as usize + x as usize) * 4;
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

// Funkcja pomocnicza dla interpolacji bilinearnej
fn lerp2d(v00: f32, v10: f32, v01: f32, v11: f32, fx: f32, fy: f32) -> f32 {
    let v0 = v00 * (1.0 - fx) + v10 * fx;
    let v1 = v01 * (1.0 - fx) + v11 * fx;
    v0 * (1.0 - fy) + v1 * fy
}