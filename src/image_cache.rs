use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use exr::prelude as exr;
use std::path::PathBuf;
use crate::image_processing::process_pixel;
use rayon::prelude::*;
use std::collections::HashMap;
use crate::utils::split_layer_and_short;
use crate::progress::ProgressSink;
use crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer;
use glam::{Mat3, Vec3};

/// Zwraca kanoniczny skrót kanału na podstawie aliasów/nazw przyjaznych.
/// Np. "red"/"Red"/"RED"/"R"/"R8" → "R"; analogicznie dla G/B/A.
#[inline]
pub(crate) fn channel_alias_to_short(input: &str) -> String {
    let trimmed = input.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper == "R" || upper.starts_with("RED") { return "R".to_string(); }
    if upper == "G" || upper.starts_with("GREEN") { return "G".to_string(); }
    if upper == "B" || upper.starts_with("BLUE") { return "B".to_string(); }
    if upper == "A" || upper.starts_with("ALPHA") { return "A".to_string(); }
    trimmed.to_string()
}

#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub name: String,
    pub channels: Vec<ChannelInfo>,
}

// split_layer_and_short przeniesione do utils

#[derive(Clone, Debug)]
pub struct ChannelInfo {
    pub name: String,           // krótka nazwa (po ostatniej kropce)
}

#[derive(Clone, Debug)]
pub struct LayerChannels {
    pub layer_name: String,
    pub width: u32,
    pub height: u32,
    // Klucze: krótkie nazwy kanałów z pliku (np. "R", "G", "B", "A", "Z", itp.)
    pub channels: HashMap<String, Vec<f32>>,
}

pub struct ImageCache {
    pub raw_pixels: Vec<(f32, f32, f32, f32)>,
    pub width: u32,
    pub height: u32,
    pub layers_info: Vec<LayerInfo>,
    pub current_layer_name: String,
    // Opcjonalna macierz konwersji z przestrzeni primaries pliku do sRGB (linear RGB)
    color_matrix_rgb_to_srgb: Option<Mat3>,
    // Cache wszystkich kanałów dla bieżącej warstwy aby uniknąć I/O przy przełączaniu
    pub current_layer_channels: Option<LayerChannels>,
}

impl ImageCache {
    pub fn new(path: &PathBuf) -> anyhow::Result<Self> {
        // Najpierw wyciągnij informacje o warstwach, wybierz najlepszą i wczytaj ją jako startowy podgląd
        let layers_info = extract_layers_info(path)?;
        let best_layer = find_best_layer(&layers_info);
        let layer_channels = load_all_channels_for_layer(path, &best_layer, None)?;

        let raw_pixels = compose_composite_from_channels(&layer_channels);
        let width = layer_channels.width;
        let height = layer_channels.height;
        let current_layer_name = layer_channels.layer_name.clone();

        // Spróbuj wyliczyć macierz konwersji primaries → sRGB na podstawie atrybutu chromaticities (dla wybranej warstwy/partu)
        let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, &best_layer).ok();

