use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use exr::prelude as exr;
use std::path::PathBuf;
use rayon::prelude::*;
use std::collections::HashMap; // potrzebne dla extract_layers_info
use crate::utils::split_layer_and_short;
use crate::ui::progress::ProgressSink;
// use crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer;
use glam::{Mat3, Vec3};
use std::sync::Arc;
use crate::io::full_exr_cache::FullExrCacheData;
use crate::io::lazy_exr_loader::LazyExrLoader;
use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;
use crate::utils::buffer_pool::BufferPool;
use std::sync::OnceLock;

// Global buffer pool for performance optimization
static GLOBAL_BUFFER_POOL: OnceLock<Arc<BufferPool>> = OnceLock::new();

pub fn set_global_buffer_pool(pool: Arc<BufferPool>) {
    GLOBAL_BUFFER_POOL.set(pool).ok();
}

fn get_buffer_pool() -> Option<&'static Arc<BufferPool>> {
    GLOBAL_BUFFER_POOL.get()
}


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
    // Stabilna lista krótkich nazw kanałów (np. "R", "G", "B", "A", "Z", itp.)
    pub channel_names: Vec<String>,
    // Dane w układzie planarnym: [ch0(0..N), ch1(0..N), ...]
    pub channel_data: Arc<[f32]>, // Zmieniono z Vec<f32> na Arc<[f32]>
}

#[allow(dead_code)]
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
    pub mip_levels: Vec<MipLevel>,
    // Histogram data dla analizy kolorów
    pub histogram: Option<Arc<crate::processing::histogram::HistogramData>>,
}

#[derive(Clone, Debug)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<f32>,
}

/// Ujednolicona funkcja MIP generation - automatycznie wybiera GPU lub CPU
fn build_mip_chain(
    base_pixels: &[f32],
    width: u32,
    height: u32,
    max_levels: usize,
    _use_gpu: bool,
) -> Vec<MipLevel> {
    // CPU-only processing
    build_mip_chain_cpu(base_pixels, width, height, max_levels)
}


fn build_mip_chain_cpu(
    base_pixels: &[f32],
    mut width: u32,
    mut height: u32,
    max_levels: usize,
) -> Vec<MipLevel> {
    let mut levels: Vec<MipLevel> = Vec::with_capacity(max_levels);
    let mut prev: Vec<f32> = base_pixels.to_vec();
    for _ in 0..max_levels {
        if width <= 1 && height <= 1 { break; }
        let new_w = (width / 2).max(1);
        let new_h = (height / 2).max(1);
        
        // Use buffer pool for better performance
        let pixel_count = (new_w as usize) * (new_h as usize) * 4;
        let mut next = if let Some(pool) = get_buffer_pool() {
            let mut buffer = pool.get_f32_buffer(pixel_count);
            buffer.resize(pixel_count, 0.0);
            buffer
        } else {
            vec![0.0; pixel_count] // Fallback for when pool isn't initialized
        };
        // Jednowątkowe uśrednianie 2x2
        for y_out in 0..(new_h as usize) {
            let y0 = (y_out * 2).min(height as usize - 1);
            let y1 = (y0 + 1).min(height as usize - 1);
            for x_out in 0..(new_w as usize) {
                let x0 = (x_out * 2).min(width as usize - 1);
                let x1 = (x0 + 1).min(width as usize - 1);
                let base0 = (y0 * (width as usize) + x0) * 4;
                let base1 = (y0 * (width as usize) + x1) * 4;
                let base2 = (y1 * (width as usize) + x0) * 4;
                let base3 = (y1 * (width as usize) + x1) * 4;
                let out_base = (y_out * (new_w as usize) + x_out) * 4;
                
                // Uśrednij 4 kanały RGBA
                for c in 0..4 {
                    let acc = (prev[base0 + c] + prev[base1 + c] + prev[base2 + c] + prev[base3 + c]) * 0.25;
                    next[out_base + c] = acc;
                }
            }
        }
        levels.push(MipLevel { width: new_w, height: new_h, pixels: next.clone() });
        prev = next;
        width = new_w; height = new_h;
        if new_w <= 32 && new_h <= 32 { break; }
    }
    levels
}

