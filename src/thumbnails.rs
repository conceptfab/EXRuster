use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use exr::prelude as exr;
use rayon::prelude::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::image_processing::process_pixel;
use std::cmp::Ordering;

/// Zwięzła reprezentacja miniaturki EXR do wyświetlenia w UI
pub struct ExrThumbnailInfo {
    pub path: PathBuf,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub num_layers: usize,
    pub image: Image,
}

/// Główny interfejs: generuje miniaturki dla wszystkich plików .exr w katalogu (bez rekursji).
/// - Przetwarzanie odbywa się równolegle (Rayon)
/// - Miniaturki powstają z kompozytu kanałów R, G, B z "najlepszej" warstwy (zob. `select_best_layer_name`)
/// - Transformacje zgodne z podglądem (ACES + gamma) przez `process_pixel`, z przekazanymi parametrami
pub fn generate_exr_thumbnails_in_dir(
    directory: &Path,
    max_thumb_size: u32,
    exposure: f32,
    gamma: f32,
) -> anyhow::Result<Vec<ExrThumbnailInfo>> {
    let files = list_exr_files(directory)?;

    // 1) Równolegle generuj dane miniaturek w typie bezpiecznym dla wątków (bez slint::Image)
    let works: Vec<ExrThumbWork> = files
        .par_iter()
        .filter_map(|path| match generate_single_exr_thumbnail_work(path, max_thumb_size, exposure, gamma) {
            Ok(work) => Some(work),
            Err(_e) => None, // tu można logować błąd
        })
        .collect();

    // 2) Na głównym wątku skonstruuj slint::Image (nie jest Send)
    let mut thumbnails: Vec<ExrThumbnailInfo> = works
        .into_iter()
        .map(|w| {
            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w.width, w.height);
            let slice = buffer.make_mut_slice();
            for (dst, chunk) in slice.iter_mut().zip(w.pixels.chunks_exact(4)) {
                *dst = Rgba8Pixel { r: chunk[0], g: chunk[1], b: chunk[2], a: chunk[3] };
            }

            ExrThumbnailInfo {
                path: w.path,
                file_name: w.file_name,
                file_size_bytes: w.file_size_bytes,
                num_layers: w.num_layers,
                image: Image::from_rgba8(buffer),
            }
        })
        .collect();

    // Sortowanie z priorytetem wiodących podkreślników, potem naturalne porównanie
    thumbnails.sort_by(|a, b| natural_cmp_with_priority(&a.file_name, &b.file_name));

    Ok(thumbnails)
}

/// Naturalne porównanie napisów (case-insensitive, sekwencje cyfr porównywane numerycznie)
pub fn natural_cmp_str(a: &str, b: &str) -> Ordering {
    fn take_number<I>(it: &mut std::iter::Peekable<I>) -> (u128, usize)
    where
        I: Iterator<Item = char>,
    {
        let mut value: u128 = 0;
        let mut len: usize = 0;
        while let Some(&ch) = it.peek() {
            if ch.is_ascii_digit() {
                it.next();
                value = value.saturating_mul(10).saturating_add((ch as u32 - '0' as u32) as u128);
                len += 1;
            } else {
                break;
            }
        }
        (value, len)
    }

    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();

    loop {
        let ca = ia.peek().copied();
        let cb = ib.peek().copied();

        match (ca, cb) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(cha), Some(chb)) => {
                let da = cha.is_ascii_digit();
                let db = chb.is_ascii_digit();
                if da && db {
                    let (va, la) = take_number(&mut ia);
                    let (vb, lb) = take_number(&mut ib);
                    match va.cmp(&vb) {
                        Ordering::Equal => {
                            // Równe numerycznie — krótsza sekwencja cyfr najpierw (np. 1 < 01)
                            match la.cmp(&lb) {
                                Ordering::Equal => continue,
                                ord => return ord,
                            }
                        }
                        ord => return ord,
                    }
                } else {
                    // Porównanie case-insensitive pojedynczych znaków
                    let la = ia.next().unwrap().to_ascii_lowercase();
                    let lb = ib.next().unwrap().to_ascii_lowercase();
                    match la.cmp(&lb) {
                        Ordering::Equal => continue,
                        ord => return ord,
                    }
                }
            }
        }
    }
}

