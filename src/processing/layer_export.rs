use crate::io::image_cache::{ExrDataSource, LayerChannels, LayerInfo};
use crate::processing::channel_classification::determine_channel_group_with_config;
use crate::utils::channel_config::load_channel_config;
use crate::processing::tone_mapping::{tone_map_and_gamma, ToneMapMode};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Export format configuration
#[derive(Clone, Debug)]
pub enum ExportFormat {
    /// PNG 16-bit per channel
    Png16,
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
            tonemap_mode: ToneMapMode::Linear,
        }
    }
}

/// High-performance layer export processor
pub struct LayerExporter {
    source: ExrDataSource,
    layers_info: Vec<LayerInfo>,
    export_params: ExportParams,
}

impl LayerExporter {
    /// Create a new layer exporter over any EXR data source (full cache or lazy).
    pub fn new(source: ExrDataSource, layers_info: Vec<LayerInfo>) -> Self {
        Self {
            source,
            layers_info,
            export_params: ExportParams::default(),
        }
    }

    /// Set export processing parameters
    pub fn with_params(mut self, params: ExportParams) -> Self {
        self.export_params = params;
        self
    }

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

    /// Export all layers in a specific group
    pub fn export_layer_group(
        &self,
        group_name: &str,
        format: ExportFormat,
        output_dir: &PathBuf,
        base_filename: &str,
    ) -> Result<Vec<PathBuf>> {
        let group_layers = self.find_layers_in_group(group_name)?;
        let mut exported_paths = Vec::new();

        for layer in group_layers {
            let path = self.export_single_layer(&layer, &format, output_dir, base_filename)?;
            exported_paths.push(path);
        }

        Ok(exported_paths)
    }

