#![allow(dead_code)]

use slint::{Image, SharedPixelBuffer, Rgba8Pixel};
use crate::ui::progress::ProgressSink;
use glam::{Vec3, Mat3};

/// Configuration for image processing operations
#[derive(Debug, Clone)]
pub struct ImageProcessingConfig {
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: i32,
    pub color_matrix: Option<Mat3>,
    pub lighting_rgb: Option<bool>, // For composite mode
    pub invert: Option<bool>,       // For depth processing
    pub max_size: Option<u32>,      // For thumbnails
}

impl ImageProcessingConfig {
    /// Create a new config with basic parameters
    pub fn new(exposure: f32, gamma: f32, tonemap_mode: i32) -> Self {
        Self {
            exposure,
            gamma,
            tonemap_mode,
            color_matrix: None,
            lighting_rgb: None,
            invert: None,
            max_size: None,
        }
    }
    
    /// Set color matrix for color space conversion
    pub fn with_color_matrix(mut self, matrix: Option<Mat3>) -> Self {
        self.color_matrix = matrix;
        self
    }
    
    /// Set lighting RGB mode for composite processing
    pub fn with_lighting_rgb(mut self, lighting_rgb: bool) -> Self {
        self.lighting_rgb = Some(lighting_rgb);
        self
    }
    
    /// Set invert flag for depth processing
    pub fn with_invert(mut self, invert: bool) -> Self {
        self.invert = Some(invert);
        self
    }
    
    /// Set maximum size for thumbnail generation
    pub fn with_max_size(mut self, max_size: u32) -> Self {
        self.max_size = Some(max_size);
        self
    }
}

/// Represents the target output type for processing
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessingTarget {
    FullImage,
    Composite,
    Thumbnail,
    DepthImage,
}

/// Data required for processing
pub struct ProcessingData<'a> {
    pub pixels: &'a [f32],
    pub width: u32,
    pub height: u32,
    pub config: &'a ImageProcessingConfig,
    pub mip_levels: Option<&'a [crate::io::image_cache::MipLevel]>, // For thumbnail processing
}

/// Unified image processing pipeline that eliminates code duplication
pub struct ProcessingPipeline;

impl ProcessingPipeline {
    /// Process pixels to image using a generic processing function
    /// 
    /// The processor_fn receives (input_pixels, output_pixels, config, dimensions)
    /// and should fill the output buffer with processed RGBA8 data
    pub fn process_pixels_to_image<F>(
        data: ProcessingData,
        target: ProcessingTarget,
        progress: Option<&dyn ProgressSink>,
        processor_fn: F,
    ) -> Image
    where
        F: FnOnce(&[f32], &mut [Rgba8Pixel], &ImageProcessingConfig, u32, u32),
    {
        match target {
            ProcessingTarget::FullImage => {
                Self::process_full_image(data, processor_fn)
            }
            ProcessingTarget::Composite => {
                Self::process_composite(data, processor_fn)
            }
            ProcessingTarget::Thumbnail => {
                Self::process_thumbnail(data, processor_fn)
            }
            ProcessingTarget::DepthImage => {
                Self::process_depth(data, progress, processor_fn)
            }
        }
    }
    
    fn process_full_image<F>(
        data: ProcessingData,
        processor_fn: F,
    ) -> Image
    where
        F: FnOnce(&[f32], &mut [Rgba8Pixel], &ImageProcessingConfig, u32, u32),
    {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(data.width, data.height);
        let out_slice = buffer.make_mut_slice();
        
        processor_fn(data.pixels, out_slice, data.config, data.width, data.height);
        
        Image::from_rgba8(buffer)
    }
    
    fn process_composite<F>(
        data: ProcessingData,
        processor_fn: F,
    ) -> Image
    where
        F: FnOnce(&[f32], &mut [Rgba8Pixel], &ImageProcessingConfig, u32, u32),
    {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(data.width, data.height);
        let out_slice = buffer.make_mut_slice();
        
        processor_fn(data.pixels, out_slice, data.config, data.width, data.height);
        
        Image::from_rgba8(buffer)
    }
    
