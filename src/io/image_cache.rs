use crate::io::fast_exr_metadata::ChannelInfo;
use crate::log_info;
use crate::ui::progress::ProgressSink;
use crate::utils::split_layer_and_short;
use rayon::prelude::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::HashMap;
use std::path::Path;
use crate::io::full_exr_cache::FullExrCacheData;
use crate::io::lazy_exr_loader::LazyExrLoader;
use glam::Mat3;
use std::sync::Arc;

/// Zwraca kanoniczny skrót kanału na podstawie aliasów/nazw przyjaznych.
/// Np. "red"/"Red"/"RED"/"R"/"R8" → "R"; analogicznie dla G/B/A.
#[inline]
pub(crate) fn channel_alias_to_short(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("R") || trimmed.len() >= 3 && trimmed[..3].eq_ignore_ascii_case("RED") {
        return "R".to_string();
    }
    if trimmed.eq_ignore_ascii_case("G") || trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("GREEN") {
        return "G".to_string();
    }
    if trimmed.eq_ignore_ascii_case("B") || trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("BLUE") {
        return "B".to_string();
    }
    if trimmed.eq_ignore_ascii_case("A") || trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("ALPHA") {
        return "A".to_string();
    }
    trimmed.to_string()
}

#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub name: String,
    pub channels: Vec<ChannelInfo>,
}

// split_layer_and_short przeniesione do utils


#[derive(Clone, Debug)]
pub struct LayerChannels {
    pub layer_name: String,
    pub width: u32,
    pub height: u32,
    // Stabilna lista krótkich nazw kanałów (np. "R", "G", "B", "A", "Z", itp.)
    pub channel_names: Vec<String>,
    // Dane w układzie planarnym: [ch0(0..N), ch1(0..N), ...]
    pub channel_data: Arc<[f32]>, // Zmieniono z Vec<f32> na Arc<[f32]>
}

pub enum ExrDataSource {
    /// Full cache mode: all data in memory (high RAM usage, fast access)
    Full(Arc<FullExrCacheData>),
    /// Lazy mode: load data on demand (low RAM usage, slower access)
    Lazy(Arc<LazyExrLoader>),
}

pub struct ImageCache {
    pub raw_pixels: Vec<f32>, // Zmiana z Vec<(f32,f32,f32,f32)> na Vec<f32>
    pub width: u32,
    pub height: u32,
    pub layers_info: Vec<LayerInfo>,
    pub current_layer_name: String,
    // Opcjonalna macierz konwersji z przestrzeni primaries pliku do sRGB (linear RGB)
    color_matrix_rgb_to_srgb: Option<Mat3>,
    // Cache macierzy kolorów dla każdej warstwy
    color_matrices: HashMap<String, Mat3>,
    // Cache wszystkich kanałów dla bieżącej warstwy aby uniknąć I/O przy przełączaniu
    pub current_layer_channels: Option<LayerChannels>,
    // EXR data source (full cache or lazy loader)
    data_source: ExrDataSource,
    // MIP cache: przeskalowane podglądy (float RGBA) do szybkiego preview
    // Histogram data dla analizy kolorów
    pub histogram: Option<Arc<crate::processing::histogram::HistogramData>>,
}

impl ImageCache {
    pub fn new_with_full_cache(
        path: &Path,
        full_cache: Arc<FullExrCacheData>,
    ) -> anyhow::Result<Self> {
        log_info!(
            "=== ImageCache::new_with_full_cache START === {}",
            path.display()
        );
        // Najpierw wyciągnij informacje o warstwach (meta), wybierz najlepszą i wczytaj ją jako startowy podgląd
        let layers_info = extract_layers_info(path)?;
        let best_layer = find_best_layer(&layers_info);
        let layer_channels = load_all_channels_for_layer_from_full(&full_cache, &best_layer, None)?;

        let raw_pixels = compose_composite_from_channels(&layer_channels);
        let width = layer_channels.width;
        let height = layer_channels.height;
        let current_layer_name = layer_channels.layer_name.clone();

        // Spróbuj wyliczyć macierz konwersji primaries → sRGB na podstawie atrybutu chromaticities (dla wybranej warstwy/partu)
        let mut color_matrices = HashMap::new();
        let color_matrix_rgb_to_srgb = crate::processing::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, &best_layer).ok();
        if let Some(matrix) = color_matrix_rgb_to_srgb {
            color_matrices.insert(best_layer.clone(), matrix);
        }