        Ok(ImageCache {
            raw_pixels,
            width,
            height,
            layers_info,
            current_layer_name,
            color_matrix_rgb_to_srgb,
            current_layer_channels: Some(layer_channels),
        })
    }
    
    pub fn load_layer(&mut self, path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
        // Jednorazowo wczytaj wszystkie kanały wybranej warstwy i zbuduj kompozyt
        let layer_channels = load_all_channels_for_layer(path, layer_name, progress)?;

        self.width = layer_channels.width;
        self.height = layer_channels.height;
        self.current_layer_name = layer_channels.layer_name.clone();
        self.raw_pixels = compose_composite_from_channels(&layer_channels);
        self.current_layer_channels = Some(layer_channels);
        // Reoblicz macierz primaries→sRGB na wypadek, gdyby warstwa/part zmieniały chromaticities
        self.color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name).ok();

        Ok(())
    }

    #[inline]
    pub fn color_matrix(&self) -> Option<Mat3> { self.color_matrix_rgb_to_srgb }
    
    pub fn process_to_image(&self, exposure: f32, gamma: f32) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let slice = buffer.make_mut_slice();
        
        // Użycie większych chunków dla lepszej wydajności
        let chunk_size = if self.raw_pixels.len() > 1_000_000 { 
            4096 
        } else { 
            2048 
        };
        
        // Przetwarzanie z lepszą lokalność pamięci
        let m = self.color_matrix_rgb_to_srgb;
        self.raw_pixels
            .par_chunks(chunk_size)
            .zip(slice.par_chunks_mut(chunk_size))
            .for_each(|(input_chunk, output_chunk)| {
                for (input_pixel, output_pixel) in input_chunk.iter().zip(output_chunk.iter_mut()) {
                    let (mut r, mut g, mut b, a) = *input_pixel;
                    if let Some(mat) = m {
                        let v = mat * Vec3::new(r, g, b);
                        r = v.x; g = v.y; b = v.z;
                    }
                    *output_pixel = process_pixel(r, g, b, a, exposure, gamma);
                }
            });
        
        Image::from_rgba8(buffer)
    }

    pub fn process_to_composite(&self, exposure: f32, gamma: f32, lighting_rgb: bool) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let slice = buffer.make_mut_slice();

        // Przetwarzanie pikseli: jeśli lighting_rgb=true (lub ogólnie warstwa kolorowa), zachowujemy normalne RGB;
        // w przeciwnym razie generujemy grayscale jako sumę R+G+B (po tone map i gamma).
        let m = self.color_matrix_rgb_to_srgb;
        self.raw_pixels
            .par_iter()
            .zip(slice.par_iter_mut())
            .for_each(|(&(r0, g0, b0, a), out)| {
                let (mut r, mut g, mut b) = (r0, g0, b0);
                if let Some(mat) = m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                if lighting_rgb {
                    *out = process_pixel(r, g, b, a, exposure, gamma);
                } else {
                    let px = process_pixel(r, g, b, a, exposure, gamma);
                    let rr = (px.r as f32) / 255.0;
                    let gg = (px.g as f32) / 255.0;
                    let bb = (px.b as f32) / 255.0;
                    let gray = (rr.max(gg).max(bb)).clamp(0.0, 1.0);
                    let g8 = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
                    *out = Rgba8Pixel { r: g8, g: g8, b: g8, a: px.a };
                }
            });

        Image::from_rgba8(buffer)
    }
    // Nowa metoda dla preview (szybsze przetwarzanie małego obrazka)
    pub fn process_to_thumbnail(&self, exposure: f32, gamma: f32, max_size: u32) -> Image {
        let scale = (max_size as f32 / self.width.max(self.height) as f32).min(1.0);
        let thumb_width = (self.width as f32 * scale) as u32;
        let thumb_height = (self.height as f32 * scale) as u32;
        
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(thumb_width, thumb_height);
        let slice = buffer.make_mut_slice();
        
        // Proste nearest neighbor sampling dla szybkości
        let m = self.color_matrix_rgb_to_srgb;
        slice.par_iter_mut().enumerate().for_each(|(i, pixel)| {
            let x = (i as u32) % thumb_width;
            let y = (i as u32) / thumb_width;
            
            let src_x = ((x as f32 / scale) as u32).min(self.width.saturating_sub(1));
            let src_y = ((y as f32 / scale) as u32).min(self.height.saturating_sub(1));
            let src_idx = (src_y as usize) * (self.width as usize) + (src_x as usize);

            let (mut r, mut g, mut b, a) = self.raw_pixels[src_idx];
            if let Some(mat) = m {
                let v = mat * Vec3::new(r, g, b);
                r = v.x; g = v.y; b = v.z;
            }
            *pixel = process_pixel(r, g, b, a, exposure, gamma);
        });
        
        Image::from_rgba8(buffer)
    }
}