    fn process_thumbnail<F>(
        data: ProcessingData,
        _processor_fn: F,
    ) -> Image
    where
        F: FnOnce(&[f32], &mut [Rgba8Pixel], &ImageProcessingConfig, u32, u32),
    {
        let max_size = data.config.max_size.unwrap_or(256);
        let scale = (max_size as f32 / data.width.max(data.height) as f32).min(1.0);
        let thumb_width = (data.width as f32 * scale) as u32;
        let thumb_height = (data.height as f32 * scale) as u32;
        
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(thumb_width, thumb_height);
        let out_slice = buffer.make_mut_slice();
        
        // Choose best source: original or closest MIP level >= target size
        let (src_pixels, src_w, src_h) = Self::select_mip_source(
            data.pixels,
            data.width,
            data.height,
            data.mip_levels,
            thumb_width.max(thumb_height),
        );
        
        // Custom thumbnail processor that handles scaling
        Self::process_thumbnail_with_scaling(
            src_pixels,
            src_w,
            src_h,
            out_slice,
            thumb_width,
            thumb_height,
            data.config,
        );
        
        Image::from_rgba8(buffer)
    }
    
    fn process_depth<F>(
        data: ProcessingData,
        progress: Option<&dyn ProgressSink>,
        processor_fn: F,
    ) -> Image
    where
        F: FnOnce(&[f32], &mut [Rgba8Pixel], &ImageProcessingConfig, u32, u32),
    {
        if let Some(p) = progress {
            p.start_indeterminate(Some("Processing depth data..."));
        }
        
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(data.width, data.height);
        let out_slice = buffer.make_mut_slice();
        
        processor_fn(data.pixels, out_slice, data.config, data.width, data.height);
        
        if let Some(p) = progress {
            p.finish(Some("Depth processed"));
        }
        
        Image::from_rgba8(buffer)
    }
    
    fn select_mip_source<'a>(
        original_pixels: &'a [f32],
        original_width: u32,
        original_height: u32,
        mip_levels: Option<&'a [crate::io::image_cache::MipLevel]>,
        target_size: u32,
    ) -> (&'a [f32], u32, u32) {
        if let Some(mips) = mip_levels {
            if !mips.is_empty() {
                // Find MIP level whose longer side is closest to target but not smaller
                if let Some(level) = mips.iter().find(|lvl| lvl.width.max(lvl.height) >= target_size) {
                    return (&level.pixels, level.width, level.height);
                }
            }
        }
        
        (original_pixels, original_width, original_height)
    }
    
    fn process_thumbnail_with_scaling(
        src_pixels: &[f32],
        src_width: u32,
        src_height: u32,
        output: &mut [Rgba8Pixel],
        thumb_width: u32,
        thumb_height: u32,
        config: &ImageProcessingConfig,
    ) {
        use rayon::prelude::*;
        
        let scale_x = (src_width as f32) / (thumb_width.max(1) as f32);
        let scale_y = (src_height as f32) / (thumb_height.max(1) as f32);
        
        // Process in parallel blocks of 4 pixels for SIMD efficiency
        output
            .par_chunks_mut(4)
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
                    let sx = src_x.min(src_width.saturating_sub(1)) as usize;
                    let sy = src_y.min(src_height.saturating_sub(1)) as usize;
                    let src_idx = sy * (src_width as usize) + sx;
                    let pixel_start = src_idx * 4;
                    
                    let mut r = src_pixels[pixel_start];
                    let mut g = src_pixels[pixel_start + 1];
                    let mut b = src_pixels[pixel_start + 2];
                    let a = src_pixels[pixel_start + 3];
                    
                    // Apply color matrix if provided
                    if let Some(mat) = config.color_matrix {
                        let v = mat * Vec3::new(r, g, b);
                        r = v.x;
                        g = v.y;
                        b = v.z;
                    }
                    
                    rr[lane] = r;
                    gg[lane] = g;
                    bb[lane] = b;
                    aa[lane] = a;
                    valid += 1;
                }
                
                // Apply exposure, gamma, and tone mapping using image processing
                if valid > 0 {
                    for (lane, pixel) in out_block.iter_mut().enumerate().take(valid) {
                        *pixel = crate::processing::image_processing::process_pixel(
                            rr[lane],
                            gg[lane], 
                            bb[lane],
                            aa[lane],
                            config.exposure,
                            config.gamma,
                            config.tonemap_mode,
                        );
                    }
                }
            });
    }
}