        Ok(ImageCache {
            raw_pixels,
            width,
            height,
            layers_info,
            current_layer_name,
            color_matrix_rgb_to_srgb,
            color_matrices,
            current_layer_channels: Some(layer_channels),
            data_source: ExrDataSource::Full(full_cache),
            histogram: None, // Będzie obliczany na żądanie
        })
    }

    pub fn new_with_lazy_loader(
        path: &Path,
        lazy_loader: Arc<LazyExrLoader>,
    ) -> anyhow::Result<Self> {
        log_info!(
            "=== ImageCache::new_with_lazy_loader START === {}",
            path.display()
        );

        // Convert lazy metadata to LayerInfo for UI and selection logic
        let layers_info: Vec<LayerInfo> = lazy_loader
            .get_metadata()
            .iter()
            .map(|meta| LayerInfo {
                name: meta.name.clone(),
                channels: meta
                    .channel_names
                    .iter()
                    .map(|n| ChannelInfo::new(n.clone()))
                    .collect(),
            })
            .collect();

        let best_layer = find_best_layer(&layers_info);
        
        // Load the initial best layer
        let layer_data = lazy_loader.get_layer_data(&best_layer, None)?;
        let layer_channels = layer_data.to_layer_channels();

        let raw_pixels = compose_composite_from_channels(&layer_channels);
        let width = layer_channels.width;
        let height = layer_channels.height;
        let current_layer_name = layer_channels.layer_name.clone();

        // Color matrix
        let mut color_matrices = HashMap::new();
        let color_matrix_rgb_to_srgb = crate::processing::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, &best_layer).ok();
        if let Some(matrix) = color_matrix_rgb_to_srgb {
            color_matrices.insert(best_layer.clone(), matrix);
        }

        Ok(ImageCache {
            raw_pixels,
            width,
            height,
            layers_info,
            current_layer_name,
            color_matrix_rgb_to_srgb,
            color_matrices,
            current_layer_channels: Some(layer_channels),
            data_source: ExrDataSource::Lazy(lazy_loader),
            histogram: None,
        })
    }

    pub fn load_layer(
        &mut self,
        path: &Path,
        layer_name: &str,
        progress: Option<&dyn ProgressSink>,
    ) -> anyhow::Result<()> {
        log_info!("=== ImageCache::load_layer START === layer: {}", layer_name);

        // Load layer data based on data source
        let layer_channels = match &self.data_source {
            ExrDataSource::Full(full_cache) => {
                load_all_channels_for_layer_from_full(full_cache, layer_name, progress)?
            }
            ExrDataSource::Lazy(lazy_loader) => {
                let layer_data = lazy_loader.get_layer_data(layer_name, progress)?;
                layer_data.to_layer_channels()
            }
        };

        self.width = layer_channels.width;
        self.height = layer_channels.height;
        self.current_layer_name = layer_channels.layer_name.clone();
        compose_composite_into_buffer(&layer_channels, &mut self.raw_pixels);
        self.current_layer_channels = Some(layer_channels);
        // Sprawdź, czy macierz dla danej warstwy jest już w cache'u
        if self.color_matrices.contains_key(layer_name) {
            self.color_matrix_rgb_to_srgb = self.color_matrices.get(layer_name).cloned();
        } else {
            self.color_matrix_rgb_to_srgb = crate::processing::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, layer_name).ok();
            if let Some(matrix) = self.color_matrix_rgb_to_srgb {
                self.color_matrices.insert(layer_name.to_string(), matrix);
            }
        }

        Ok(())
    }

    pub fn update_histogram(&mut self) -> anyhow::Result<()> {
        let mut histogram = crate::processing::histogram::HistogramData::new(256);
        histogram.compute_from_rgba_pixels(&self.raw_pixels)?;
        self.histogram = Some(Arc::new(histogram));
        log_info!(
            "Histogram updated: {} pixels processed",
            self.histogram.as_ref().unwrap().total_pixels
        );
        Ok(())
    }

    pub fn get_histogram_data(&self) -> Option<Arc<crate::processing::histogram::HistogramData>> {
        self.histogram.clone()
    }

    pub fn process_to_image(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Image {
        log_info!(
            "=== PROCESS_TO_IMAGE START === {}x{}",
            self.width, self.height
        );

        log_info!("Using CPU-only processing");

        // GPU processing removed - using CPU processing only

        // Fallback CPU (SIMD + Rayon)
        log_info!("Using CPU processing for {}x{}", self.width, self.height);
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // Optymalizowana SIMD: separuj SIMD od skalarnej reszty
        self.process_rgba_chunks_optimized(
            &self.raw_pixels,
            out_slice,
            exposure,
            gamma,
            tonemap_mode,
            color_m,
        );

        log_info!("=== PROCESS_TO_IMAGE END - CPU completed ===");
        Image::from_rgba8(buffer)
    }

    fn process_rgba_chunks_optimized(
        &self,
        input: &[f32],
        output: &mut [Rgba8Pixel],
        exposure: f32,
        gamma: f32,
        tonemap_mode: i32,
        color_m: Option<Mat3>,
    ) {
        // Use unified SIMD processing function with parallel processing
        crate::processing::simd_processing::process_rgba_chunk_optimized(
            input,
            output,
            exposure,
            gamma,
            tonemap_mode,
            color_m,
            false,
            true,
        );
    }

    pub fn process_to_composite(
        &self,
        exposure: f32,
        gamma: f32,
        tonemap_mode: i32,
        lighting_rgb: bool,
    ) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // Optymalizowana SIMD: separuj SIMD od skalarnej reszty
        self.process_rgba_chunks_composite_optimized(
            &self.raw_pixels,
            out_slice,
            exposure,
            gamma,
            tonemap_mode,
            color_m,
            lighting_rgb,
        );

        Image::from_rgba8(buffer)
    }

    #[allow(clippy::too_many_arguments)]
    fn process_rgba_chunks_composite_optimized(
        &self,
        input: &[f32],
        output: &mut [Rgba8Pixel],
        exposure: f32,
        gamma: f32,
        tonemap_mode: i32,
        color_m: Option<Mat3>,
        lighting_rgb: bool,
    ) {
        // Use unified SIMD processing function with sequential processing
        crate::processing::simd_processing::process_rgba_chunk_optimized(
            input,
            output,
            exposure,
            gamma,
            tonemap_mode,
            color_m,
            !lighting_rgb,
            false,
        );
    }
}