pub(crate) fn extract_layers_info(path: &PathBuf) -> anyhow::Result<Vec<LayerInfo>> {
        // Odczytaj jedynie meta-dane (nagłówki) bez pikseli
        let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/false)?;

        // Mapowanie: nazwa_warstwy -> kanały
        let mut layer_map: HashMap<String, Vec<ChannelInfo>> = HashMap::new();
        // Kolejność pierwszego wystąpienia nazw warstw do stabilnego porządku w UI
        let mut layer_order: Vec<String> = Vec::new();

        for header in meta.headers.iter() {
            // Preferuj nazwę z atrybutu warstwy; jeśli brak, kanały mogą być w formacie "warstwa.kanał"
            let base_layer_name: Option<String> = header
                .own_attributes
                .layer_name
                .as_ref()
                .map(|t| t.to_string());

            for ch in header.channels.list.iter() {
                let full_channel_name = ch.name.to_string();
                let (layer_name_effective, short_channel_name) =
                    split_layer_and_short(&full_channel_name, base_layer_name.as_deref());

                let entry = layer_map.entry(layer_name_effective.clone()).or_insert_with(|| {
                    layer_order.push(layer_name_effective.clone());
                    Vec::new()
                });

                entry.push(ChannelInfo { name: short_channel_name });
            }
        }

        // Zbuduj listę warstw w kolejności pierwszego wystąpienia
        let mut layers: Vec<LayerInfo> = Vec::with_capacity(layer_map.len());
        for name in layer_order {
            if let Some(channels) = layer_map.remove(&name) {
                layers.push(LayerInfo { name, channels });
            }
        }

        Ok(layers)
}

pub(crate) fn find_best_layer(layers_info: &[LayerInfo]) -> String {
    // Plan A: Sprawdź czy istnieje warstwa pusta ("") z kanałami R, G, B
    // Ta warstwa zawiera główne kanały obrazu bez prefiksu
    if let Some(layer) = layers_info.iter().find(|l| l.name.is_empty()) {
        let mut has_r = false;
        let mut has_g = false;
        let mut has_b = false;
        for ch in &layer.channels {
            let n = ch.name.trim().to_ascii_uppercase();
            if n == "R" { has_r = true; }
            else if n == "G" { has_g = true; }
            else if n == "B" { has_b = true; }
        }
        if has_r && has_g && has_b {
            return layer.name.clone();
        }
    }
    
    // Plan B: Priorytetowa lista nazw warstw (zgodnie z mini.md)
    let priority_names = ["beauty", "Beauty", "RGBA", "rgba", "default", "Default", "combined", "Combined"];
    
    // Sprawdź czy istnieje warstwa o priorytetowej nazwie
    for priority_name in &priority_names {
        if let Some(layer) = layers_info.iter().find(|l| l.name.to_lowercase().contains(&priority_name.to_lowercase())) {
            return layer.name.clone();
        }
    }
    
    // Plan C: Znajdź pierwszą warstwę z kanałami R, G, B (porównanie dokładne krótkich nazw)
    for layer in layers_info {
        let mut has_r = false;
        let mut has_g = false;
        let mut has_b = false;
        for ch in &layer.channels {
            let n = ch.name.trim().to_ascii_uppercase();
            if n == "R" { has_r = true; }
            else if n == "G" { has_g = true; }
            else if n == "B" { has_b = true; }
        }
        if has_r && has_g && has_b {
            return layer.name.clone();
        }
    }
    
    // Plan D (ostateczność): Pierwsza warstwa
    layers_info.first()
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "Layer 1".to_string())
}

