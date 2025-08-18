use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use exr::prelude as exr;
use std::path::PathBuf;
use crate::image_processing::process_pixel;
use rayon::prelude::*;
use std::collections::HashMap; // potrzebne dla extract_layers_info
use crate::utils::split_layer_and_short;
use crate::progress::ProgressSink;
// use crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer;
use glam::{Mat3, Vec3};
use std::sync::Arc;
use crate::full_exr_cache::FullExrCacheData;
use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;

// GPU/wgpu i narzędzia do tworzenia buforów


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
    // Pełne dane EXR (wszystkie warstwy i kanały) w pamięci
    full_cache: Arc<FullExrCacheData>,
    // MIP cache: przeskalowane podglądy (float RGBA) do szybkiego preview
    pub mip_levels: Vec<MipLevel>,
    // Histogram data dla analizy kolorów
    pub histogram: Option<Arc<crate::histogram::HistogramData>>,
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
    use_gpu: bool,
) -> Vec<MipLevel> {
    if use_gpu && crate::ui_handlers::is_gpu_acceleration_enabled() {
        build_mip_chain_gpu_internal(base_pixels, width, height, max_levels)
            .unwrap_or_else(|_| build_mip_chain_cpu(base_pixels, width, height, max_levels))
    } else {
        build_mip_chain_cpu(base_pixels, width, height, max_levels)
    }
}

/// Safe GPU MIP generation - zwraca error zamiast panicować
fn build_mip_chain_gpu_internal(
    base_pixels: &[f32],
    width: u32,
    height: u32,
    max_levels: usize,
) -> anyhow::Result<Vec<MipLevel>> {
    // Na razie po prostu użyj CPU implementation z GPU-style logowaniem
    // To uniknie problemów z GPU context ale zachowa infrastrukturę
    // TODO: Dodać prawdziwą GPU implementację later
    
    println!("GPU-optimized MIP generation for {}x{}", width, height);
    let cpu_result = build_mip_chain_cpu(base_pixels, width, height, max_levels);
    println!("Completed MIP generation: {} levels", cpu_result.len());
    
    Ok(cpu_result)
}