#[inline]
fn count_leading_underscores(s: &str) -> usize {
    let mut count = 0usize;
    for ch in s.chars() {
        if ch == '_' { count += 1; } else { break; }
    }
    count
}

/// Najpierw więcej wiodących '_' => wyżej, a przy remisie naturalne porównanie całej nazwy
pub fn natural_cmp_with_priority(a: &str, b: &str) -> Ordering {
    let ua = count_leading_underscores(a);
    let ub = count_leading_underscores(b);
    match ub.cmp(&ua) { // więcej '_' ma pierwszeństwo
        Ordering::Equal => natural_cmp_str(a, b),
        ord => ord,
    }
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
    max_thumb_size: u32,
    exposure: f32,
    gamma: f32,
) -> anyhow::Result<ExrThumbWork> {
    // Wczytaj płaskie warstwy (FlatSamples) do łatwego indeksowania pikseli
    let image = exr::read_all_flat_layers_from_file(path)
        .with_context(|| format!("Błąd odczytu EXR: {}", path.display()))?;

    // Lokalne helpery korzystające z wyinferowanego typu `image`
    let mut layer_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for layer in image.layer_data.iter() {
        let base_attr = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
        for ch in &layer.channel_data.list {
            let full = ch.name.to_string();
            let (lname, _short) = split_layer_and_short(&full, base_attr.as_deref());
            *layer_map.entry(lname).or_insert(0) += 1;
        }
    }
    let mut candidates: Vec<(String, usize)> = layer_map.into_iter().collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    // wybór najlepszej nazwy warstwy
    let mut best_layer_name = String::new();
    let candidate_names: Vec<String> = candidates.iter().map(|(n, _)| n.clone()).collect();

    // Closure: czy dana warstwa ma RGB w tym obrazie
    let has_rgb_in = |wanted_layer: &str| -> bool {
        let wanted_lower = wanted_layer.to_lowercase();
        for layer in image.layer_data.iter() {
            let base_attr = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
            let mut group_found = false;
            let mut r = false; let mut g = false; let mut b = false;
            for ch in &layer.channel_data.list {
                let full = ch.name.to_string();
                let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());
                if group_matches(&lname, &wanted_lower) {
                    group_found = true;
                    let su = short.to_ascii_uppercase();
                    if su == "R" || su == "RED" || su.starts_with('R') { r = true; }
                    if su == "G" || su == "GREEN" || su.starts_with('G') { g = true; }
                    if su == "B" || su == "BLUE" || su.starts_with('B') { b = true; }
                }
            }
            if group_found && r && g && b { return true; }
        }
        false
    };

    // 1) warstwa pusta z RGB
    if candidate_names.iter().any(|n| n.is_empty()) {
        if has_rgb_in("") { best_layer_name = String::new(); }
    }
    // 2) nazwy priorytetowe
    if best_layer_name.is_empty() {
        let priority = ["beauty", "Beauty", "RGBA", "rgba", "default", "Default", "combined", "Combined"];
        for name in priority {
            if let Some(found) = candidate_names.iter().find(|n| n.to_lowercase().contains(&name.to_lowercase())) {
                best_layer_name = found.clone();
                break;
            }
        }
    }
    // 3) pierwsza z RGB
    if best_layer_name.is_empty() {
        for n in &candidate_names {
            if has_rgb_in(n) { best_layer_name = n.clone(); break; }
        }
    }
    // 4) fallback
    if best_layer_name.is_empty() { best_layer_name = candidate_names.first().cloned().unwrap_or_default(); }

    // znajdź grupę RGB
    let mut layer_ref_idx: Option<usize> = None;
    let mut r_idx: Option<usize> = None;
    let mut g_idx: Option<usize> = None;
    let mut b_idx: Option<usize> = None;
    let mut a_idx: Option<usize> = None;
    let mut group_indices: Vec<usize> = Vec::new();
    let wanted_lower = best_layer_name.to_lowercase();
    for (li, layer) in image.layer_data.iter().enumerate() {
        let base_attr = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
        let mut group_found = false;
        for (idx, ch) in layer.channel_data.list.iter().enumerate() {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());
            if group_matches(&lname, &wanted_lower) {
                group_found = true;
                group_indices.push(idx);
                let su = short.to_ascii_uppercase();
                match su.as_str() {
                    "R" | "RED" => r_idx = Some(idx),
                    "G" | "GREEN" => g_idx = Some(idx),
                    "B" | "BLUE" => b_idx = Some(idx),
                    "A" | "ALPHA" => a_idx = Some(idx),
                    _ => {
                        if r_idx.is_none() && su.starts_with('R') { r_idx = Some(idx); }
                        if g_idx.is_none() && su.starts_with('G') { g_idx = Some(idx); }
                        if b_idx.is_none() && su.starts_with('B') { b_idx = Some(idx); }
                    }
                }
            }
        }
        if group_found {
            layer_ref_idx = Some(li);
            break;
        }
    }

    let layer_ref = match layer_ref_idx { Some(i) => &image.layer_data[i], None => anyhow::bail!("Nie znaleziono grupy RGB dla warstwy '{}'.", best_layer_name) };

    if r_idx.is_none() { r_idx = group_indices.get(0).cloned(); }
    if g_idx.is_none() { g_idx = group_indices.get(1).cloned().or(r_idx); }
    if b_idx.is_none() { b_idx = group_indices.get(2).cloned().or(g_idx).or(r_idx); }

    let (ri, gi, bi) = match (r_idx, g_idx, b_idx) { (Some(ri), Some(gi), Some(bi)) => (ri, gi, bi), _ => anyhow::bail!("Brak kanałów RGB w warstwie '{}'.", best_layer_name) };

    let width = layer_ref.size.width() as u32;
    let height = layer_ref.size.height() as u32;

    // Oblicz rozmiar miniaturki
    let scale = (max_thumb_size as f32 / width.max(height) as f32).min(1.0);
    let thumb_w = (width as f32 * scale) as u32;
    let thumb_h = (height as f32 * scale) as u32;

    // Bufor wyjściowy miniaturki (RGBA8)
    let mut pixels: Vec<u8> = vec![0; (thumb_w as usize) * (thumb_h as usize) * 4];

    // Samplowanie nearest-neighbor z mapowaniem procesem jak w preview (ACES + gamma)
    pixels
        .par_chunks_mut(4)
        .enumerate()
        .for_each(|(i, out)| {
            let x = (i as u32) % thumb_w;
            let y = (i as u32) / thumb_w;

            let src_x = ((x as f32 / scale) as u32).min(width.saturating_sub(1));
            let src_y = ((y as f32 / scale) as u32).min(height.saturating_sub(1));
            let src_idx = (src_y as usize) * (width as usize) + (src_x as usize);

            let r = layer_ref.channel_data.list[ri].sample_data.value_by_flat_index(src_idx).to_f32();
            let g = layer_ref.channel_data.list[gi].sample_data.value_by_flat_index(src_idx).to_f32();
            let b = layer_ref.channel_data.list[bi].sample_data.value_by_flat_index(src_idx).to_f32();
            let a = a_idx.map(|ci| layer_ref.channel_data.list[ci].sample_data.value_by_flat_index(src_idx).to_f32()).unwrap_or(1.0);

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
        num_layers: candidates.len(),
        pixels,
    })
}

#[inline]
fn group_matches(lname: &str, wanted_lower: &str) -> bool {
    let lname_lower = lname.to_lowercase();
    if wanted_lower.is_empty() && lname_lower.is_empty() {
        true
    } else if wanted_lower.is_empty() || lname_lower.is_empty() {
        false
    } else {
        // preferowana zgodność: exact, potem prefix, na końcu contains
        lname_lower == wanted_lower
            || lname_lower.starts_with(&wanted_lower)
            || lname_lower.contains(&wanted_lower)
    }
}

#[inline]
fn split_layer_and_short(full: &str, base_attr: Option<&str>) -> (String, String) {
    if let Some(base) = base_attr {
        let short = full.rsplit('.').next().unwrap_or(full).to_string();
        (base.to_string(), short)
    } else if let Some(p) = full.rfind('.') {
        (full[..p].to_string(), full[p + 1..].to_string())
    } else {
        ("".to_string(), full.to_string())
    }
}