pub(crate) fn load_specific_layer(path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<(Vec<(f32, f32, f32, f32)>, u32, u32, String)> {
    // Szybka ścieżka: jeżeli prosimy o bazową/typową warstwę RGBA, użyj gotowej funkcji czytającej pierwszą RGBA
    // Dotyczy częstych nazw: "", "beauty", "rgba", "default", "combined"
    let lname = layer_name.trim();
    let lname_lower = lname.to_ascii_lowercase();
    let is_typical_rgba = lname.is_empty()
        || lname_lower == "beauty"
        || lname_lower == "rgba"
        || lname_lower == "default"
        || lname_lower == "combined";

    if is_typical_rgba {
        if let Some(p) = progress { p.set(0.08, Some("Fast path: RGBA layer")); }
        if let Ok((pixels, w, h, _)) = load_first_rgba_layer(path) {
            if let Some(p) = progress { p.set(0.92, Some("Fast path done")); }
            return Ok((pixels, w, h, layer_name.to_string()));
        }
    }

    // Standardowa ścieżka: wczytaj płaskie warstwy (bez mip-map), aby uzyskać FlatSamples
    if let Some(p) = progress { p.set(0.1, Some("Reading layer data...")); }
    let any_image = exr::read_all_flat_layers_from_file(path)?;

    // Szukaj grupy kanałów odpowiadającej nazwie warstwy (spójne z extract_layers_info)
    let wanted_lower = layer_name.to_lowercase();
    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);

        let base_attr: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());

        // Indeksy R/G/B/A w grupie, jeśli dopasowano, oraz lista wszystkich indeksów w grupie
        let mut r_idx: Option<usize> = None;
        let mut g_idx: Option<usize> = None;
        let mut b_idx: Option<usize> = None;
        let mut a_idx: Option<usize> = None;
        let mut group_found = false;
        let mut group_indices: Vec<usize> = Vec::with_capacity(layer.channel_data.list.len());

        let name_matches = |lname: &str| -> bool {
            let lname_lower = lname.to_lowercase();
            if wanted_lower.is_empty() && lname_lower.is_empty() {
                true
            } else if wanted_lower.is_empty() || lname_lower.is_empty() {
                false
            } else {
                lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
            }
        };

        for (idx, ch) in layer.channel_data.list.iter().enumerate() {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());

            if name_matches(&lname) {
                group_found = true;
                group_indices.push(idx);
                let su = short.to_ascii_uppercase();
                match su.as_str() {
                    "R" | "RED" => r_idx = Some(idx),
                    "G" | "GREEN" => g_idx = Some(idx),
                    "B" | "BLUE" => b_idx = Some(idx),
                    "A" | "ALPHA" => a_idx = Some(idx),
                    _ => {
                        // Dodatkowe heurystyki: nazwy zaczynające się od R/G/B
                        if r_idx.is_none() && su.starts_with('R') { r_idx = Some(idx); }
                        else if g_idx.is_none() && su.starts_with('G') { g_idx = Some(idx); }
                        else if b_idx.is_none() && su.starts_with('B') { b_idx = Some(idx); }
                    }
                }
            }
        }

        if group_found {
            if let Some(p) = progress { p.set(0.4, Some("Processing pixels...")); }
            // Zapewnij 3 kanały: jeśli brakuje, uzupełnij z listy kanałów grupy lub duplikuj poprzedni
            if r_idx.is_none() {
                r_idx = group_indices.get(0).cloned();
            }
            if g_idx.is_none() {
                g_idx = group_indices.get(1).cloned().or(r_idx);
            }
            if b_idx.is_none() {
                b_idx = group_indices.get(2).cloned().or(g_idx).or(r_idx);
            }

            // Jeżeli nadal coś jest None (pusta grupa), zgłoś błąd
            let (ri, gi, bi) = match (r_idx, g_idx, b_idx) {
                (Some(ri), Some(gi), Some(bi)) => (ri, gi, bi),
                _ => anyhow::bail!("Warstwa '{}' nie zawiera kanałów do kompozytu", layer_name),
            };

            let mut out: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(pixel_count);
            for i in 0..pixel_count {
                let r = layer.channel_data.list[ri].sample_data.value_by_flat_index(i).to_f32();
                let g = layer.channel_data.list[gi].sample_data.value_by_flat_index(i).to_f32();
                let b = layer.channel_data.list[bi].sample_data.value_by_flat_index(i).to_f32();
                let a = a_idx.map(|ci| layer.channel_data.list[ci].sample_data.value_by_flat_index(i).to_f32()).unwrap_or(1.0);
                out.push((r, g, b, a));
            }
            if let Some(p) = progress { p.set(0.9, Some("Finalizing...")); }
            // Zwracamy żądaną nazwę jako aktualną, aby była spójna z UI
            return Ok((out, width, height, layer_name.to_string()));
        }
    }

    // Jeśli nie znaleziono warstwy, fallback do pierwszej RGBA
    let (pixels, width, height, _) = load_first_rgba_layer(path)?;
    Ok((pixels, width, height, layer_name.to_string()))
}