// === GPU path implementation ===

// GPU functions removed - using CPU-only processing

pub(crate) fn extract_layers_info(path: &Path) -> anyhow::Result<Vec<LayerInfo>> {
    // Odczytaj jedynie meta-dane (nagłówki) bez pikseli
    let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/ false)?;

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

            let entry = layer_map
                .entry(layer_name_effective.clone())
                .or_insert_with(|| {
                    layer_order.push(layer_name_effective.clone());
                    Vec::new()
                });

            entry.push(ChannelInfo::new(short_channel_name));
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
    // Pick the first layer with RGBA channels, then fall back to RGB,
    // then to the first layer. Previously this function routed through
    // crate::io::metadata_traits::utils::find_best_layer via a throwaway
    // MetadataLayerInfo wrapper; the fallback code after that wrapper was
    // unreachable because the wrapper returned Some as long as any layer
    // existed. Inlined here with the same observable behaviour.
    let has_channel = |layer: &LayerInfo, wanted: &str| -> bool {
        layer.channels.iter().any(|c| c.name == wanted)
    };

    layers_info
        .iter()
        .find(|l| {
            has_channel(l, "R")
                && has_channel(l, "G")
                && has_channel(l, "B")
                && has_channel(l, "A")
        })
        .or_else(|| {
            layers_info
                .iter()
                .find(|l| has_channel(l, "R") && has_channel(l, "G") && has_channel(l, "B"))
        })
        .or_else(|| layers_info.first())
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "Layer 1".to_string())
}

