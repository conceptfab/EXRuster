use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use rayon::prelude::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use exr::prelude as exr;

use crate::image_processing::process_pixel;
use crate::image_cache::{extract_layers_info, find_best_layer, load_specific_layer};
use crate::progress::ProgressSink;

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

    // 1) Równolegle generuj dane miniaturek w typie bezpiecznym dla wątków (bez slint::Image)
    let works: Vec<ExrThumbWork> = files
        .par_iter()
        .enumerate()
        .filter_map(|(_i, path)| match generate_single_exr_thumbnail_work(path, thumb_height, exposure, gamma) {
            Ok(work) => Some(work),
            Err(_e) => None, // tu można logować błąd
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
    // Scentralizowany wybór i wczytanie warstwy
    let path_buf = path.to_path_buf();
    let layers_info = extract_layers_info(&path_buf)
        .with_context(|| format!("Błąd odczytu EXR: {}", path.display()))?;
    let best_layer_name = find_best_layer(&layers_info);
    let (raw_pixels, width, height, _current_layer) = load_specific_layer(&path_buf, &best_layer_name, None)
        .with_context(|| format!("Błąd wczytania warstwy '{}': {}", best_layer_name, path.display()))?;

    // Wylicz macierz konwersji primaries → sRGB (per‑part/per‑layer) z adaptacją Bradford
    let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(&path_buf, &best_layer_name).ok();

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

// === Chromaticities helpers (lokalne dla thumbnails) ===
fn compute_rgb_to_srgb_matrix_from_file_for_layer(path: &PathBuf, layer_name: &str) -> anyhow::Result<[[f32; 3]; 3]> {
    let img = exr::read_all_data_from_file(path)?;
    let wanted_lower = layer_name.to_lowercase();
    let mut nums: Option<Vec<f64>> = None;

    // Spróbuj znaleźć chromaticities w atrybutach odpowiadającego partu/warstwy
    'outer: for layer in img.layer_data.iter() {
        let base_name: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
        let lname = base_name.unwrap_or_else(|| "".to_string());
        let lname_lower = lname.to_lowercase();
        let matches = (wanted_lower.is_empty() && lname_lower.is_empty()) || (!wanted_lower.is_empty() && lname_lower.contains(&wanted_lower));
        if matches {
            if let Some((_k, v)) = layer.attributes.other.iter().find(|(k, _)| {
                let name_dbg = format!("{:?}", k).to_lowercase();
                let name = name_dbg.trim_matches('"');
                name == "chromaticities"
            }) {
                let mut out: Vec<f64> = Vec::new();
                let mut cur = String::new();
                for c in format!("{:?}", v).chars() {
                    if c.is_ascii_digit() || c == '.' || c == '-' { cur.push(c); }
                    else { if !cur.is_empty() { if let Ok(n) = cur.parse::<f64>() { out.push(n); } cur.clear(); } }
                }
                if !cur.is_empty() { if let Ok(n) = cur.parse::<f64>() { out.push(n); } }
                if out.len() >= 8 { nums = Some(out); break 'outer; }
            }
        }
    }

    // Fallback: globalny nagłówek
    let nums = if let Some(n) = nums { n } else {
        let mut out: Vec<f64> = Vec::new();
        if let Some((_k, v)) = img.attributes.other.iter().find(|(k, _)| {
            let name_dbg = format!("{:?}", k).to_lowercase();
            let name = name_dbg.trim_matches('"');
            name == "chromaticities"
        }) {
            let mut cur = String::new();
            for c in format!("{:?}", v).chars() {
                if c.is_ascii_digit() || c == '.' || c == '-' { cur.push(c); }
                else { if !cur.is_empty() { if let Ok(n) = cur.parse::<f64>() { out.push(n); } cur.clear(); } }
            }
            if !cur.is_empty() { if let Ok(n) = cur.parse::<f64>() { out.push(n); } }
        }
        out
    };

    if nums.len() < 8 { anyhow::bail!("chromaticities attribute not found or incomplete"); }

    let rx = nums[0]; let ry = nums[1];
    let gx = nums[2]; let gy = nums[3];
    let bx = nums[4]; let by = nums[5];
    let wx = nums[6]; let wy = nums[7];

    let m_src = rgb_to_xyz_from_primaries(rx, ry, gx, gy, bx, by, wx, wy);
    // Adaptacja Bradford do D65
    let m_adapt = bradford_adaptation_matrix((wx, wy), (0.3127, 0.3290));
    let m_xyz_to_srgb = xyz_to_srgb_matrix();
    let m = mul3x3(m_xyz_to_srgb, mul3x3(m_adapt, m_src));
    Ok(m)
}

fn rgb_to_xyz_from_primaries(rx: f64, ry: f64, gx: f64, gy: f64, bx: f64, by: f64, wx: f64, wy: f64) -> [[f32; 3]; 3] {
    let rz = 1.0 - rx - ry; let gz = 1.0 - gx - gy; let bz = 1.0 - bx - by;
    let r = [rx/ry, 1.0, rz/ry];
    let g = [gx/gy, 1.0, gz/gy];
    let b = [bx/by, 1.0, bz/by];
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let wz = 1.0 - wx - wy; let w = [wx/wy, 1.0, wz/wy];
    let s = solve3(m, w);
    [
        [ (m[0][0]*s[0]) as f32, (m[0][1]*s[1]) as f32, (m[0][2]*s[2]) as f32 ],
        [ (m[1][0]*s[0]) as f32, (m[1][1]*s[1]) as f32, (m[1][2]*s[2]) as f32 ],
        [ (m[2][0]*s[0]) as f32, (m[2][1]*s[1]) as f32, (m[2][2]*s[2]) as f32 ],
    ]
}

fn xyz_to_srgb_matrix() -> [[f32; 3]; 3] {
    [
        [ 3.2404542, -1.5371385, -0.4985314 ],
        [ -0.9692660, 1.8760108, 0.0415560 ],
        [ 0.0556434, -0.2040259, 1.0572252 ],
    ]
}

fn bradford_adaptation_matrix(src_xy: (f64, f64), dst_xy: (f64, f64)) -> [[f32;3];3] {
    // Bradford cone response matrix and its inverse (f64 dla dokładności)
    let m = [
        [ 0.8951_f64,  0.2664, -0.1614],
        [-0.7502,      1.7135,  0.0367],
        [ 0.0389,     -0.0685,  1.0296],
    ];
    let m_inv = [
        [ 0.9869929, -0.1470543, 0.1599627],
        [ 0.4323053,  0.5183603, 0.0492912],
        [-0.0085287,  0.0400428, 0.9684867],
    ];

    let src_xyz = xy_to_xyz(src_xy.0, src_xy.1);
    let dst_xyz = xy_to_xyz(dst_xy.0, dst_xy.1);
    let src_lms = mul3x1(m, src_xyz);
    let dst_lms = mul3x1(m, dst_xyz);
    let scale = [dst_lms[0]/src_lms[0], dst_lms[1]/src_lms[1], dst_lms[2]/src_lms[2]];
    let s = [
        [scale[0], 0.0, 0.0],
        [0.0, scale[1], 0.0],
        [0.0, 0.0, scale[2]],
    ];
    let ms = mul3x3_f64(s, m);
    let tmp = mul3x3_f64(m_inv, ms);
    [
        [tmp[0][0] as f32, tmp[0][1] as f32, tmp[0][2] as f32],
        [tmp[1][0] as f32, tmp[1][1] as f32, tmp[1][2] as f32],
        [tmp[2][0] as f32, tmp[2][1] as f32, tmp[2][2] as f32],
    ]
}

fn xy_to_xyz(x: f64, y: f64) -> [f64;3] {
    let z = 1.0 - x - y;
    [x/y, 1.0, z/y]
}

fn mul3x3(a: [[f32;3];3], b: [[f32;3];3]) -> [[f32;3];3] {
    let mut m = [[0.0f32;3];3];
    for i in 0..3 { for j in 0..3 { m[i][j] = a[i][0]*b[0][j] + a[i][1]*b[1][j] + a[i][2]*b[2][j]; } }
    m
}

fn mul3x3_f64(a: [[f64;3];3], b: [[f64;3];3]) -> [[f64;3];3] {
    let mut m = [[0.0f64;3];3];
    for i in 0..3 { for j in 0..3 { m[i][j] = a[i][0]*b[0][j] + a[i][1]*b[1][j] + a[i][2]*b[2][j]; } }
    m
}

fn mul3x1(a: [[f64;3];3], v: [f64;3]) -> [f64;3] {
    [
        a[0][0]*v[0] + a[0][1]*v[1] + a[0][2]*v[2],
        a[1][0]*v[0] + a[1][1]*v[1] + a[1][2]*v[2],
        a[2][0]*v[0] + a[2][1]*v[1] + a[2][2]*v[2],
    ]
}

fn solve3(m: [[f64;3];3], w: [f64;3]) -> [f64;3] {
    let det =
        m[0][0]*(m[1][1]*m[2][2]-m[1][2]*m[2][1]) -
        m[0][1]*(m[1][0]*m[2][2]-m[1][2]*m[2][0]) +
        m[0][2]*(m[1][0]*m[2][1]-m[1][1]*m[2][0]);
    if det.abs() < 1e-12 { return [1.0,1.0,1.0]; }
    let inv_det = 1.0/det;
    let inv = [
        [ (m[1][1]*m[2][2]-m[1][2]*m[2][1])*inv_det, (m[0][2]*m[2][1]-m[0][1]*m[2][2])*inv_det, (m[0][1]*m[1][2]-m[0][2]*m[1][1])*inv_det ],
        [ (m[1][2]*m[2][0]-m[1][0]*m[2][2])*inv_det, (m[0][0]*m[2][2]-m[0][2]*m[2][0])*inv_det, (m[0][2]*m[1][0]-m[0][0]*m[1][2])*inv_det ],
        [ (m[1][0]*m[2][1]-m[1][1]*m[2][0])*inv_det, (m[0][1]*m[2][0]-m[0][0]*m[2][1])*inv_det, (m[0][0]*m[1][1]-m[0][1]*m[1][0])*inv_det ],
    ];
    let s0 = inv[0][0]*w[0] + inv[0][1]*w[1] + inv[0][2]*w[2];
    let s1 = inv[1][0]*w[0] + inv[1][1]*w[1] + inv[1][2]*w[2];
    let s2 = inv[2][0]*w[0] + inv[2][1]*w[1] + inv[2][2]*w[2];
    [s0,s1,s2]
}