fn load_first_rgba_layer(path: &PathBuf) -> anyhow::Result<(Vec<(f32, f32, f32, f32)>, u32, u32, String)> {
    use std::convert::Infallible;
    use std::cell::RefCell;
    use std::rc::Rc;
    
    let pixels = Rc::new(RefCell::new(Vec::new()));
    let dimensions = Rc::new(RefCell::new((0u32, 0u32)));
    
    let pixels_clone1 = pixels.clone();
    let pixels_clone2 = pixels.clone();
    let dimensions_clone = dimensions.clone();
    
    exr::read_first_rgba_layer_from_file(
        path,
        move |resolution, _| -> Result<(), Infallible> {
            let width = resolution.width() as u32;
            let height = resolution.height() as u32;
            *dimensions_clone.borrow_mut() = (width, height);
            pixels_clone1.borrow_mut().reserve_exact((width * height) as usize);
            Ok(())
        },
        move |_, _, (r, g, b, a): (f32, f32, f32, f32)| {
            pixels_clone2.borrow_mut().push((r, g, b, a));
        },
    )?;

    let (width, height) = *dimensions.borrow();
    let raw_pixels = match Rc::try_unwrap(pixels) {
        Ok(cell) => cell.into_inner(),
        Err(rc) => rc.borrow().clone(),
    };
    
    Ok((raw_pixels, width, height, "First RGBA Layer".to_string()))
}

// Funkcja usunięta - nie jest używana w uproszczonej implementacji

// usunięto rozbudowane wykrywanie rodzaju kanału — UI pokazuje teraz realne kanały bez grupowania

impl ImageCache {
    /// Wczytuje jeden wskazany kanał z danej warstwy i zapisuje go jako grayscale (R=G=B=val, A=1)
    pub fn load_channel(&mut self, path: &PathBuf, layer_name: &str, channel_short: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
        // Zapewnij, że cache kanałów dla żądanej warstwy jest dostępny
        let need_reload = self.current_layer_channels.as_ref().map(|lc| lc.layer_name.to_lowercase() != layer_name.to_lowercase()).unwrap_or(true);
        if need_reload {
            // Załaduj wskazaną warstwę (zapełni current_layer_channels oraz ustawi kompozyt)
            self.load_layer(path, layer_name, progress)?;
        }

        // Teraz mamy current_layer_channels dla właściwej warstwy
        let layer_cache = self.current_layer_channels.as_ref().ok_or_else(|| anyhow::anyhow!("Brak cache kanałów dla warstwy"))?;

        let find_channel_vec = |wanted: &str| -> Option<&Vec<f32>> {
            // 1) dokładne dopasowanie (case-sensitive)
            if let Some(v) = layer_cache.channels.get(wanted) { return Some(v); }
            // 2) case-insensitive
            let wanted_lower = wanted.to_lowercase();
            if let Some((_, v)) = layer_cache.channels.iter().find(|(k, _)| k.to_lowercase() == wanted_lower) { return Some(v); }
            // 3) według kanonicznego skrótu R/G/B/A
            let wanted_canon = channel_alias_to_short(wanted).to_ascii_uppercase();
            if let Some((_, v)) = layer_cache.channels.iter().find(|(k, _)| channel_alias_to_short(k).to_ascii_uppercase() == wanted_canon) { return Some(v); }
            None
        };

        // Specjalne traktowanie Depth
        let wanted_upper = channel_short.to_ascii_uppercase();
        let is_depth = wanted_upper == "Z" || wanted_upper.contains("DEPTH");

        let channel_vec = if is_depth {
            // Preferuj dokładnie "Z"; w razie braku wybierz kanał zawierający "DEPTH" albo "DISTANCE"
            find_channel_vec("Z")
                .or_else(|| layer_cache.channels.iter().find(|(k, _)| k.to_ascii_uppercase().contains("DEPTH") || k.to_ascii_uppercase() == "DISTANCE").map(|(_, v)| v))
        } else {
            find_channel_vec(channel_short)
        };

        let channel_vec = channel_vec.ok_or_else(|| anyhow::anyhow!(format!("Nie znaleziono kanału '{}' w warstwie '{}'", channel_short, layer_cache.layer_name)))?;

        let pixel_count = (layer_cache.width as usize) * (layer_cache.height as usize);
        debug_assert_eq!(channel_vec.len(), pixel_count, "Długość kanału nie zgadza się z rozmiarem obrazu");

        let mut out: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(pixel_count);
        for &v in channel_vec.iter() {
            out.push((v, v, v, 1.0));
        }

        self.raw_pixels = out;
        self.width = layer_cache.width;
        self.height = layer_cache.height;
        self.current_layer_name = layer_cache.layer_name.clone();
        Ok(())
    }