impl ImageCache {
    pub fn new_with_full_cache(path: &PathBuf, full_cache: Arc<FullExrCacheData>) -> anyhow::Result<Self> {
        println!("=== ImageCache::new_with_full_cache START === {}", path.display());
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

        let mip_levels = build_mip_chain(&raw_pixels, width, height, 4, true);
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
            mip_levels,
            histogram: None, // Będzie obliczany na żądanie
        })
    }
    
    /// Create ImageCache with lazy loading (low memory usage)
    #[allow(dead_code)]
    pub fn new_with_lazy_loader(path: &PathBuf, max_cached_layers: usize) -> anyhow::Result<Self> {
        println!("=== ImageCache::new_with_lazy_loader START === {}", path.display());
        
        // Create lazy loader (loads only metadata)
        let lazy_loader = Arc::new(LazyExrLoader::new(path.clone(), max_cached_layers)?);
        
        // Extract layer info from metadata  
        let metadata = lazy_loader.get_metadata();
        let mut layers_info = Vec::with_capacity(metadata.len());
        
        for layer_meta in metadata {
            let channels = layer_meta.channel_names.iter()
                .map(|name| ChannelInfo { name: name.clone() })
                .collect();
            layers_info.push(LayerInfo {
                name: layer_meta.name.clone(),
                channels,
            });
        }
        
        // Find best layer and load it initially
        let best_layer = find_best_layer(&layers_info);
        let layer_data = lazy_loader.get_layer_data(&best_layer, None)?;
        let layer_channels = layer_data.to_layer_channels();
        
        let raw_pixels = compose_composite_from_channels(&layer_channels);
        let width = layer_channels.width;
        let height = layer_channels.height;
        let current_layer_name = layer_channels.layer_name.clone();
        
        // Color matrix calculation
        let mut color_matrices = HashMap::new();
        let color_matrix_rgb_to_srgb = crate::processing::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, &best_layer).ok();
        if let Some(matrix) = color_matrix_rgb_to_srgb {
            color_matrices.insert(best_layer.clone(), matrix);
        }
        
        let mip_levels = build_mip_chain(&raw_pixels, width, height, 4, true);
        
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
            mip_levels,
            histogram: None,
        })
    }
    
    pub fn load_layer(&mut self, path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
        println!("=== ImageCache::load_layer START === layer: {}", layer_name);
        
        // Load layer data based on data source
        let layer_channels = match &self.data_source {
            ExrDataSource::Full(full_cache) => {
                load_all_channels_for_layer_from_full(full_cache, layer_name, progress)?
            },
            ExrDataSource::Lazy(lazy_loader) => {
                let layer_data = lazy_loader.get_layer_data(layer_name, progress)?;
                layer_data.to_layer_channels()
            },
        };

        self.width = layer_channels.width;
        self.height = layer_channels.height;
        self.current_layer_name = layer_channels.layer_name.clone();
        self.raw_pixels = compose_composite_from_channels(&layer_channels);
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
        // Odbuduj MIP-y dla nowego obrazu
        self.mip_levels = build_mip_chain(&self.raw_pixels, self.width, self.height, 4, true);

        Ok(())
    }

    pub fn update_histogram(&mut self) -> anyhow::Result<()> {
        let mut histogram = crate::processing::histogram::HistogramData::new(256);
        histogram.compute_from_rgba_pixels(&self.raw_pixels)?;
        self.histogram = Some(Arc::new(histogram));
        println!("Histogram updated: {} pixels processed", self.histogram.as_ref().unwrap().total_pixels);
        Ok(())
    }

    pub fn get_histogram_data(&self) -> Option<Arc<crate::processing::histogram::HistogramData>> {
        self.histogram.clone()
    }

    pub fn process_to_image(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Image {
        println!("=== PROCESS_TO_IMAGE START === {}x{}", self.width, self.height);
        
        println!("Using CPU-only processing");
        
        // GPU processing removed - using CPU processing only

        // Fallback CPU (SIMD + Rayon)
        println!("Using CPU processing for {}x{}", self.width, self.height);
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // Optymalizowana SIMD: separuj SIMD od skalarnej reszty  
        self.process_rgba_chunks_optimized(&self.raw_pixels, out_slice, exposure, gamma, tonemap_mode, color_m);

        println!("=== PROCESS_TO_IMAGE END - CPU completed ===");
        Image::from_rgba8(buffer)
    }
    
    fn process_rgba_chunks_optimized(&self, input: &[f32], output: &mut [Rgba8Pixel], exposure: f32, gamma: f32, tonemap_mode: i32, color_m: Option<Mat3>) {
        // Use parallel processing with new optimized SIMD module
        use rayon::prelude::*;
        
        input.par_chunks_exact(crate::processing::simd_processing::SIMD_CHUNK_SIZE)
            .zip(output.par_chunks_exact_mut(crate::processing::simd_processing::SIMD_PIXEL_COUNT))
            .for_each(|(in_chunk, out_chunk)| {
                // Safe: par_chunks_exact guarantees correct sizes
                let in_array: &[f32; 16] = unsafe { in_chunk.try_into().unwrap_unchecked() };
                let out_array: &mut [Rgba8Pixel; 4] = unsafe { out_chunk.try_into().unwrap_unchecked() };
                
                crate::processing::simd_processing::process_simd_chunk_rgba(
                    in_array, out_array, exposure, gamma, tonemap_mode, color_m
                );
            });
        
        // Handle remainder with optimized scalar processing
        let simd_elements = (input.len() / crate::processing::simd_processing::SIMD_CHUNK_SIZE) * crate::processing::simd_processing::SIMD_CHUNK_SIZE;
        let simd_pixels = simd_elements / 4;
        
        if simd_elements < input.len() {
            crate::processing::simd_processing::process_scalar_pixels(
                &input[simd_elements..],
                &mut output[simd_pixels..],
                exposure, gamma, tonemap_mode, color_m, false
            );
        }
    }
    
    
    pub fn process_to_composite(&self, exposure: f32, gamma: f32, tonemap_mode: i32, lighting_rgb: bool) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // Optymalizowana SIMD: separuj SIMD od skalarnej reszty  
        self.process_rgba_chunks_composite_optimized(&self.raw_pixels, out_slice, exposure, gamma, tonemap_mode, color_m, lighting_rgb);
        
        Image::from_rgba8(buffer)
    }
    
    fn process_rgba_chunks_composite_optimized(&self, input: &[f32], output: &mut [Rgba8Pixel], exposure: f32, gamma: f32, tonemap_mode: i32, color_m: Option<Mat3>, lighting_rgb: bool) {
        // Use the optimized SIMD processing module
        crate::processing::simd_processing::process_image_optimized(
            input, output, exposure, gamma, tonemap_mode, color_m, !lighting_rgb
        );
    }
    
    // Nowa metoda dla preview (szybsze przetwarzanie małego obrazka)
    pub fn process_to_thumbnail(&self, exposure: f32, gamma: f32, tonemap_mode: i32, max_size: u32) -> Image {
        let scale = (max_size as f32 / self.width.max(self.height) as f32).min(1.0);
        let thumb_width = (self.width as f32 * scale) as u32;
        let thumb_height = (self.height as f32 * scale) as u32;
        
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(thumb_width, thumb_height);
        let slice = buffer.make_mut_slice();
        
        // Wybierz najlepsze źródło: oryginał lub najbliższy MIP >= docelowej wielkości
        let (src_pixels, src_w, src_h): (&[f32], u32, u32) = {
            if self.mip_levels.is_empty() { (&self.raw_pixels[..], self.width, self.height) } else {
                // wybierz poziom, którego dłuższy bok jest najbliższy docelowemu, ale nie mniejszy niż docelowy
                let target = thumb_width.max(thumb_height);
                if let Some(lvl) = self.mip_levels.iter().find(|lvl| lvl.width.max(lvl.height) >= target) {
                    (&lvl.pixels[..], lvl.width, lvl.height)
                } else {
            
                    (&self.raw_pixels[..], self.width, self.height)
                }
            }
        };

        // GPU processing removed - CPU only
        let gpu_processed_src: Option<Vec<u8>> = None;

        // Proste nearest neighbor sampling dla szybkości, ale przetwarzanie blokami 4 pikseli
        let m = self.color_matrix_rgb_to_srgb;
        let scale_x = (src_w as f32) / (thumb_width.max(1) as f32);
        let scale_y = (src_h as f32) / (thumb_height.max(1) as f32);
        // SIMD: paczki po 4 piksele miniatury (równolegle na blokach 4 pikseli)
        if let Some(ref gpu_src) = gpu_processed_src {
            // Skalowanie NN z już przetworzonych RGBA8 (piksel-po-pikselu)
            slice
                .par_iter_mut()
                .enumerate()
                .for_each(|(i, px)| {
                    if i >= (thumb_width as usize) * (thumb_height as usize) { return; }
                    let x = (i as u32) % thumb_width;
                    let y = (i as u32) / thumb_width;
                    let src_x = ((x as f32) * scale_x) as u32;
                    let src_y = ((y as f32) * scale_y) as u32;
                    let sx = src_x.min(src_w.saturating_sub(1)) as usize;
                    let sy = src_y.min(src_h.saturating_sub(1)) as usize;
                    let src_idx = sy * (src_w as usize) + sx;
                    let base = src_idx * 4;
                    *px = Rgba8Pixel { r: gpu_src[base], g: gpu_src[base + 1], b: gpu_src[base + 2], a: gpu_src[base + 3] };
                });
        } else {
            slice
                .par_chunks_mut(4) // 4 piksele na blok
                .enumerate()
                .for_each(|(block_idx, out_block)| {
                    let base = block_idx * 4;
                    let mut rr = [0.0f32; 4];
                    let mut gg = [0.0f32; 4];
                    let mut bb = [0.0f32; 4];
                    let mut aa = [1.0f32; 4];
                    let mut valid = 0usize;
                    for lane in 0..4 {
                        let i = base + lane;
                        if i >= (thumb_width as usize) * (thumb_height as usize) { break; }
                        let x = (i as u32) % thumb_width;
                        let y = (i as u32) / thumb_width;
                        let src_x = ((x as f32) * scale_x) as u32;
                        let src_y = ((y as f32) * scale_y) as u32;
                        let sx = src_x.min(src_w.saturating_sub(1)) as usize;
                        let sy = src_y.min(src_h.saturating_sub(1)) as usize;
                        let src_idx = sy * (src_w as usize) + sx;
                        let pixel_start = src_idx * 4;
                        let mut r = src_pixels[pixel_start];
                        let mut g = src_pixels[pixel_start + 1];
                        let mut b = src_pixels[pixel_start + 2];
                        let a = src_pixels[pixel_start + 3];
                        if let Some(mat) = m {
                            let v = mat * Vec3::new(r, g, b);
                            r = v.x; g = v.y; b = v.z;
                        }
                        rr[lane] = r; gg[lane] = g; bb[lane] = b; aa[lane] = a; valid += 1;
                    }
                    let (r8, g8, b8) = crate::processing::tone_mapping::tone_map_and_gamma_simd(
                        f32x4::from_array(rr), f32x4::from_array(gg), f32x4::from_array(bb), exposure, gamma, tonemap_mode);
                    let a8 = f32x4::from_array(aa).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
                    let ra: [f32; 4] = r8.into();
                    let ga: [f32; 4] = g8.into();
                    let ba: [f32; 4] = b8.into();
                    let aa: [f32; 4] = a8.into();
                    for lane in 0..valid.min(4) {
                        out_block[lane] = Rgba8Pixel {
                            r: (ra[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                            g: (ga[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                            b: (ba[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                            a: (aa[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                        };
                    }
                });
        }
        
        Image::from_rgba8(buffer)
    }


}

// === GPU path implementation ===

// GPU functions removed - using CPU-only processing



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
    // Use unified metadata approach for better layer selection
    use crate::io::metadata_traits::{LayerDescriptor, utils::find_best_layer as unified_find_best};
    
    // Convert to unified format for consistent layer selection logic
    let unified_layers: Vec<crate::io::metadata_traits::UnifiedLayerInfo> = 
        layers_info.iter().cloned().map(|l| l.into()).collect();
    
    if let Some(best_layer) = unified_find_best(&unified_layers) {
        return best_layer.name().to_string();
    }
    
    // Fallback to original logic if needed
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
    
    let priority_names = ["beauty", "Beauty", "RGBA", "rgba", "default", "Default", "combined", "Combined"];
    
    for priority_name in &priority_names {
        if let Some(layer) = layers_info.iter().find(|l| l.name.to_lowercase().contains(&priority_name.to_lowercase())) {
            return layer.name.clone();
        }
    }
    
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
    
    layers_info.first()
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "Layer 1".to_string())
}

// Pomocnicze: wczytuje wszystkie kanały dla wybranej warstwy do pamięci (bez dalszego I/O przy przełączaniu)
pub(crate) fn load_all_channels_for_layer_from_full(
    full: &Arc<FullExrCacheData>,
    layer_name: &str,
    _progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<LayerChannels> {
    if let Some(p) = _progress { p.start_indeterminate(Some("Reading layer channels...")); }

    let wanted_lower = layer_name.to_lowercase();

    for layer in full.layers.iter() {
        let lname_lower = layer.name.to_lowercase();
        let matches = if wanted_lower.is_empty() && lname_lower.is_empty() {
            true
        } else if wanted_lower.is_empty() || lname_lower.is_empty() {
            false
        } else {
            lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
        };
        if matches {
            if let Some(p) = _progress { p.set(0.35, Some("Copying channel data...")); }
            let channel_names = layer.channel_names.clone();
    
            let channel_data = Arc::from(layer.channel_data.as_slice());
            if let Some(p) = _progress { p.finish(Some("Layer channels loaded")); }
            return Ok(LayerChannels { layer_name: layer_name.to_string(), width: layer.width, height: layer.height, channel_names, channel_data });
        }
    }

    if let Some(p) = _progress { p.reset(); }
    anyhow::bail!(format!("Nie znaleziono warstwy '{}'", layer_name))
}

/// Zachowany wariant czytający z dysku (używany w ścieżkach niezależnych od globalnego cache)
#[allow(dead_code)]
pub(crate) fn load_all_channels_for_layer(
    path: &PathBuf,
    layer_name: &str,
    _progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<LayerChannels> {
    let any_image = exr::read_all_flat_layers_from_file(path)?;
    let wanted_lower = layer_name.to_lowercase();
    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);
        let base_attr: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
        let lname_lower = base_attr.as_deref().unwrap_or("").to_lowercase();
        let matches = if wanted_lower.is_empty() && lname_lower.is_empty() {
            true
        } else if wanted_lower.is_empty() || lname_lower.is_empty() {
            false
        } else {
            lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
        };
        if matches {
            let num_channels = layer.channel_data.list.len();
            let mut channel_names: Vec<String> = Vec::with_capacity(num_channels);
            
            // Use buffer pool for channel data
            let channel_data_size = pixel_count * num_channels;
            let mut channel_data_vec = if let Some(pool) = get_buffer_pool() {
                pool.get_f32_buffer(channel_data_size)
            } else {
                Vec::with_capacity(channel_data_size)
            };
            for (ci, ch) in layer.channel_data.list.iter().enumerate() {
                let full = ch.name.to_string();
                let (_lname, short) = split_layer_and_short(&full, base_attr.as_deref());
                channel_names.push(short);
                for i in 0..pixel_count { channel_data_vec.push(layer.channel_data.list[ci].sample_data.value_by_flat_index(i).to_f32()); }
            }
            let channel_data = Arc::from(channel_data_vec.into_boxed_slice()); // Convert Vec to Arc<[f32]>
            return Ok(LayerChannels { layer_name: layer_name.to_string(), width, height, channel_names, channel_data });
        }
    }
    anyhow::bail!(format!("Nie znaleziono warstwy '{}'", layer_name))
}

// Pomocnicze: buduje kompozyt RGB z mapy kanałów - zoptymalizowana wersja
fn compose_composite_from_channels(layer_channels: &LayerChannels) -> Vec<f32> {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    
    // Use buffer pool for better performance
    let buffer_size = pixel_count * 4;
    let mut out = if let Some(pool) = get_buffer_pool() {
        pool.get_f32_buffer(buffer_size)
    } else {
        Vec::with_capacity(buffer_size)
    };
    
    let pick_exact_index = |name: &str| -> Option<usize> { layer_channels.channel_names.iter().position(|n| n == name) };
    let pick_prefix_index = |prefix: char| -> Option<usize> {
        let prefix = prefix.to_ascii_uppercase();
        layer_channels.channel_names.iter().position(|n| n.to_ascii_uppercase().starts_with(prefix))
    };

    // Sprawdź czy warstwa ma kanały RGB - jeśli nie, użyj pierwszych 3 dostępnych kanałów
    let has_rgb = pick_exact_index("R").is_some() || pick_prefix_index('R').is_some();
    
    let (r_idx, g_idx, b_idx) = if has_rgb {
        // Standardowe mapowanie RGB
        let r_idx = pick_exact_index("R").or_else(|| pick_prefix_index('R')).unwrap_or(0);
        let g_idx = pick_exact_index("G").or_else(|| pick_prefix_index('G')).unwrap_or(r_idx);
        let b_idx = pick_exact_index("B").or_else(|| pick_prefix_index('B')).unwrap_or(g_idx);
        (r_idx, g_idx, b_idx)
    } else {
        // Dla warstw bez RGB (np. cryptomatte) - użyj pierwszych 3 kanałów
        let num_channels = layer_channels.channel_names.len();
        let r_idx = 0;
        let g_idx = if num_channels > 1 { 1 } else { 0 };
        let b_idx = if num_channels > 2 { 2 } else { g_idx };
        println!("Non-RGB layer '{}': mapping channels [{}] -> R:{}, G:{}, B:{}", 
                 layer_channels.layer_name, 
                 layer_channels.channel_names.join(", "), 
                 r_idx, g_idx, b_idx);
        (r_idx, g_idx, b_idx)
    };
    
    let a_idx = pick_exact_index("A").or_else(|| pick_prefix_index('A'));

    let base_r = r_idx * pixel_count;
    let base_g = g_idx * pixel_count;
    let base_b = b_idx * pixel_count;
    let a_base_opt = a_idx.map(|ai| ai * pixel_count);

    let r_plane = &layer_channels.channel_data[base_r..base_r + pixel_count];
    let g_plane = &layer_channels.channel_data[base_g..base_g + pixel_count];
    let b_plane = &layer_channels.channel_data[base_b..base_b + pixel_count];
    let a_plane = a_base_opt.map(|ab| &layer_channels.channel_data[ab..ab + pixel_count]);

    // Optimized bulk memory operations - use unsafe for maximum performance
    unsafe {
        out.set_len(pixel_count * 4);
        let out_ptr = out.as_mut_ptr();
        
        // Process pixels in chunks of 4 for better cache efficiency
        for i in 0..pixel_count {
            let base_idx = i * 4;
            *out_ptr.add(base_idx) = r_plane[i];
            *out_ptr.add(base_idx + 1) = g_plane[i];
            *out_ptr.add(base_idx + 2) = b_plane[i];
            *out_ptr.add(base_idx + 3) = if let Some(a) = a_plane { a[i] } else { 1.0 };
        }
    }

    out
}

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

        let pixel_count = (layer_cache.width as usize) * (layer_cache.height as usize);

        let find_channel_index = |wanted: &str| -> Option<usize> {
            // 1) dokładne dopasowanie (case-sensitive)
            if let Some(idx) = layer_cache.channel_names.iter().position(|k| k == wanted) { return Some(idx); }
            // 2) case-insensitive
            let wanted_lower = wanted.to_lowercase();
            if let Some((idx, _)) = layer_cache.channel_names.iter().enumerate().find(|(_, k)| k.to_lowercase() == wanted_lower) { return Some(idx); }
            // 3) według kanonicznego skrótu R/G/B/A
            let wanted_canon = channel_alias_to_short(wanted).to_ascii_uppercase();
            if let Some((idx, _)) = layer_cache.channel_names.iter().enumerate().find(|(_, k)| channel_alias_to_short(k).to_ascii_uppercase() == wanted_canon) { return Some(idx); }
            None
        };

        // Specjalne traktowanie Depth
        let wanted_upper = channel_short.to_ascii_uppercase();
        let is_depth = wanted_upper == "Z" || wanted_upper.contains("DEPTH");

        let channel_index_opt = if is_depth {
            // Preferuj dokładnie "Z"; w razie braku wybierz kanał zawierający "DEPTH" albo "DISTANCE"
            find_channel_index("Z").or_else(|| {
                layer_cache
                    .channel_names
                    .iter()
                    .position(|k| k.to_ascii_uppercase().contains("DEPTH") || k.to_ascii_uppercase() == "DISTANCE")
            })
        } else {
            find_channel_index(channel_short)
        };

        let channel_index = channel_index_opt
            .ok_or_else(|| anyhow::anyhow!(format!("Nie znaleziono kanału '{}' w warstwie '{}'", channel_short, layer_cache.layer_name)))?;

        let base = channel_index * pixel_count;
        let channel_slice = &layer_cache.channel_data[base..base + pixel_count];

        // Use buffer pool for better performance
        let buffer_size = pixel_count * 4;
        let mut out = if let Some(pool) = get_buffer_pool() {
            pool.get_f32_buffer(buffer_size)
        } else {
            Vec::with_capacity(buffer_size)
        };
        
        for &v in channel_slice.iter() {
            out.push(v); // R
            out.push(v); // G
            out.push(v); // B
            out.push(1.0); // A
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
        let mut values: Vec<f32> = self.raw_pixels.par_chunks_exact(4).map(|chunk| chunk[0]).collect();
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
        self.raw_pixels.par_chunks_exact(4).zip(slice.par_iter_mut()).for_each(|(chunk, out)| {
            let g8 = map_val(chunk[0]);
            *out = Rgba8Pixel { r: g8, g: g8, b: g8, a: 255 };
        });

        if let Some(p) = progress { p.finish(Some("Depth processed")); }
        Image::from_rgba8(buffer)
    }
    
    /// Get memory usage statistics for monitoring
    #[allow(dead_code)]
    pub fn get_memory_stats(&self) -> String {
        match &self.data_source {
            ExrDataSource::Full(_) => {
                let current_mb = (self.raw_pixels.len() * 4) / (1024 * 1024);
                let mip_mb: usize = self.mip_levels.iter()
                    .map(|mip| (mip.pixels.len() * 4) / (1024 * 1024))
                    .sum();
                format!("Full cache mode: {}MB current + {}MB MIPs", current_mb, mip_mb)
            },
            ExrDataSource::Lazy(loader) => {
                let (cached_layers, total_layers, cached_mb) = loader.get_cache_stats();
                let current_mb = (self.raw_pixels.len() * 4) / (1024 * 1024);
                let mip_mb: usize = self.mip_levels.iter()
                    .map(|mip| (mip.pixels.len() * 4) / (1024 * 1024))
                    .sum();
                format!("Lazy mode: {}/{} layers cached ({}MB) + current {}MB + MIPs {}MB", 
                       cached_layers, total_layers, cached_mb, current_mb, mip_mb)
            }
        }
    }
    
    /// Clear cached data to free memory (only works in lazy mode)
    #[allow(dead_code)]
    pub fn clear_data_cache(&self) {
        if let ExrDataSource::Lazy(loader) = &self.data_source {
            loader.clear_cache();
        }
    }
}


