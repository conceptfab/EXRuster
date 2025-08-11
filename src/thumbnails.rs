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
            let res = generate_single_exr_thumbnail_work(path, thumb_height, exposure, gamma);
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
) -> anyhow::Result<ExrThumbWork> {
    // Scentralizowany wybór i wczytanie warstwy – w trybie szybkim
    // 1) Szybka próba: pierwsza RGBA dla typowych nazw
    let path_buf = path.to_path_buf();
    let layers_info = extract_layers_info(&path_buf)
        .with_context(|| format!("Błąd odczytu EXR: {}", path.display()))?;
    let best_layer_name = find_best_layer(&layers_info);
    let (raw_pixels, width, height, _current_layer) = load_specific_layer(&path_buf, &best_layer_name, None)
        .with_context(|| format!("Błąd wczytania warstwy '{}': {}", best_layer_name, path.display()))?;

    // Wylicz macierz konwersji primaries → sRGB (per‑part/per‑layer) z adaptacją Bradford
    let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(&path_buf.as_path(), &best_layer_name).ok();

    // Oblicz rozmiar miniaturki - zawsze 150px wysokości, szerokość proporcjonalna
    let scale = thumb_height as f32 / height as f32;
    let thumb_h = thumb_height;
    let thumb_w = (width as f32 * scale) as u32;

    // Bufor wyjściowy miniaturki (RGBA8)
    let mut pixels: Vec<u8> = vec![0; (thumb_w as usize) * (thumb_h as usize) * 4];

    // Samplowanie nearest-neighbor z mapowaniem procesem jak w preview (ACES + gamma)
    let raw_width = width as usize;
    let m = color_matrix_rgb_to_srgb;
    pixels
        .par_chunks_mut(4)
        .enumerate()
        .for_each(|(i, out)| {
            let x = (i as u32) % thumb_w;
            let y = (i as u32) / thumb_w;

            let src_x = ((x as f32 / scale) as u32).min(width.saturating_sub(1));
            let src_y = ((y as f32 / scale) as u32).min(height.saturating_sub(1));
            let src_idx = (src_y as usize) * raw_width + (src_x as usize);

            let (mut r, mut g, mut b, a) = raw_pixels[src_idx];
            if let Some(mat) = m {
                let rr = mat[0][0] * r + mat[0][1] * g + mat[0][2] * b;
                let gg = mat[1][0] * r + mat[1][1] * g + mat[1][2] * b;
                let bb = mat[2][0] * r + mat[2][1] * g + mat[2][2] * b;
                r = rr; g = gg; b = bb;
            }
            let px = process_pixel(r, g, b, a, exposure, gamma);
            out[0] = px.r; out[1] = px.g; out[2] = px.b; out[3] = px.a;
        });

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    let file_size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    Ok(ExrThumbWork {
        path: path.to_path_buf(),
        file_name,
        file_size_bytes,
        width: thumb_w,
        height: thumb_h,
        num_layers: layers_info.len(),
        pixels,
    })
}