// Pomocnicze: wczytuje wszystkie kanały dla wybranej warstwy do pamięci (bez dalszego I/O przy przełączaniu)
pub(crate) fn load_all_channels_for_layer_from_full(
    full: &Arc<FullExrCacheData>,
    layer_name: &str,
    _progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<LayerChannels> {
    if let Some(p) = _progress {
        p.start_indeterminate(Some("Reading layer channels..."));
    }

    for layer in full.layers.iter() {
        let matches = if layer_name.is_empty() && layer.name.is_empty() {
            true
        } else if layer_name.is_empty() || layer.name.is_empty() {
            false
        } else {
            layer.name.eq_ignore_ascii_case(layer_name)
                || layer.name.as_bytes().windows(layer_name.len()).any(|w| w.eq_ignore_ascii_case(layer_name.as_bytes()))
                || layer_name.as_bytes().windows(layer.name.len()).any(|w| w.eq_ignore_ascii_case(layer.name.as_bytes()))
        };
        if matches {
            if let Some(p) = _progress {
                p.set(0.35, Some("Copying channel data..."));
            }
            let channel_names = layer.channel_names.clone();

            let channel_data = Arc::clone(&layer.channel_data);
            if let Some(p) = _progress {
                p.finish(Some("Layer channels loaded"));
            }
            return Ok(LayerChannels {
                layer_name: layer_name.to_string(),
                width: layer.width,
                height: layer.height,
                channel_names,
                channel_data,
            });
        }
    }

    if let Some(p) = _progress {
        p.reset();
    }
    anyhow::bail!(format!("Nie znaleziono warstwy '{}'", layer_name))
}

// Pomocnicze: buduje kompozyt RGB z mapy kanałów - zoptymalizowana wersja
/// Resolve RGB(A) channel indices from channel names list
fn resolve_rgb_indices(channel_names: &[String]) -> (usize, usize, usize, Option<usize>) {
    let pick_exact = |name: &str| -> Option<usize> {
        channel_names.iter().position(|n| n == name)
    };
    let pick_prefix = |prefix: char| -> Option<usize> {
        let prefix = prefix.to_ascii_uppercase();
        channel_names
            .iter()
            .position(|n| n.to_ascii_uppercase().starts_with(prefix))
    };

    let r_idx_opt = pick_exact("R").or_else(|| pick_prefix('R'));
    let g_idx_opt = pick_exact("G").or_else(|| pick_prefix('G'));
    let b_idx_opt = pick_exact("B").or_else(|| pick_prefix('B'));

    let has_any_rgb = r_idx_opt.is_some() || g_idx_opt.is_some() || b_idx_opt.is_some();

    let (r_idx, g_idx, b_idx) = if has_any_rgb {
        let r_idx = r_idx_opt.unwrap_or_else(|| g_idx_opt.or(b_idx_opt).unwrap_or(0));
        let g_idx = g_idx_opt.unwrap_or(r_idx);
        let b_idx = b_idx_opt.unwrap_or(g_idx);
        (r_idx, g_idx, b_idx)
    } else {
        let num_channels = channel_names.len();
        let r_idx = 0;
        let g_idx = if num_channels > 1 { 1 } else { 0 };
        let b_idx = if num_channels > 2 { 2 } else { g_idx };
        (r_idx, g_idx, b_idx)
    };

    let a_idx = pick_exact("A").or_else(|| pick_prefix('A'));
    (r_idx, g_idx, b_idx, a_idx)
}

/// Compose RGBA composite into an existing buffer (reuses allocation)
fn compose_composite_into_buffer(layer_channels: &LayerChannels, out: &mut Vec<f32>) {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    let buffer_size = pixel_count * 4;
    out.resize(buffer_size, 0.0);

    let (r_idx, g_idx, b_idx, a_idx) = resolve_rgb_indices(&layer_channels.channel_names);

    let r_plane = &layer_channels.channel_data[r_idx * pixel_count..(r_idx + 1) * pixel_count];
    let g_plane = &layer_channels.channel_data[g_idx * pixel_count..(g_idx + 1) * pixel_count];
    let b_plane = &layer_channels.channel_data[b_idx * pixel_count..(b_idx + 1) * pixel_count];
    let a_plane = a_idx.map(|ai| &layer_channels.channel_data[ai * pixel_count..(ai + 1) * pixel_count]);

    // Use sequential for small images (< 2MP), parallel for larger
    let compose_fn = |(i, chunk): (usize, &mut [f32])| {
        chunk[0] = r_plane[i];
        chunk[1] = g_plane[i];
        chunk[2] = b_plane[i];
        chunk[3] = a_plane.map_or(1.0, |a| a[i]);
    };

    if pixel_count < 2_000_000 {
        out.chunks_exact_mut(4).enumerate().for_each(compose_fn);
    } else {
        out.par_chunks_exact_mut(4).enumerate().for_each(compose_fn);
    }
}

pub(crate) fn compose_composite_from_channels(layer_channels: &LayerChannels) -> Vec<f32> {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    let mut out = Vec::with_capacity(pixel_count * 4);
    compose_composite_into_buffer(layer_channels, &mut out);
    out
}

impl ImageCache {
    /// Wczytuje jeden wskazany kanał z danej warstwy i zapisuje go jako grayscale (R=G=B=val, A=1)
    pub fn load_channel(
        &mut self,
        path: &Path,
        layer_name: &str,
        channel_short: &str,
        progress: Option<&dyn ProgressSink>,
    ) -> anyhow::Result<()> {
        // Zapewnij, że cache kanałów dla żądanej warstwy jest dostępny
        let need_reload = self
            .current_layer_channels
            .as_ref()
            .map(|lc| lc.layer_name.to_lowercase() != layer_name.to_lowercase())
            .unwrap_or(true);
        if need_reload {
            // Załaduj wskazaną warstwę (zapełni current_layer_channels oraz ustawi kompozyt)
            self.load_layer(path, layer_name, progress)?;
        }

        // Teraz mamy current_layer_channels dla właściwej warstwy
        let layer_cache = self
            .current_layer_channels
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Brak cache kanałów dla warstwy"))?;

        let pixel_count = (layer_cache.width as usize) * (layer_cache.height as usize);

        let find_channel_index = |wanted: &str| -> Option<usize> {
            // 1) dokładne dopasowanie (case-sensitive)
            if let Some(idx) = layer_cache.channel_names.iter().position(|k| k == wanted) {
                return Some(idx);
            }
            // 2) case-insensitive
            let wanted_lower = wanted.to_lowercase();
            if let Some((idx, _)) = layer_cache
                .channel_names
                .iter()
                .enumerate()
                .find(|(_, k)| k.to_lowercase() == wanted_lower)
            {
                return Some(idx);
            }
            // 3) według kanonicznego skrótu R/G/B/A
            let wanted_canon = channel_alias_to_short(wanted).to_ascii_uppercase();
            if let Some((idx, _)) = layer_cache
                .channel_names
                .iter()
                .enumerate()
                .find(|(_, k)| channel_alias_to_short(k).to_ascii_uppercase() == wanted_canon)
            {
                return Some(idx);
            }
            None
        };

        // Specjalne traktowanie Depth
        let wanted_upper = channel_short.to_ascii_uppercase();
        let is_depth = wanted_upper == "Z" || wanted_upper.contains("DEPTH");

        let channel_index_opt = if is_depth {
            // Preferuj dokładnie "Z"; w razie braku wybierz kanał zawierający "DEPTH" albo "DISTANCE"
            find_channel_index("Z").or_else(|| {
                layer_cache.channel_names.iter().position(|k| {
                    k.to_ascii_uppercase().contains("DEPTH") || k.eq_ignore_ascii_case("DISTANCE")
                })
            })
        } else {
            find_channel_index(channel_short)
        };

        let channel_index = channel_index_opt.ok_or_else(|| {
            anyhow::anyhow!(format!(
                "Nie znaleziono kanału '{}' w warstwie '{}'",
                channel_short, layer_cache.layer_name
            ))
        })?;

        let base = channel_index * pixel_count;
        let channel_slice = &layer_cache.channel_data[base..base + pixel_count];

        // Reuse existing raw_pixels buffer - expand grayscale channel to RGBA
        let buffer_size = pixel_count * 4;
        self.raw_pixels.resize(buffer_size, 0.0);

        // Parallel grayscale expansion using rayon
        self.raw_pixels.par_chunks_exact_mut(4).enumerate().for_each(|(i, chunk)| {
            let v = channel_slice[i];
            chunk[0] = v; // R
            chunk[1] = v; // G
            chunk[2] = v; // B
            chunk[3] = 1.0; // A
        });
        self.width = layer_cache.width;
        self.height = layer_cache.height;
        self.current_layer_name = layer_cache.layer_name.clone();
        Ok(())
    }

    /// Specjalne renderowanie głębi: auto-normalizacja percentylowa + opcjonalne odwrócenie
    pub fn process_depth_image_with_progress(
        &self,
        invert: bool,
        progress: Option<&dyn ProgressSink>,
    ) -> Image {
        if let Some(p) = progress {
            p.start_indeterminate(Some("Processing depth data..."));
        }
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let slice = buffer.make_mut_slice();

        // Wyciągnij z surowych pikseli jeden kanał (zakładamy, że R=G=B=val)
        let mut values: Vec<f32> = self
            .raw_pixels
            .par_chunks_exact(4)
            .map(|chunk| chunk[0])
            .collect();
        if values.is_empty() {
            return Image::from_rgba8(buffer);
        }

        // Policz percentyle 1% i 99% (odporne na outliery) w ~O(n)
        use std::cmp::Ordering;
        let len = values.len();
        let p_lo_idx = ((len as f32) * 0.01).floor() as usize;
        let mut p_hi_idx = ((len as f32) * 0.99).ceil() as isize - 1;
        if p_hi_idx < 0 {
            p_hi_idx = 0;
        }
        let p_hi_idx = (p_hi_idx as usize).min(len - 1);
        let (_, lo_ref, _) = values
            .select_nth_unstable_by(p_lo_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let mut lo = *lo_ref;
        let (_, hi_ref, _) = values
            .select_nth_unstable_by(p_hi_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let mut hi = *hi_ref;
        if let Some(p) = progress {
            p.set(0.4, Some("Computing percentiles..."));
        }
        if !lo.is_finite() || !hi.is_finite() || (hi - lo).abs() < 1e-20 {
            let mut min_v = f32::INFINITY;
            let mut max_v = f32::NEG_INFINITY;
            for &v in &values {
                let nv = if v.is_finite() { v } else { 0.0 };
                if nv < min_v {
                    min_v = nv;
                }
                if nv > max_v {
                    max_v = nv;
                }
            }
            lo = min_v;
            hi = max_v;
        }
        if (hi - lo).abs() < 1e-12 {
            hi = lo + 1.0;
        }

        let map_val = |v: f32| -> u8 {
            let mut t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            if invert {
                t = 1.0 - t;
            }
            (t * 255.0).round().clamp(0.0, 255.0) as u8
        };

        if let Some(p) = progress {
            p.set(0.8, Some("Rendering depth image..."));
        }
        self.raw_pixels
            .par_chunks_exact(4)
            .zip(slice.par_iter_mut())
            .for_each(|(chunk, out)| {
                let g8 = map_val(chunk[0]);
                *out = Rgba8Pixel {
                    r: g8,
                    g: g8,
                    b: g8,
                    a: 255,
                };
            });

        if let Some(p) = progress {
            p.finish(Some("Depth processed"));
        }
        Image::from_rgba8(buffer)
    }

}