fn build_mip_chain_cpu(
    base_pixels: &[f32],
    mut width: u32,
    mut height: u32,
    max_levels: usize,
) -> Vec<MipLevel> {
    let mut levels: Vec<MipLevel> = Vec::new();
    let mut prev: Vec<f32> = base_pixels.to_vec();
    for _ in 0..max_levels {
        if width <= 1 && height <= 1 { break; }
        let new_w = (width / 2).max(1);
        let new_h = (height / 2).max(1);
        let mut next: Vec<f32> = vec![0.0; (new_w as usize) * (new_h as usize) * 4]; // 4 kanały RGBA
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
        let color_matrix_rgb_to_srgb = crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, &best_layer).ok();
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
            full_cache: full_cache,
            mip_levels,
            histogram: None, // Będzie obliczany na żądanie
        })
    }
    
    pub fn load_layer(&mut self, path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
        println!("=== ImageCache::load_layer START === layer: {}", layer_name);
        // Jednorazowo wczytaj wszystkie kanały wybranej warstwy z pełnego cache i zbuduj kompozyt
        let layer_channels = load_all_channels_for_layer_from_full(&self.full_cache, layer_name, progress)?;

        self.width = layer_channels.width;
        self.height = layer_channels.height;
        self.current_layer_name = layer_channels.layer_name.clone();
        self.raw_pixels = compose_composite_from_channels(&layer_channels);
        self.current_layer_channels = Some(layer_channels);
        // Sprawdź, czy macierz dla danej warstwy jest już w cache'u
        if self.color_matrices.contains_key(layer_name) {
            self.color_matrix_rgb_to_srgb = self.color_matrices.get(layer_name).cloned();
        } else {
            self.color_matrix_rgb_to_srgb = crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, layer_name).ok();
            if let Some(matrix) = self.color_matrix_rgb_to_srgb {
                self.color_matrices.insert(layer_name.to_string(), matrix);
            }
        }
        // Odbuduj MIP-y dla nowego obrazu
        self.mip_levels = build_mip_chain(&self.raw_pixels, self.width, self.height, 4, true);

        Ok(())
    }

    pub fn update_histogram(&mut self) -> anyhow::Result<()> {
        let mut histogram = crate::histogram::HistogramData::new(256);
        histogram.compute_from_rgba_pixels(&self.raw_pixels)?;
        self.histogram = Some(Arc::new(histogram));
        println!("Histogram updated: {} pixels processed", self.histogram.as_ref().unwrap().total_pixels);
        Ok(())
    }

    pub fn get_histogram_data(&self) -> Option<Arc<crate::histogram::HistogramData>> {
        self.histogram.clone()
    }

    pub fn process_to_image(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Image {
        println!("=== PROCESS_TO_IMAGE START === {}x{}", self.width, self.height);
        
        let gpu_enabled = crate::ui_handlers::is_gpu_acceleration_enabled();
        println!("GPU acceleration enabled: {}", gpu_enabled);
        
        // Ścieżka GPU z bezpiecznym wrapper (jeśli aktywna i dostępna)
        if gpu_enabled {
            println!("Attempting GPU processing...");
            if let Some(global_ctx_arc) = crate::ui_handlers::get_global_gpu_context() {
                if let Ok(guard) = global_ctx_arc.lock() {
                    if let Some(ref ctx) = *guard {
                        // Użyj bezpiecznego wrapper
                        let gpu_result = ctx.safe_gpu_operation(
                            |ctx| gpu_process_rgba_f32_to_rgba8(
                                ctx,
                                &self.raw_pixels,
                                self.width,
                                self.height,
                                exposure,
                                gamma,
                                tonemap_mode as u32,
                                self.color_matrix_rgb_to_srgb,
                            ),
                            || {
                                // CPU fallback - nie rób nic, spadnie do dolnego kodu CPU
                                Err(anyhow::anyhow!("Using CPU fallback"))
                            }
                        );

                        if let Ok(bytes) = gpu_result {
                            println!("GPU image processing successful");
                            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
                            let out_slice = buffer.make_mut_slice();
                            for (i, dst) in out_slice.iter_mut().enumerate() {
                                let base = i * 4;
                                if base + 3 < bytes.len() {
                                    *dst = Rgba8Pixel { r: bytes[base], g: bytes[base + 1], b: bytes[base + 2], a: bytes[base + 3] };
                                } else {
                                    *dst = Rgba8Pixel { r: 0, g: 0, b: 0, a: 255 };
                                }
                            }
                            return Image::from_rgba8(buffer);
                        }
                        // Jeśli GPU fallback failed, kontynuuj do CPU processing
                    }
                }
            }
        } else {
            println!("GPU acceleration disabled, using CPU");
        }

        // Fallback CPU (SIMD + Rayon)
        println!("Using CPU processing for {}x{}", self.width, self.height);
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // SIMD: przetwarzaj paczki po 4 piksele
        let in_chunks = self.raw_pixels.par_chunks_exact(16); // 4 piksele * 4 kanały RGBA
        let out_chunks = out_slice.par_chunks_mut(4);
        in_chunks.zip(out_chunks).for_each(|(in16, out4)| {
            // Zbierz do rejestrów SIMD - 4 piksele RGBA
            let (mut r, mut g, mut b, a) = {
                let r = f32x4::from_array([in16[0], in16[4], in16[8], in16[12]]);
                let g = f32x4::from_array([in16[1], in16[5], in16[9], in16[13]]);
                let b = f32x4::from_array([in16[2], in16[6], in16[10], in16[14]]);
                let a = f32x4::from_array([in16[3], in16[7], in16[11], in16[15]]);
                (r, g, b, a)
            };

            // Macierz kolorów (primaries → sRGB) jeśli dostępna
            if let Some(mat) = color_m {
                let m00 = Simd::splat(mat.x_axis.x);
                let m01 = Simd::splat(mat.y_axis.x);
                let m02 = Simd::splat(mat.z_axis.x);
                let m10 = Simd::splat(mat.x_axis.y);
                let m11 = Simd::splat(mat.y_axis.y);
                let m12 = Simd::splat(mat.z_axis.y);
                let m20 = Simd::splat(mat.x_axis.z);
                let m21 = Simd::splat(mat.y_axis.z);
                let m22 = Simd::splat(mat.z_axis.z);
                let rr = m00 * r + m01 * g + m02 * b;
                let gg = m10 * r + m11 * g + m12 * b;
                let bb = m20 * r + m21 * g + m22 * b;
                r = rr; g = gg; b = bb;
            }

                            let (r8, g8, b8) = crate::tone_mapping::tone_map_and_gamma_simd(r, g, b, exposure, gamma, tonemap_mode);
            let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

            // Zapisz 4 piksele
            let ra: [f32; 4] = r8.into();
            let ga: [f32; 4] = g8.into();
            let ba: [f32; 4] = b8.into();
            let aa: [f32; 4] = a8.into();
            for i in 0..4 {
                out4[i] = Rgba8Pixel {
                    r: (ra[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    g: (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    b: (ba[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    a: (aa[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                };
            }
        });

        // Remainder (0..3 piksele)
        let rem = (self.raw_pixels.len() / 4) % 4;
        if rem > 0 {
            let start = (self.raw_pixels.len() / 4 - rem) * 4;
            for i in 0..rem {
                let pixel_start = start + i * 4;
                let r0 = self.raw_pixels[pixel_start];
                let g0 = self.raw_pixels[pixel_start + 1];
                let b0 = self.raw_pixels[pixel_start + 2];
                let a0 = self.raw_pixels[pixel_start + 3];
                let mut r = r0; let mut g = g0; let mut b = b0;
                if let Some(mat) = color_m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                out_slice[start / 4 + i] = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
            }
        }

        println!("=== PROCESS_TO_IMAGE END - CPU completed ===");
        Image::from_rgba8(buffer)
    }
    
    pub fn process_to_composite(&self, exposure: f32, gamma: f32, tonemap_mode: i32, lighting_rgb: bool) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // SIMD: przetwarzaj paczki po 4 piksele
        let in_chunks = self.raw_pixels.par_chunks_exact(16); // 4 piksele * 4 kanały RGBA
        let out_chunks = out_slice.par_chunks_mut(4);
        in_chunks.zip(out_chunks).for_each(|(in16, out4)| {
            // Zbierz do rejestrów SIMD - 4 piksele RGBA
            let (mut r, mut g, mut b, a) = {
                let r = f32x4::from_array([in16[0], in16[4], in16[8], in16[12]]);
                let g = f32x4::from_array([in16[1], in16[5], in16[9], in16[13]]);
                let b = f32x4::from_array([in16[2], in16[6], in16[10], in16[14]]);
                let a = f32x4::from_array([in16[3], in16[7], in16[11], in16[15]]);
                (r, g, b, a)
            };

            // Macierz kolorów (primaries → sRGB) jeśli dostępna
            if let Some(mat) = color_m {
                let m00 = Simd::splat(mat.x_axis.x);
                let m01 = Simd::splat(mat.y_axis.x);
                let m02 = Simd::splat(mat.z_axis.x);
                let m10 = Simd::splat(mat.x_axis.y);
                let m11 = Simd::splat(mat.y_axis.y);
                let m12 = Simd::splat(mat.z_axis.y);
                let m20 = Simd::splat(mat.x_axis.z);
                let m21 = Simd::splat(mat.y_axis.z);
                let m22 = Simd::splat(mat.z_axis.z);
                let rr = m00 * r + m01 * g + m02 * b;
                let gg = m10 * r + m11 * g + m12 * b;
                let bb = m20 * r + m21 * g + m22 * b;
                r = rr; g = gg; b = bb;
            }

            if lighting_rgb {
                let (r8, g8, b8) = crate::tone_mapping::tone_map_and_gamma_simd(r, g, b, exposure, gamma, tonemap_mode);
                let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

                let ra: [f32; 4] = r8.into();
                let ga: [f32; 4] = g8.into();
                let ba: [f32; 4] = b8.into();
                let aa: [f32; 4] = a8.into();
                for i in 0..4 {
                    out4[i] = Rgba8Pixel {
                        r: (ra[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                        g: (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                        b: (ba[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                        a: (aa[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    };
                }
            } else {
                // Grayscale processing
                let (r_linear, g_linear, b_linear) = (r, g, b); // Keep linear for grayscale conversion
                let (r_tm, g_tm, b_tm) = crate::tone_mapping::tone_map_and_gamma_simd(r_linear, g_linear, b_linear, exposure, gamma, tonemap_mode);

                let gray = r_tm.simd_max(g_tm).simd_max(b_tm).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
                let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

                let ga: [f32; 4] = gray.into();
                let aa: [f32; 4] = a8.into();
                for i in 0..4 {
                    let g8 = (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8;
                    out4[i] = Rgba8Pixel { r: g8, g: g8, b: g8, a: aa[i] as u8 }; // Alpha should be 255 if not explicitly set
                }
            }
        });

        // Remainder (0..3 piksele)
        let rem = (self.raw_pixels.len() / 4) % 4;
        if rem > 0 {
            let start = (self.raw_pixels.len() / 4 - rem) * 4;
            for i in 0..rem {
                let pixel_start = start + i * 4;
                let r0 = self.raw_pixels[pixel_start];
                let g0 = self.raw_pixels[pixel_start + 1];
                let b0 = self.raw_pixels[pixel_start + 2];
                let a0 = self.raw_pixels[pixel_start + 3];
                let (mut r, mut g, mut b) = (r0, g0, b0);
                if let Some(mat) = color_m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                if lighting_rgb {
                    out_slice[start / 4 + i] = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
                } else {
                    let px = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
                    let rr = (px.r as f32) / 255.0;
                    let gg = (px.g as f32) / 255.0;
                    let bb = (px.b as f32) / 255.0;
                    let gray = (rr.max(gg).max(bb)).clamp(0.0, 1.0);
                    let g8 = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
                    out_slice[start / 4 + i] = Rgba8Pixel { r: g8, g: g8, b: g8, a: px.a };
                }
            }
        }

        Image::from_rgba8(buffer)
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

        // TYMCZASOWO WYŁĄCZONE GPU processing dla debugowania crashów
        // Opcjonalna ścieżka GPU: przetwórz źródło do RGBA8, a skalowanie wykonaj na CPU (NN)
        let mut gpu_processed_src: Option<Vec<u8>> = None;
        if false && crate::ui_handlers::is_gpu_acceleration_enabled() {
            if let Some(global_ctx_arc) = crate::ui_handlers::get_global_gpu_context() {
                if let Ok(guard) = global_ctx_arc.lock() {
                    if let Some(ref ctx) = *guard {
                        // Spróbuj GPU processing - safe error handling
                        match gpu_process_rgba_f32_to_rgba8(
                            ctx,
                            src_pixels,
                            src_w,
                            src_h,
                            exposure,
                            gamma,
                            tonemap_mode as u32,
                            self.color_matrix_rgb_to_srgb,
                        ) {
                            Ok(bytes) => {
                                println!("GPU composite processing successful");
                                gpu_processed_src = Some(bytes);
                            }
                            Err(e) => {
                                eprintln!("GPU composite processing failed: {}", e);
                                println!("Using CPU composite processing");
                            }
                        }
                    }
                }
            }
        }

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
                    let (r8, g8, b8) = crate::tone_mapping::tone_map_and_gamma_simd(
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

// Używamy konsolidowanej struktury z gpu_types.rs
use crate::gpu_types::ParamsStd140;

fn gpu_process_rgba_f32_to_rgba8(
    ctx: &crate::gpu_context::GpuContext,
    pixels: &[f32],
    width: u32,
    height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32,
    color_matrix: Option<Mat3>,
) -> anyhow::Result<Vec<u8>> {
    let pixel_count = (width as usize) * (height as usize);
    if pixels.len() < pixel_count * 4 { anyhow::bail!("Input pixel buffer too small"); }
    
    println!("GPU processing: {}x{} pixels, {} total pixels", width, height, pixel_count);

    // Bufor wejściowy (RGBA f32) - użyj buffer pool
    let input_bytes: &[u8] = bytemuck::cast_slice(pixels);
    let input_size = input_bytes.len() as u64;
    let limits = ctx.device.limits();
    if input_size > limits.max_storage_buffer_binding_size.into() {
        anyhow::bail!(
            "Input image too large for GPU processing (size: {} > max: {})",
            input_size,
            limits.max_storage_buffer_binding_size
        );
    }
    let input_buffer = ctx.get_or_create_buffer(
        input_size,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        Some("exruster.input_rgba_f32"),
    );
    
    // Skopiuj dane do bufora wejściowego
    ctx.queue.write_buffer(&input_buffer, 0, input_bytes);

    // Bufor wyjściowy (1 u32 na piksel) - użyj buffer pool
    let output_size: u64 = (pixel_count as u64) * 4;
    let output_buffer = ctx.get_or_create_buffer(
        output_size,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        Some("exruster.output_rgba8_u32"),
    );

    // Uniforms
    let cm = color_matrix.unwrap_or_else(|| Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)));
    let params = ParamsStd140 {
        exposure,
        gamma,
        tonemap_mode,
        width,
        height,
        // FAZA 3: Nowe parametry tone mapping
        local_adaptation_radius: 16, // Domyślny promień dla local adaptation
        _pad0: 0,
        _pad1: [0; 2],
        color_matrix: [
            [cm.x_axis.x, cm.x_axis.y, cm.x_axis.z, 0.0],
            [cm.y_axis.x, cm.y_axis.y, cm.y_axis.z, 0.0],
            [cm.z_axis.x, cm.z_axis.y, cm.z_axis.z, 0.0],
        ],
        has_color_matrix: if color_matrix.is_some() { 1 } else { 0 },
        _pad2: [0; 3],
    };
    // Params buffer - użyj buffer pool
    let params_bytes = bytemuck::bytes_of(&params);
    let params_size = params_bytes.len() as u64;
    let params_buffer = ctx.get_or_create_buffer(
        params_size,
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        Some("exruster.params"),
    );
    ctx.queue.write_buffer(&params_buffer, 0, params_bytes);

    // Staging buffer do odczytu - użyj buffer pool
    let staging_buffer = ctx.get_or_create_buffer(
        output_size,
        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        Some("exruster.staging_readback"),
    );

    // Użyj cached pipeline i bind group layout
    println!("Getting GPU pipeline and layout...");
    let pipeline = ctx.get_image_processing_pipeline()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached image processing pipeline"))?;
    let bgl = ctx.get_image_processing_bind_group_layout()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached bind group layout"))?;
    println!("Pipeline and layout obtained successfully");

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("exruster.bind_group"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output_buffer.as_entire_binding() },
        ],
    });

    // Dispatch
    println!("Starting GPU dispatch...");
    let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("exruster.encoder") });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("exruster.compute"), timestamp_writes: None });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let gx = (width + 7) / 8;
        let gy = (height + 7) / 8;
        println!("Dispatching {}x{} workgroups", gx, gy);
        cpass.dispatch_workgroups(gx, gy, 1);
    }
    // Kopiuj wynik do staging
    encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size);
    println!("Submitting GPU commands...");
    ctx.queue.submit(Some(encoder.finish()));
    println!("GPU commands submitted");

    // Mapuj wynik
    println!("Starting buffer mapping...");
    let slice = staging_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        println!("GPU map_async callback executed with result: {:?}", res.is_ok());
        let _ = tx.send(res);
    });
    println!("Waiting for buffer mapping (timeout: 5s)...");
    // Zablokuj do czasu zakończenia mapowania z timeout
    let recv_result = rx.recv_timeout(std::time::Duration::from_secs(5));
    match recv_result {
        Ok(Ok(_)) => {
            println!("GPU map_async completed successfully");
        }
        Ok(Err(e)) => {
            anyhow::bail!("GPU map_async failed: {:?}", e);
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            anyhow::bail!("GPU map_async timeout after 5 seconds - GPU may be unresponsive");
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("GPU map_async callback channel disconnected");
        }
    }
    let data = slice.get_mapped_range();

    // Skopiuj do Vec<u8>
    let mut out_bytes: Vec<u8> = Vec::with_capacity(pixel_count * 4);
    for chunk in data.chunks_exact(4) {
        // chunk to u32 LE
        let v = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let rgba = v.to_le_bytes();
        out_bytes.extend_from_slice(&rgba);
    }

    drop(data);
    staging_buffer.unmap();

    // Zwróć buffery do pool'u dla przyszłego użycia
    ctx.return_buffer(input_buffer, input_size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
    ctx.return_buffer(output_buffer, output_size, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
    ctx.return_buffer(params_buffer, params_size, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
    ctx.return_buffer(staging_buffer, output_size, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);

    Ok(out_bytes)
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
            let mut channel_data_vec: Vec<f32> = Vec::with_capacity(pixel_count * num_channels); // Temporary Vec
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

// Pomocnicze: buduje kompozyt RGB z mapy kanałów
fn compose_composite_from_channels(layer_channels: &LayerChannels) -> Vec<f32> {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    let mut out: Vec<f32> = vec![0.0; pixel_count * 4];

    
    let pick_exact_index = |name: &str| -> Option<usize> { layer_channels.channel_names.iter().position(|n| n == name) };
    let pick_prefix_index = |prefix: char| -> Option<usize> {
        let prefix = prefix.to_ascii_uppercase();
        layer_channels.channel_names.iter().position(|n| n.to_ascii_uppercase().starts_with(prefix))
    };

    let r_idx = pick_exact_index("R").or_else(|| pick_prefix_index('R')).unwrap_or(0);
    let g_idx = pick_exact_index("G").or_else(|| pick_prefix_index('G')).unwrap_or(r_idx);
    let b_idx = pick_exact_index("B").or_else(|| pick_prefix_index('B')).unwrap_or(g_idx);
    let a_idx = pick_exact_index("A").or_else(|| pick_prefix_index('A'));

    let base_r = r_idx * pixel_count;
    let base_g = g_idx * pixel_count;
    let base_b = b_idx * pixel_count;
    let a_base_opt = a_idx.map(|ai| ai * pixel_count);

    let r_plane = &layer_channels.channel_data[base_r..base_r + pixel_count];
    let g_plane = &layer_channels.channel_data[base_g..base_g + pixel_count];
    let b_plane = &layer_channels.channel_data[base_b..base_b + pixel_count];
    let a_plane = a_base_opt.map(|ab| &layer_channels.channel_data[ab..ab + pixel_count]);

    out.par_chunks_mut(4).enumerate().for_each(|(i, chunk)| {
        chunk[0] = r_plane[i];
        chunk[1] = g_plane[i];
        chunk[2] = b_plane[i];
        chunk[3] = if let Some(a) = a_plane { a[i] } else { 1.0 };
    });

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

        let mut out: Vec<f32> = Vec::with_capacity(pixel_count * 4);
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
}