    /// Export all layers from all groups
    pub fn export_all_layers(
        &self,
        format: ExportFormat,
        output_dir: &PathBuf,
        base_filename: &str,
    ) -> Result<Vec<PathBuf>> {
        let mut exported_paths = Vec::new();

        for layer in &self.layers_info {
            let path = self.export_single_layer(layer, &format, output_dir, base_filename)?;
            exported_paths.push(path);
        }

        Ok(exported_paths)
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

    /// Find all layers belonging to a specific group
    fn find_layers_in_group(&self, group_name: &str) -> Result<Vec<LayerInfo>> {
        // Convert group name to internal format
        let group_key = match group_name {
            "beauty" => "base",
            "scene" => "scene",
            "objects" => "scene_objects",
            "cryptomatte" => "cryptomatte",
            "lights" => "light",
            _ => return Err(anyhow::anyhow!("Unknown group: {}", group_name)),
        };

        let config = load_channel_config().unwrap_or_else(|_| {
            crate::utils::channel_config::get_fallback_config()
        });
        
        let mut group_layers = Vec::new();

        for layer in &self.layers_info {
            let layer_group = determine_channel_group_with_config(&layer.name, &config);
            let group_key_owned = group_key.to_string();
            if layer_group == group_key_owned || layer_group == config.groups.get(group_key).map(|g| g.name.clone()).unwrap_or_default() {
                group_layers.push(layer.clone());
            }
        }

        if group_layers.is_empty() {
            return Err(anyhow::anyhow!("No layers found in group: {}", group_name));
        }

        Ok(group_layers)
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
        let processed_pixels = self.process_layer_to_pixels(&layer_channels, format)?;

        // Generate output filename
        let output_path =
            self.generate_output_path(output_dir, base_filename, &layer_info.name, format)?;

        // Save to file based on format
        self.save_processed_pixels(&processed_pixels, &layer_channels, format, &output_path)?;

        Ok(output_path)
    }

    /// Load layer channels from whichever data source backs this export.
    fn load_layer_channels(&self, layer_name: &str) -> Result<LayerChannels> {
        match &self.source {
            ExrDataSource::Full(full) => {
                crate::io::image_cache::load_all_channels_for_layer_from_full(full, layer_name, None)
            }
            ExrDataSource::Lazy(lazy) => {
                Ok(lazy.get_layer_data(layer_name, None)?.to_layer_channels())
            }
            ExrDataSource::Hdr => anyhow::bail!("Export is not supported for HDR files"),
        }
    }

    /// Process layer to final pixels.
    ///
    /// PNG16 gets the display pipeline (exposure → tone map → gamma → 16-bit).
    /// Tiff32Float deliberately gets none of it: a float export exists to carry
    /// the linear scene values, and tone mapping would destroy exactly the
    /// high-range data the format is chosen for.
    fn process_layer_to_pixels(
        &self,
        layer_channels: &LayerChannels,
        format: &ExportFormat,
    ) -> Result<ProcessedPixels> {
        let pixel_count = (layer_channels.width * layer_channels.height) as usize;

        // Compose RGB from channels (shared helper — identical semantics to the
        // cache-side compositor, including RGB fallback for cryptomatte/etc layers).
        let rgb_pixels = crate::io::image_cache::compose_composite_from_channels(layer_channels);

        let has_alpha = layer_channels.channel_names.iter().any(|name| {
            let n = name.to_ascii_uppercase();
            n == "A" || n.starts_with("ALPHA")
        });
        let channels: u8 = if has_alpha { 4 } else { 3 };

        let data = match format {
            ExportFormat::Tiff32Float => {
                // Raw linear values, unclamped — HDR above 1.0 survives.
                let mut out = Vec::with_capacity(pixel_count * channels as usize);
                for chunk in rgb_pixels.chunks_exact(4) {
                    out.extend_from_slice(&chunk[..channels as usize]);
                }
                PixelData::F32(out)
            }
            ExportFormat::Png16 => PixelData::U16(match layer_channels.channel_names.len() {
                // Grayscale channel - expand to RGB
                1 => self.process_grayscale_pixels(&rgb_pixels, pixel_count)?,
                // RGB/RGBA channels - full color processing with tone mapping
                _ => self.process_color_pixels(&rgb_pixels, pixel_count, has_alpha)?,
            }),
        };

        Ok(ProcessedPixels {
            data,
            width: layer_channels.width,
            height: layer_channels.height,
            channels,
        })
    }

    /// Process grayscale pixels with tone mapping
    fn process_grayscale_pixels(&self, pixels: &[f32], pixel_count: usize) -> Result<Vec<u16>> {
        let mut processed = Vec::with_capacity(pixel_count * 3);

        let exposure_multiplier = 2.0_f32.powf(self.export_params.exposure);
        let gamma_inv = 1.0 / self.export_params.gamma.max(1e-4);
        let use_srgb = (self.export_params.gamma - 2.2).abs() < 0.2
            || (self.export_params.gamma - 2.4).abs() < 0.2;
        let mode = self.export_params.tonemap_mode;

        for chunk in pixels.chunks_exact(4) {
            let gray_value = chunk[0];
            let (processed_gray, _, _) = tone_map_and_gamma(
                gray_value,
                gray_value,
                gray_value,
                exposure_multiplier,
                gamma_inv,
                use_srgb,
                mode,
            );

            let u16_value = (processed_gray.clamp(0.0, 1.0) * 65535.0).round() as u16;
            processed.extend_from_slice(&[u16_value, u16_value, u16_value]);
        }

        Ok(processed)
    }

    /// Process color pixels with tone mapping and color correction (SIMD optimized)
    fn process_color_pixels(
        &self,
        pixels: &[f32],
        pixel_count: usize,
        has_alpha: bool,
    ) -> Result<Vec<u16>> {
        let output_channels = if has_alpha { 4 } else { 3 };
        let mut processed = Vec::with_capacity(pixel_count * output_channels);

        // Use SIMD-optimized processing when feature is enabled
        #[cfg(feature = "unified_simd")]
        {
            processed.extend(self.process_color_pixels_simd_optimized(pixels, has_alpha));
        }

        #[cfg(not(feature = "unified_simd"))]
        {
            let exposure_multiplier = 2.0_f32.powf(self.export_params.exposure);
            let gamma_inv = 1.0 / self.export_params.gamma.max(1e-4);
            let use_srgb = (self.export_params.gamma - 2.2).abs() < 0.2
                || (self.export_params.gamma - 2.4).abs() < 0.2;
            let mode = self.export_params.tonemap_mode;

            for chunk in pixels.chunks_exact(4) {
                let (r, g, b) = tone_map_and_gamma(
                    chunk[0],
                    chunk[1],
                    chunk[2],
                    exposure_multiplier,
                    gamma_inv,
                    use_srgb,
                    mode,
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
        use std::simd::{f32x4, u16x4, Simd, SimdFloat, SimdUint};

        let exposure_multiplier = 2.0f32.powf(self.export_params.exposure);
        let gamma_inv = 1.0 / self.export_params.gamma.max(1e-4);
        let use_srgb = (self.export_params.gamma - 2.2).abs() < 0.2
            || (self.export_params.gamma - 2.4).abs() < 0.2;
        let mode = self.export_params.tonemap_mode;

        // Process pixels in SIMD chunks of 4
        pixels
            .par_chunks_exact(16) // 4 pixels * 4 channels = 16 floats
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

                    // Apply tone mapping (exposure already applied)
                    let (tone_r, tone_g, tone_b) = tone_map_and_gamma(
                        r,
                        g,
                        b,
                        1.0,
                        gamma_inv,
                        use_srgb,
                        mode,
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
        output_dir: &Path,
        base_filename: &str,
        layer_name: &str,
        format: &ExportFormat,
    ) -> Result<PathBuf> {
        let extension = match format {
            ExportFormat::Png16 => "png",
            ExportFormat::Tiff32Float => "tiff",
        };

        let sanitized_layer_name =
            layer_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
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
            ExportFormat::Tiff32Float => {
                self.save_tiff32_float(pixels, output_path)?;
            }
        }

        Ok(())
    }

    /// Save as PNG 16-bit
    fn save_png16(&self, pixels: &ProcessedPixels, output_path: &PathBuf) -> Result<()> {
        use image::{ImageBuffer, Rgba};

        let PixelData::U16(data) = &pixels.data else {
            anyhow::bail!("PNG16 export requires 16-bit pixel data");
        };

        if pixels.channels == 4 {
            let img_buffer =
                ImageBuffer::<Rgba<u16>, _>::from_raw(pixels.width, pixels.height, data.clone())
                    .ok_or_else(|| anyhow::anyhow!("Failed to create RGBA image buffer"))?;

            img_buffer.save(output_path)?;
        } else {
            let img_buffer = ImageBuffer::<image::Rgb<u16>, _>::from_raw(
                pixels.width,
                pixels.height,
                data.clone(),
            )
            .ok_or_else(|| anyhow::anyhow!("Failed to create RGB image buffer"))?;

            img_buffer.save(output_path)?;
        }

        Ok(())
    }

    /// Save as TIFF 32-bit float — writes the linear values through unchanged.
    fn save_tiff32_float(&self, pixels: &ProcessedPixels, output_path: &PathBuf) -> Result<()> {
        use std::fs::File;
        use tiff::encoder::{colortype, TiffEncoder};

        let PixelData::F32(float_data) = &pixels.data else {
            anyhow::bail!("TIFF float export requires 32-bit float pixel data");
        };

        let file = File::create(output_path)?;
        let mut tiff = TiffEncoder::new(file)?;

        match pixels.channels {
            4 => {
                tiff.write_image::<colortype::RGBA32Float>(pixels.width, pixels.height, float_data)
                    .map_err(|e| anyhow::anyhow!("Failed to write RGBA TIFF: {}", e))?;
            }
            3 => {
                tiff.write_image::<colortype::RGB32Float>(pixels.width, pixels.height, float_data)
                    .map_err(|e| anyhow::anyhow!("Failed to write RGB TIFF: {}", e))?;
            }
            _ => anyhow::bail!("Unsupported channel count for TIFF: {}", pixels.channels),
        }

        Ok(())
    }
}

/// Processed pixel data container.
///
/// PNG16 carries display-referred 16-bit integers; Tiff32Float carries the raw
/// linear scene values, so the two cannot share a representation.
enum PixelData {
    U16(Vec<u16>),
    F32(Vec<f32>),
}

struct ProcessedPixels {
    data: PixelData,
    width: u32,
    height: u32,
    channels: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::full_exr_cache::{FullExrCacheData, FullLayer};
    use std::sync::Arc;

    /// 2x1 layer, planar: R=[5.0, 5.0] G=[0.5, 0.5] B=[0.25, 0.25].
    /// R is deliberately above 1.0 — that is the HDR value a float export must keep.
    fn test_exporter() -> LayerExporter {
        let layer = FullLayer {
            name: "Beauty".into(),
            width: 2,
            height: 1,
            channel_names: vec!["R".into(), "G".into(), "B".into()],
            channel_data: Arc::from(vec![5.0f32, 5.0, 0.5, 0.5, 0.25, 0.25].into_boxed_slice()),
        };
        let cache = Arc::new(FullExrCacheData {
            layers: vec![layer],
        });
        let layers_info = cache.to_layers_info();
        LayerExporter::new(ExrDataSource::Full(cache), layers_info)
    }

    #[test]
    fn tiff32_export_preserves_hdr_values() {
        let exporter = test_exporter();
        let lc = exporter.load_layer_channels("Beauty").unwrap();
        let processed = exporter
            .process_layer_to_pixels(&lc, &ExportFormat::Tiff32Float)
            .unwrap();

        let PixelData::F32(data) = processed.data else {
            panic!("Tiff32Float must produce f32 pixel data");
        };
        assert_eq!(processed.channels, 3);
        assert_eq!(
            data[0], 5.0,
            "a value above 1.0 must survive a 32-bit float export"
        );
        assert_eq!(data[1], 0.5);
        assert_eq!(data[2], 0.25);
    }

    #[test]
    fn png16_export_stays_u16() {
        let exporter = test_exporter();
        let lc = exporter.load_layer_channels("Beauty").unwrap();
        let processed = exporter
            .process_layer_to_pixels(&lc, &ExportFormat::Png16)
            .unwrap();

        let PixelData::U16(data) = processed.data else {
            panic!("Png16 must produce u16 pixel data");
        };
        // Display-referred: the HDR red clamps to full scale.
        assert_eq!(data[0], u16::MAX);
    }
}