/// Standard processor functions for common use cases
pub mod processors {
    use super::*;
    
    /// Standard image processor using optimized SIMD processing
    pub fn standard_image_processor(
        input: &[f32],
        output: &mut [Rgba8Pixel],
        config: &ImageProcessingConfig,
        _width: u32,
        _height: u32,
    ) {
        crate::processing::simd_processing::process_rgba_chunk_optimized(
            input,
            output,
            config.exposure,
            config.gamma,
            config.tonemap_mode,
            config.color_matrix,
            false, // Not grayscale
            false, // Sequential processing for pipeline
        );
    }
    
    /// Composite processor with lighting RGB mode
    pub fn composite_processor(
        input: &[f32],
        output: &mut [Rgba8Pixel],
        config: &ImageProcessingConfig,
        _width: u32,
        _height: u32,
    ) {
        let lighting_rgb = config.lighting_rgb.unwrap_or(false);
        crate::processing::simd_processing::process_rgba_chunk_optimized(
            input,
            output,
            config.exposure,
            config.gamma,
            config.tonemap_mode,
            config.color_matrix,
            !lighting_rgb, // Inverted logic as per original code
            false, // Sequential processing for pipeline
        );
    }
    
    /// Depth image processor
    pub fn depth_processor(
        input: &[f32],
        output: &mut [Rgba8Pixel],
        config: &ImageProcessingConfig,
        _width: u32,
        _height: u32,
    ) {
        use rayon::prelude::*;
        
        // Extract depth values from first channel
        let values: Vec<f32> = input.par_chunks_exact(4).map(|chunk| chunk[0]).collect();
        if values.is_empty() {
            return;
        }
        
        // Find min/max for normalization
        let min_val = values.par_iter().cloned().fold(|| f32::INFINITY, f32::min).reduce(|| f32::INFINITY, f32::min);
        let max_val = values.par_iter().cloned().fold(|| f32::NEG_INFINITY, f32::max).reduce(|| f32::NEG_INFINITY, f32::max);
        
        if min_val >= max_val {
            return;
        }
        
        let range = max_val - min_val;
        let invert = config.invert.unwrap_or(false);
        
        // Process depth values to grayscale
        output.par_iter_mut().enumerate().for_each(|(i, pixel)| {
            if i < values.len() {
                let mut normalized = (values[i] - min_val) / range;
                if invert {
                    normalized = 1.0 - normalized;
                }
                normalized = normalized.clamp(0.0, 1.0);
                let gray = (normalized * 255.0) as u8;
                *pixel = Rgba8Pixel { r: gray, g: gray, b: gray, a: 255 };
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_config_builder() {
        let config = ImageProcessingConfig::new(1.0, 2.2, 0)
            .with_color_matrix(None)
            .with_lighting_rgb(true)
            .with_max_size(512);
        
        assert_eq!(config.exposure, 1.0);
        assert_eq!(config.gamma, 2.2);
        assert_eq!(config.tonemap_mode, 0);
        assert_eq!(config.lighting_rgb, Some(true));
        assert_eq!(config.max_size, Some(512));
    }
    
    #[test]
    fn test_processing_target_enum() {
        assert_eq!(ProcessingTarget::FullImage, ProcessingTarget::FullImage);
        assert_ne!(ProcessingTarget::FullImage, ProcessingTarget::Thumbnail);
    }
}