    /// Specjalne renderowanie głębi: auto-normalizacja percentylowa + opcjonalne odwrócenie
    pub fn process_depth_image_with_progress(&self, invert: bool, progress: Option<&dyn ProgressSink>) -> Image {
        if let Some(p) = progress { p.start_indeterminate(Some("Processing depth data...")); }
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let slice = buffer.make_mut_slice();

        // Wyciągnij z surowych pikseli jeden kanał (zakładamy, że R=G=B=val)
        let mut values: Vec<f32> = self.raw_pixels.iter().map(|(r, _g, _b, _a)| *r).collect();
        if values.is_empty() {
            return Image::from_rgba8(buffer);
        }

        // Policz percentyle 1% i 99% (odporne na outliery) w ~O(n)
        use std::cmp::Ordering;
        let len = values.len();
        let p_lo_idx = ((len as f32) * 0.01).floor() as usize;
        let mut p_hi_idx = ((len as f32) * 0.99).ceil() as isize - 1;
        if p_hi_idx < 0 { p_hi_idx = 0; }
        let p_hi_idx = (p_hi_idx as usize).min(len - 1);
        let (_, lo_ref, _) = values.select_nth_unstable_by(p_lo_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let mut lo = *lo_ref;
        let (_, hi_ref, _) = values.select_nth_unstable_by(p_hi_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let mut hi = *hi_ref;
        if let Some(p) = progress { p.set(0.4, Some("Computing percentiles...")); }
        if !lo.is_finite() || !hi.is_finite() || (hi - lo).abs() < 1e-20 {
            // Fallback do min/max jeśli degeneracja lub NaN/Inf
            let mut min_v = f32::INFINITY;
            let mut max_v = f32::NEG_INFINITY;
            for &v in &values {
                let nv = if v.is_finite() { v } else { 0.0 };
                if nv < min_v { min_v = nv; }
                if nv > max_v { max_v = nv; }
            }
            lo = min_v;
            hi = max_v;
        }
        if (hi - lo).abs() < 1e-12 {
            hi = lo + 1.0;
        }

        let map_val = |v: f32| -> u8 {
            let mut t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            if invert { t = 1.0 - t; }
            (t * 255.0).round().clamp(0.0, 255.0) as u8
        };

        if let Some(p) = progress { p.set(0.8, Some("Rendering depth image...")); }
        self.raw_pixels.par_iter().zip(slice.par_iter_mut()).for_each(|(&(r, _g, _b, _a), out)| {
            let g8 = map_val(r);
            *out = Rgba8Pixel { r: g8, g: g8, b: g8, a: 255 };
        });

        if let Some(p) = progress { p.finish(Some("Depth processed")); }
        Image::from_rgba8(buffer)
    }
    // uproszczono API: używaj `process_depth_image_with_progress` bezpośrednio

    // usunięto: specjalny preview Cryptomatte
}

/// Hashuje identyfikator z cryptomatte (f32 bit pattern) do stabilnego koloru w 0..1
// usunięto: hash_id_to_color

/// Buduje kolorowy preview dla warstwy Cryptomatte, łącząc pary (id, coverage)
// usunięto: funkcja preview warstwy Cryptomatte

// (usunięto) stary loader pojedynczego kanału – zastąpiony cachingiem wszystkich kanałów warstwy

// Pomocnicze: wczytuje wszystkie kanały dla wybranej warstwy do pamięci (bez dalszego I/O przy przełączaniu)
pub(crate) fn load_all_channels_for_layer(
    path: &PathBuf,
    layer_name: &str,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<LayerChannels> {
    if let Some(p) = progress { p.start_indeterminate(Some("Reading layer channels...")); }
    let any_image = exr::read_all_flat_layers_from_file(path)?;

    let wanted_lower = layer_name.to_lowercase();

    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);

        let base_attr: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
        let name_matches = |lname: &str| -> bool {
            let lname_lower = lname.to_lowercase();
            if wanted_lower.is_empty() && lname_lower.is_empty() {
                true
            } else if wanted_lower.is_empty() || lname_lower.is_empty() {
                false
            } else {
                lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
            }
        };

        // Sprawdź czy warstwa pasuje nazwą
        let mut matched_indices: Vec<(usize, String)> = Vec::new();
        for (idx, ch) in layer.channel_data.list.iter().enumerate() {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());
            if name_matches(&lname) {
                matched_indices.push((idx, short));
            }
        }

        if !matched_indices.is_empty() {
            if let Some(p) = progress { p.set(0.35, Some("Copying channel data...")); }
            let mut map: HashMap<String, Vec<f32>> = HashMap::new();
            for (ci, short_name) in matched_indices {
                let mut v: Vec<f32> = Vec::with_capacity(pixel_count);
                for i in 0..pixel_count {
                    v.push(layer.channel_data.list[ci].sample_data.value_by_flat_index(i).to_f32());
                }
                map.insert(short_name, v);
            }
            if let Some(p) = progress { p.finish(Some("Layer channels loaded")); }
            return Ok(LayerChannels { layer_name: layer_name.to_string(), width, height, channels: map });
        }
    }

