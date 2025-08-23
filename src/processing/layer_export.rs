use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use rayon::prelude::*;
use crate::io::image_cache::{LayerInfo, LayerChannels};
use crate::io::full_exr_cache::FullExrCacheData;
use crate::processing::tone_mapping::{tone_map_and_gamma, ToneMapMode};

/// Layer export configuration types
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum LayerExportConfig {
    /// Export only the base/beauty layer
    BaseOnly,
}

/// Export format configuration
#[derive(Clone, Debug)]
pub enum ExportFormat {
    /// PNG 16-bit per channel
    Png16,
    /// TIFF 16-bit per channel
    Tiff16,
    /// TIFF 32-bit float per channel
    Tiff32Float,
}

/// Export processing parameters
#[derive(Clone, Debug)]
pub struct ExportParams {
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: ToneMapMode,
}

impl Default for ExportParams {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            gamma: 2.2,
            tonemap_mode: ToneMapMode::ACES,
        }
    }
}

/// High-performance layer export processor
pub struct LayerExporter {
    cache: Arc<FullExrCacheData>,
    layers_info: Vec<LayerInfo>,
    export_params: ExportParams,
}

impl LayerExporter {
    /// Create a new layer exporter with full EXR cache
    pub fn new(cache: Arc<FullExrCacheData>, layers_info: Vec<LayerInfo>) -> Self {
        Self {
            cache,
            layers_info,
            export_params: ExportParams::default(),
        }
    }

    /// Set export processing parameters
    pub fn with_params(mut self, params: ExportParams) -> Self {
        self.export_params = params;
        self
    }

    /// Export layers based on configuration

    /// Export the base layer specifically
    pub fn export_base_layer(
        &self,
        format: ExportFormat,
        output_dir: &PathBuf,
        base_filename: &str,
    ) -> Result<PathBuf> {
        let base_layer = self.find_base_layer()?;
        self.export_single_layer(&base_layer, &format, output_dir, base_filename)
    }


    /// Find the base/beauty layer
    fn find_base_layer(&self) -> Result<LayerInfo> {
        // Use the same logic as image_cache.rs find_best_layer
        let base_layer_name = crate::io::image_cache::find_best_layer(&self.layers_info);
        
        self.layers_info
            .iter()
            .find(|layer| layer.name == base_layer_name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Base layer not found: {}", base_layer_name))
    }


    /// Export a single layer with high performance
    fn export_single_layer(
        &self,
        layer_info: &LayerInfo,
        format: &ExportFormat,
        output_dir: &PathBuf,
        base_filename: &str,
    ) -> Result<PathBuf> {
        // Load layer channels from cache
        let layer_channels = self.load_layer_channels(&layer_info.name)?;
        
        // Process layer to RGB/RGBA pixels
        let processed_pixels = self.process_layer_to_pixels(&layer_channels)?;
        
        // Generate output filename
        let output_path = self.generate_output_path(
            output_dir, 
            base_filename, 
            &layer_info.name, 
            format
        )?;
        
        // Save to file based on format
        self.save_processed_pixels(&processed_pixels, &layer_channels, format, &output_path)?;
        
        Ok(output_path)
    }

    /// Load layer channels from full cache
    fn load_layer_channels(&self, layer_name: &str) -> Result<LayerChannels> {
        crate::io::image_cache::load_all_channels_for_layer_from_full(
            &self.cache,
            layer_name,
            None
        )
    }

    /// Process layer to final pixels with tone mapping and color correction
    fn process_layer_to_pixels(&self, layer_channels: &LayerChannels) -> Result<ProcessedPixels> {
        let pixel_count = (layer_channels.width * layer_channels.height) as usize;
        
        // Compose RGB from channels
        let rgb_pixels = self.compose_rgb_from_channels(layer_channels);
        
        // Apply tone mapping and gamma correction in parallel
        let has_alpha = layer_channels.channel_names.iter().any(|name| {
            let n = name.to_ascii_uppercase();
            n == "A" || n.starts_with("ALPHA")
        });
        
        let channels = if has_alpha { 4 } else { 3 };
        
        let processed_data = match layer_channels.channel_names.len() {
            1 => {
                // Grayscale channel - expand to RGB
                self.process_grayscale_pixels(&rgb_pixels, pixel_count)?
            }
            _ => {
                // RGB/RGBA channels - full color processing
                self.process_color_pixels(&rgb_pixels, pixel_count, has_alpha)?
            }
        };
        
        Ok(ProcessedPixels {
            data: processed_data,
            width: layer_channels.width,
            height: layer_channels.height,
            channels,
        })
    }

    /// Compose RGB from layer channels (highly optimized version)
    fn compose_rgb_from_channels(&self, layer_channels: &LayerChannels) -> Vec<f32> {
        let pixel_count = (layer_channels.width * layer_channels.height) as usize;
        
        // Allocate buffer for RGB composition
        let buffer_size = pixel_count * 4;
        let mut rgb_pixels = Vec::with_capacity(buffer_size);

        // Find RGB and Alpha channels with optimized lookup
        let find_channel = |name: &str| -> Option<usize> {
            let name_upper = name.to_ascii_uppercase();
            layer_channels.channel_names.iter().position(|n| {
                let n_upper = n.to_ascii_uppercase();
                n_upper == name_upper || n_upper.starts_with(&name_upper)
            })
        };

        let r_idx = find_channel("R").unwrap_or(0);
        let g_idx = find_channel("G").unwrap_or(r_idx);
        let b_idx = find_channel("B").unwrap_or(g_idx);
        let a_idx = find_channel("A");

        // Extract channel slices with bounds checking
        let channel_data = &layer_channels.channel_data;
        let r_slice = &channel_data[r_idx * pixel_count..(r_idx + 1) * pixel_count];
        let g_slice = &channel_data[g_idx * pixel_count..(g_idx + 1) * pixel_count];
        let b_slice = &channel_data[b_idx * pixel_count..(b_idx + 1) * pixel_count];
        let a_slice = a_idx.map(|idx| {
            &channel_data[idx * pixel_count..(idx + 1) * pixel_count]
        });

        // High-performance SIMD composition when available
        #[cfg(feature = "unified_simd")]
        {
            self.compose_rgb_simd_optimized(
                &mut rgb_pixels, 
                r_slice, 
                g_slice, 
                b_slice, 
                a_slice, 
                pixel_count
            );
        }

        #[cfg(not(feature = "unified_simd"))]
        {
            // Fallback to safe parallel composition
            rgb_pixels.resize(buffer_size, 0.0);
            rgb_pixels.par_chunks_exact_mut(4)
                .enumerate()
                .for_each(|(i, chunk)| {
                    chunk[0] = r_slice[i];
                    chunk[1] = g_slice[i];
                    chunk[2] = b_slice[i];
                    chunk[3] = a_slice.map_or(1.0, |a| a[i]);
                });
        }

        rgb_pixels
    }

    #[cfg(feature = "unified_simd")]
    /// SIMD-optimized RGB composition for maximum performance
    fn compose_rgb_simd_optimized(
        &self,
        rgb_pixels: &mut Vec<f32>,
        r_slice: &[f32],
        g_slice: &[f32],
        b_slice: &[f32],
        a_slice: Option<&[f32]>,
        pixel_count: usize,
    ) {
        use std::simd::{f32x4, Simd};

        let buffer_size = pixel_count * 4;
        unsafe {
            rgb_pixels.set_len(buffer_size);
        }

        let chunks = pixel_count / 4;
        let remainder = pixel_count % 4;

        // Process in SIMD chunks of 4 pixels
        for chunk_idx in 0..chunks {
            let base_pixel = chunk_idx * 4;
            let base_output = base_pixel * 4;

            // Load 4 values from each channel
            let r_chunk = f32x4::from_slice(&r_slice[base_pixel..base_pixel + 4]);
            let g_chunk = f32x4::from_slice(&g_slice[base_pixel..base_pixel + 4]);
            let b_chunk = f32x4::from_slice(&b_slice[base_pixel..base_pixel + 4]);
            let a_chunk = if let Some(a) = a_slice {
                f32x4::from_slice(&a[base_pixel..base_pixel + 4])
            } else {
                f32x4::splat(1.0)
            };

            // Interleave RGBA data efficiently
            for i in 0..4 {
                rgb_pixels[base_output + i * 4] = r_chunk[i];
                rgb_pixels[base_output + i * 4 + 1] = g_chunk[i];
                rgb_pixels[base_output + i * 4 + 2] = b_chunk[i];
                rgb_pixels[base_output + i * 4 + 3] = a_chunk[i];
            }
        }

        // Handle remaining pixels
        for i in 0..remainder {
            let pixel_idx = chunks * 4 + i;
            let output_idx = pixel_idx * 4;
            
            rgb_pixels[output_idx] = r_slice[pixel_idx];
            rgb_pixels[output_idx + 1] = g_slice[pixel_idx];
            rgb_pixels[output_idx + 2] = b_slice[pixel_idx];
            rgb_pixels[output_idx + 3] = a_slice.map_or(1.0, |a| a[pixel_idx]);
        }
    }

    /// Process grayscale pixels with tone mapping
    fn process_grayscale_pixels(&self, pixels: &[f32], pixel_count: usize) -> Result<Vec<u16>> {
        let mut processed = Vec::with_capacity(pixel_count * 3);
        
        for chunk in pixels.chunks_exact(4) {
            let gray_value = chunk[0];
            let (processed_gray, _, _) = tone_map_and_gamma(
                gray_value,
                gray_value,
                gray_value,
                self.export_params.exposure,
                self.export_params.gamma,
                self.export_params.tonemap_mode,
            );
            
            let u16_value = (processed_gray.clamp(0.0, 1.0) * 65535.0).round() as u16;
            processed.extend_from_slice(&[u16_value, u16_value, u16_value]);
        }
        
        Ok(processed)
    }

    /// Process color pixels with tone mapping and color correction (SIMD optimized)
    fn process_color_pixels(&self, pixels: &[f32], pixel_count: usize, has_alpha: bool) -> Result<Vec<u16>> {
        let output_channels = if has_alpha { 4 } else { 3 };
        let mut processed = Vec::with_capacity(pixel_count * output_channels);
        
        // Use SIMD-optimized processing when feature is enabled
        #[cfg(feature = "unified_simd")]
        {
            processed.extend(self.process_color_pixels_simd_optimized(pixels, has_alpha));
        }
        
        #[cfg(not(feature = "unified_simd"))]
        {
            for chunk in pixels.chunks_exact(4) {
                let (r, g, b) = tone_map_and_gamma(
                    chunk[0],
                    chunk[1],
                    chunk[2],
                    self.export_params.exposure,
                    self.export_params.gamma,
                    self.export_params.tonemap_mode,
                );
                
                let r_u16 = (r.clamp(0.0, 1.0) * 65535.0).round() as u16;
                let g_u16 = (g.clamp(0.0, 1.0) * 65535.0).round() as u16;
                let b_u16 = (b.clamp(0.0, 1.0) * 65535.0).round() as u16;
                
                if has_alpha {
                    let a_u16 = (chunk[3].clamp(0.0, 1.0) * 65535.0).round() as u16;
                    processed.extend_from_slice(&[r_u16, g_u16, b_u16, a_u16]);
                } else {
                    processed.extend_from_slice(&[r_u16, g_u16, b_u16]);
                }
            }
        }
        
        Ok(processed)
    }

    #[cfg(feature = "unified_simd")]
    /// SIMD-optimized color pixel processing
    fn process_color_pixels_simd_optimized(&self, pixels: &[f32], has_alpha: bool) -> Vec<u16> {
        use std::simd::{f32x4, Simd, SimdFloat, SimdUint, u16x4};
        
        let exposure_multiplier = 2.0f32.powf(self.export_params.exposure);
        let gamma_inv = 1.0 / self.export_params.gamma;
        
        // Process pixels in SIMD chunks of 4
        pixels.par_chunks_exact(16) // 4 pixels * 4 channels = 16 floats
            .flat_map(|chunk_16| {
                // Load 4 RGBA pixels at once
                let pixel_data: [f32x4; 4] = [
                    f32x4::from_slice(&chunk_16[0..4]),   // Pixel 0: RGBA
                    f32x4::from_slice(&chunk_16[4..8]),   // Pixel 1: RGBA
                    f32x4::from_slice(&chunk_16[8..12]),  // Pixel 2: RGBA
                    f32x4::from_slice(&chunk_16[12..16]), // Pixel 3: RGBA
                ];
                
                let mut results = Vec::with_capacity(if has_alpha { 16 } else { 12 });
                
                for pixel_rgba in pixel_data {
                    // Apply exposure
                    let exposed = pixel_rgba * f32x4::splat(exposure_multiplier);
                    
                    // Extract RGB channels for tone mapping
                    let r = exposed[0];
                    let g = exposed[1];
                    let b = exposed[2];
                    let a = exposed[3];
                    
                    // Apply tone mapping (scalar for now, could be vectorized further)
                    let (tone_r, tone_g, tone_b) = tone_map_and_gamma(
                        r, g, b,
                        0.0, // exposure already applied
                        self.export_params.gamma,
                        self.export_params.tonemap_mode,
                    );
                    
                    // Convert to u16
                    let r_u16 = (tone_r.clamp(0.0, 1.0) * 65535.0).round() as u16;
                    let g_u16 = (tone_g.clamp(0.0, 1.0) * 65535.0).round() as u16;
                    let b_u16 = (tone_b.clamp(0.0, 1.0) * 65535.0).round() as u16;
                    
                    results.push(r_u16);
                    results.push(g_u16);
                    results.push(b_u16);
                    
                    if has_alpha {
                        let a_u16 = (a.clamp(0.0, 1.0) * 65535.0).round() as u16;
                        results.push(a_u16);
                    }
                }
                
                results
            })
            .collect()
    }

    /// Generate output file path
    fn generate_output_path(
        &self,
        output_dir: &PathBuf,
        base_filename: &str,
        layer_name: &str,
        format: &ExportFormat,
    ) -> Result<PathBuf> {
        let extension = match format {
            ExportFormat::Png16 => "png",
            ExportFormat::Tiff16 | ExportFormat::Tiff32Float => "tiff",
        };

        let sanitized_layer_name = layer_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let filename = if layer_name.is_empty() {
            format!("{}_base.{}", base_filename, extension)
        } else {
            format!("{}_{}.{}", base_filename, sanitized_layer_name, extension)
        };

        Ok(output_dir.join(filename))
    }

    /// Save processed pixels to file
    fn save_processed_pixels(
        &self,
        pixels: &ProcessedPixels,
        _layer_channels: &LayerChannels,
        format: &ExportFormat,
        output_path: &PathBuf,
    ) -> Result<()> {
        std::fs::create_dir_all(output_path.parent().unwrap())?;

        match format {
            ExportFormat::Png16 => {
                self.save_png16(pixels, output_path)?;
            }
            ExportFormat::Tiff16 => {
                self.save_tiff16(pixels, output_path)?;
            }
            ExportFormat::Tiff32Float => {
                self.save_tiff32_float(pixels, output_path)?;
            }
        }

        Ok(())
    }

    /// Save as PNG 16-bit
    fn save_png16(&self, pixels: &ProcessedPixels, output_path: &PathBuf) -> Result<()> {
        use image::{ImageBuffer, Rgba};

        if pixels.channels == 4 {
            let img_buffer = ImageBuffer::<Rgba<u16>, _>::from_raw(
                pixels.width,
                pixels.height,
                pixels.data.clone(),
            ).ok_or_else(|| anyhow::anyhow!("Failed to create RGBA image buffer"))?;

            img_buffer.save(output_path)?;
        } else {
            let img_buffer = ImageBuffer::<image::Rgb<u16>, _>::from_raw(
                pixels.width,
                pixels.height,
                pixels.data.clone(),
            ).ok_or_else(|| anyhow::anyhow!("Failed to create RGB image buffer"))?;

            img_buffer.save(output_path)?;
        }

        Ok(())
    }

    /// Save as TIFF 16-bit
    fn save_tiff16(&self, pixels: &ProcessedPixels, output_path: &PathBuf) -> Result<()> {
        use image::{ImageBuffer, Rgb, Rgba};

        if pixels.channels == 4 {
            let img_buffer = ImageBuffer::<Rgba<u16>, _>::from_raw(
                pixels.width,
                pixels.height,
                pixels.data.clone(),
            ).ok_or_else(|| anyhow::anyhow!("Failed to create RGBA image buffer"))?;

            img_buffer.save(output_path)?;
        } else {
            let img_buffer = ImageBuffer::<Rgb<u16>, _>::from_raw(
                pixels.width,
                pixels.height,
                pixels.data.clone(),
            ).ok_or_else(|| anyhow::anyhow!("Failed to create RGB image buffer"))?;

            img_buffer.save(output_path)?;
        }

        Ok(())
    }

    /// Save as TIFF 32-bit float
    fn save_tiff32_float(&self, pixels: &ProcessedPixels, output_path: &PathBuf) -> Result<()> {
        use std::fs::File;
        use tiff::encoder::{TiffEncoder, colortype};
        
        // Convert u16 data back to f32 for 32-bit float export
        let float_data: Vec<f32> = pixels.data
            .par_iter()
            .map(|&value| (value as f32) / 65535.0)
            .collect();
        
        let file = File::create(output_path)?;
        let mut tiff = TiffEncoder::new(file)?;
        
        match pixels.channels {
            4 => {
                tiff.write_image::<colortype::RGBA32Float>(
                    pixels.width,
                    pixels.height,
                    &float_data,
                ).map_err(|e| anyhow::anyhow!("Failed to write RGBA TIFF: {}", e))?;
            }
            3 => {
                tiff.write_image::<colortype::RGB32Float>(
                    pixels.width,
                    pixels.height,
                    &float_data,
                ).map_err(|e| anyhow::anyhow!("Failed to write RGB TIFF: {}", e))?;
            }
            1 => {
                tiff.write_image::<colortype::Gray32Float>(
                    pixels.width,
                    pixels.height,
                    &float_data,
                ).map_err(|e| anyhow::anyhow!("Failed to write grayscale TIFF: {}", e))?;
            }
            _ => anyhow::bail!("Unsupported channel count for TIFF: {}", pixels.channels)
        }

        Ok(())
    }
}

/// Processed pixel data container
struct ProcessedPixels {
    data: Vec<u16>,
    width: u32,
    height: u32,
    channels: u8,
}