    if let Some(p) = progress { p.reset(); }
    anyhow::bail!(format!("Nie znaleziono warstwy '{}'", layer_name))
}

// Pomocnicze: buduje kompozyt RGB z mapy kanałów
fn compose_composite_from_channels(layer_channels: &LayerChannels) -> Vec<(f32, f32, f32, f32)> {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    let mut out: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(pixel_count);

    // Heurystyki: najpierw dokładne R/G/B/A, potem nazwy zaczynające się od R/G/B, a na końcu pierwszy dostępny kanał
    let pick_exact = |name: &str| -> Option<&Vec<f32>> { layer_channels.channels.get(name) };
    let pick_prefix = |prefix: char| -> Option<&Vec<f32>> {
        let prefix = prefix.to_ascii_uppercase();
        layer_channels.channels.iter().find(|(k, _)| k.to_ascii_uppercase().starts_with(prefix)).map(|(_, v)| v)
    };

    let r = pick_exact("R").or_else(|| pick_prefix('R')).or_else(|| layer_channels.channels.values().next());
    let g = pick_exact("G").or_else(|| pick_prefix('G')).or(r);
    let b = pick_exact("B").or_else(|| pick_prefix('B')).or(g).or(r);
    let a = pick_exact("A").or_else(|| pick_prefix('A'));

    let r = r.expect("Brak kanału R do kompozytu");
    let g = g.expect("Brak kanału G do kompozytu");
    let b = b.expect("Brak kanału B do kompozytu");

    for i in 0..pixel_count {
        let rr = r[i];
        let gg = g[i];
        let bb = b[i];
        let aa = a.map(|av| av[i]).unwrap_or(1.0);
        out.push((rr, gg, bb, aa));
    }

    out
}