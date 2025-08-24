use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;
use slint::Rgba8Pixel;
use glam::{Mat3, Vec3};
use crate::processing::image_processing::process_pixel;
use crate::processing::histogram::LuminanceWeights;

#[cfg(feature = "unified_simd")]
use crate::processing::tone_mapping::ToneMapMode;

/// Optimized SIMD processing functions for image processing
/// Separates SIMD and scalar code paths for better performance

#[cfg(feature = "unified_simd")]
mod unified_processing {
    use super::*;
    
    /// Parameters for pixel processing operations
    #[derive(Clone, Copy)]
    pub struct ProcessParams {
        pub exposure: f32,
        pub gamma: f32,
        pub tonemap_mode: ToneMapMode,
    }

    impl ProcessParams {
        pub fn new(exposure: f32, gamma: f32, tonemap_mode: i32) -> Self {
            Self {
                exposure,
                gamma,
                tonemap_mode: ToneMapMode::from(tonemap_mode),
            }
        }
    }

    /// Scalar processing implementation
    pub struct ScalarProcessor;

    impl ScalarProcessor {
        pub fn apply_exposure(&self, value: f32, exposure_multiplier: f32) -> f32 {
            value * exposure_multiplier
        }
        
        pub fn apply_tonemap(&self, value: f32, mode: ToneMapMode) -> f32 {
            use ToneMapMode::*;
            match mode {
                ACES => {
                    let a = 2.51;
                    let b = 0.03;
                    let c = 2.43;
                    let d = 0.59;
                    let e = 0.14;
                    ((value * (a * value + b)) / (value * (c * value + d) + e)).clamp(0.0, 1.0)
                },
                Reinhard => value / (1.0 + value),
                Filmic => {
                    let x_max = 0.22 * 11.2;
                    let linear = 0.22 * value;
                    let squared = 0.1 * value * value;
                    ((linear + squared) / (1.0 + linear + squared)).min(x_max)
                },
                Hable => {
                    let a = 0.15;
                    let b = 0.50;
                    let c = 0.10;
                    let d = 0.20;
                    let e = 0.02;
                    let f = 0.30;
                    let w = 11.2;
                    
                    let curr = ((value * (a * value + c * b) + d * e) / (value * (a * value + b) + d * f)) - e / f;
                    let white_scale = ((w * (a * w + c * b) + d * e) / (w * (a * w + b) + d * f)) - e / f;
                    (curr / white_scale).clamp(0.0, 1.0)
                },
                Linear => value.clamp(0.0, 1.0),
                Local => value.clamp(0.0, 1.0), // Fallback for local tone mapping
            }
        }
        
        pub fn apply_gamma(&self, value: f32, gamma_inv: f32, use_srgb: bool) -> f32 {
            if use_srgb {
                if value <= 0.0031308 {
                    12.92 * value
                } else {
                    1.055 * value.powf(1.0 / 2.4) - 0.055
                }
            } else {
                value.powf(gamma_inv).clamp(0.0, 1.0)
            }
        }
        
        pub fn clamp_unit(&self, value: f32) -> f32 {
            value.clamp(0.0, 1.0)
        }
        
        pub fn select_finite(&self, value: f32, fallback: f32) -> f32 {
            if value.is_finite() && value >= 0.0 { value } else { fallback }
        }
    }

    /// Unified processing function that works with scalar types
    pub fn process_pixel_unified(
        processor: &ScalarProcessor,
        r: f32, g: f32, b: f32, a: f32,
        params: &ProcessParams,
    ) -> (f32, f32, f32, f32) {
        let exposure_multiplier = 2.0_f32.powf(params.exposure);
        
        // Clean up inputs (handle NaN/Inf)
        let clean_r = processor.select_finite(r, 0.0);
        let clean_g = processor.select_finite(g, 0.0);
        let clean_b = processor.select_finite(b, 0.0);
        let clean_a = processor.select_finite(a, 1.0);
        
        // Apply exposure
        let exposed_r = processor.apply_exposure(clean_r, exposure_multiplier);
        let exposed_g = processor.apply_exposure(clean_g, exposure_multiplier);
        let exposed_b = processor.apply_exposure(clean_b, exposure_multiplier);
        
        // Apply tone mapping
        let tone_mapped_r = processor.apply_tonemap(exposed_r, params.tonemap_mode);
        let tone_mapped_g = processor.apply_tonemap(exposed_g, params.tonemap_mode);
        let tone_mapped_b = processor.apply_tonemap(exposed_b, params.tonemap_mode);
        
        // Apply gamma correction
        let use_srgb = (params.gamma - 2.2).abs() < 0.2 || (params.gamma - 2.4).abs() < 0.2;
        let gamma_inv = 1.0 / params.gamma.max(1e-4);
        
        let final_r = processor.apply_gamma(tone_mapped_r, gamma_inv, use_srgb);
        let final_g = processor.apply_gamma(tone_mapped_g, gamma_inv, use_srgb);
        let final_b = processor.apply_gamma(tone_mapped_b, gamma_inv, use_srgb);
        let final_a = processor.clamp_unit(clean_a);
        
        (final_r, final_g, final_b, final_a)
    }
}

/// SIMD processing configuration
pub const SIMD_CHUNK_SIZE: usize = 16; // 4 pixels * 4 channels RGBA
pub const SIMD_PIXEL_COUNT: usize = 4; // Process 4 pixels per SIMD operation

/// Process a chunk of 16 f32 values (4 RGBA pixels) using SIMD - optimized input
/// Input: [R0,G0,B0,A0, R1,G1,B1,A1, R2,G2,B2,A2, R3,G3,B3,A3]
/// Output: 4 Rgba8Pixel values
#[inline(always)]
pub fn process_simd_chunk_rgba(
    input: &[f32],
    output: &mut [Rgba8Pixel],
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
) {
    // Load RGBA channels into SIMD registers
    let (mut r, mut g, mut b, a) = load_rgba_simd(input);

    // Apply color matrix transformation if provided
    if let Some(mat) = color_matrix {
        (r, g, b) = apply_color_matrix_simd(r, g, b, mat);
    }

    // Apply tone mapping and gamma correction
    let (r8, g8, b8) = crate::processing::tone_mapping::tone_map_and_gamma_simd(r, g, b, exposure, gamma, tonemap_mode);
    let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

    // Convert to u8 and store
    store_rgba_simd(r8, g8, b8, a8, output);
}

/// Process a chunk with grayscale output - optimized input
#[inline(always)]
pub fn process_simd_chunk_grayscale(
    input: &[f32],
    output: &mut [Rgba8Pixel],
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
) {
    let (mut r, mut g, mut b, a) = load_rgba_simd(input);

    if let Some(mat) = color_matrix {
        (r, g, b) = apply_color_matrix_simd(r, g, b, mat);
    }

    let (r_tm, g_tm, b_tm) = crate::processing::tone_mapping::tone_map_and_gamma_simd(r, g, b, exposure, gamma, tonemap_mode);
    
    // Convert to grayscale using Rec.709 luminance weights
    let (wr, wg, wb) = LuminanceWeights::default().coefficients();
    let gray = (Simd::splat(wr) * r_tm + Simd::splat(wg) * g_tm + Simd::splat(wb) * b_tm)
        .simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
    let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

    store_grayscale_simd(gray, a8, output);
}

/// Load RGBA data from interleaved f32 slice into SIMD registers - optimized
#[inline(always)]
fn load_rgba_simd(input: &[f32]) -> (f32x4, f32x4, f32x4, f32x4) {
    // Use direct SIMD operations - more efficient than array conversions
    let r = f32x4::from_array([input[0], input[4], input[8], input[12]]);
    let g = f32x4::from_array([input[1], input[5], input[9], input[13]]);
    let b = f32x4::from_array([input[2], input[6], input[10], input[14]]);
    let a = f32x4::from_array([input[3], input[7], input[11], input[15]]);
    (r, g, b, a)
}

/// Apply color matrix transformation using SIMD
#[inline(always)]
fn apply_color_matrix_simd(r: f32x4, g: f32x4, b: f32x4, mat: Mat3) -> (f32x4, f32x4, f32x4) {
    // Pre-splat matrix elements for SIMD
    let m00 = Simd::splat(mat.x_axis.x);
    let m01 = Simd::splat(mat.y_axis.x);  
    let m02 = Simd::splat(mat.z_axis.x);
    let m10 = Simd::splat(mat.x_axis.y);
    let m11 = Simd::splat(mat.y_axis.y);
    let m12 = Simd::splat(mat.z_axis.y);
    let m20 = Simd::splat(mat.x_axis.z);
    let m21 = Simd::splat(mat.y_axis.z);
    let m22 = Simd::splat(mat.z_axis.z);

    // Matrix multiplication
    let rr = m00 * r + m01 * g + m02 * b;
    let gg = m10 * r + m11 * g + m12 * b;
    let bb = m20 * r + m21 * g + m22 * b;
    
    (rr, gg, bb)
}

/// Store RGBA SIMD results to Rgba8Pixel slice - optimized
#[inline(always)]
fn store_rgba_simd(r: f32x4, g: f32x4, b: f32x4, a: f32x4, output: &mut [Rgba8Pixel]) {
    let ra: [f32; 4] = r.into();
    let ga: [f32; 4] = g.into();
    let ba: [f32; 4] = b.into();
    let aa: [f32; 4] = a.into();

    for i in 0..4 {
        output[i] = Rgba8Pixel {
            r: (ra[i] * 255.0).round().clamp(0.0, 255.0) as u8,
            g: (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8,
            b: (ba[i] * 255.0).round().clamp(0.0, 255.0) as u8,
            a: (aa[i] * 255.0).round().clamp(0.0, 255.0) as u8,
        };
    }
}

/// Store grayscale SIMD results to Rgba8Pixel slice - optimized
#[inline(always)]
fn store_grayscale_simd(gray: f32x4, a: f32x4, output: &mut [Rgba8Pixel]) {
    let ga: [f32; 4] = gray.into();
    let aa: [f32; 4] = a.into();

    for i in 0..4 {
        let g8 = (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8;
        output[i] = Rgba8Pixel { r: g8, g: g8, b: g8, a: aa[i] as u8 };
    }
}

/// Scalar processing for remainder pixels that don't fit in SIMD chunks
pub fn process_scalar_pixels(
    input: &[f32],
    output: &mut [Rgba8Pixel], 
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
    grayscale: bool,
) {
    // Option to use unified processing approach
    #[cfg(feature = "unified_simd")]
    {
        use unified_processing::{ScalarProcessor, ProcessParams, process_pixel_unified};
        let processor = ScalarProcessor;
        let params = ProcessParams::new(exposure, gamma, tonemap_mode);
        
        let pixel_count = input.len() / 4;
        for i in 0..pixel_count {
            let pixel_start = i * 4;
            let mut r = input[pixel_start];
            let mut g = input[pixel_start + 1];
            let mut b = input[pixel_start + 2];
            let a = input[pixel_start + 3];
            
            // Apply color matrix if provided
            if let Some(mat) = color_matrix {
                let v = mat * Vec3::new(r, g, b);
                r = v.x; g = v.y; b = v.z;
            }
            
            let (final_r, final_g, final_b, final_a) = process_pixel_unified(&processor, r, g, b, a, &params);
            
            if grayscale {
                let gray = LuminanceWeights::default().luminance(final_r, final_g, final_b);
                output[i] = Rgba8Pixel {
                    r: (gray * 255.0) as u8,
                    g: (gray * 255.0) as u8,
                    b: (gray * 255.0) as u8,
                    a: (final_a * 255.0) as u8,
                };
            } else {
                output[i] = Rgba8Pixel {
                    r: (final_r * 255.0) as u8,
                    g: (final_g * 255.0) as u8,
                    b: (final_b * 255.0) as u8,
                    a: (final_a * 255.0) as u8,
                };
            }
        }
        return;
    }
    
    // Original implementation (fallback)
    let pixel_count = input.len() / 4;
    
    for i in 0..pixel_count {
        let pixel_start = i * 4;
        let r0 = input[pixel_start];
        let g0 = input[pixel_start + 1];
        let b0 = input[pixel_start + 2];
        let a0 = input[pixel_start + 3];
        
        // Apply color matrix if provided
        let (mut r, mut g, mut b) = (r0, g0, b0);
        if let Some(mat) = color_matrix {
            let v = mat * Vec3::new(r, g, b);
            r = v.x; g = v.y; b = v.z;
        }

        if grayscale {
            let px = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
            let rr = (px.r as f32) / 255.0;
            let gg = (px.g as f32) / 255.0;
            let bb = (px.b as f32) / 255.0;
            let gray_val = rr.max(gg).max(bb).clamp(0.0, 1.0);
            let g8 = (gray_val * 255.0).round().clamp(0.0, 255.0) as u8;
            output[i] = Rgba8Pixel { r: g8, g: g8, b: g8, a: px.a };
        } else {
            output[i] = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
        }
    }
}


/// Unified SIMD processing function - consolidates patterns from image_cache.rs
/// Handles both parallel and sequential processing with consistent SIMD optimization
pub fn process_rgba_chunk_optimized(
    input: &[f32],
    output: &mut [Rgba8Pixel],
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
    grayscale: bool,
    parallel: bool,
) {
    let total_pixels = input.len() / 4;
    let simd_pixels = (total_pixels / SIMD_PIXEL_COUNT) * SIMD_PIXEL_COUNT;
    let simd_elements = simd_pixels * 4;

    // Process SIMD chunks - parallel or sequential
    if simd_pixels > 0 {
        if parallel {
            // Parallel processing using rayon
            use rayon::prelude::*;
            input[..simd_elements]
                .par_chunks_exact(SIMD_CHUNK_SIZE)
                .zip(output[..simd_pixels].par_chunks_exact_mut(SIMD_PIXEL_COUNT))
                .for_each(|(in_chunk, out_chunk)| {
                    if grayscale {
                        process_simd_chunk_grayscale(in_chunk, out_chunk, exposure, gamma, tonemap_mode, color_matrix);
                    } else {
                        process_simd_chunk_rgba(in_chunk, out_chunk, exposure, gamma, tonemap_mode, color_matrix);
                    }
                });
        } else {
            // Sequential processing
            input[..simd_elements]
                .chunks_exact(SIMD_CHUNK_SIZE)
                .zip(output[..simd_pixels].chunks_exact_mut(SIMD_PIXEL_COUNT))
                .for_each(|(in_chunk, out_chunk)| {
                    if grayscale {
                        process_simd_chunk_grayscale(in_chunk, out_chunk, exposure, gamma, tonemap_mode, color_matrix);
                    } else {
                        process_simd_chunk_rgba(in_chunk, out_chunk, exposure, gamma, tonemap_mode, color_matrix);
                    }
                });
        }
    }

    // Process remainder pixels with scalar code - consistent for both modes
    if simd_elements < input.len() {
        process_scalar_pixels(
            &input[simd_elements..],
            &mut output[simd_pixels..],
            exposure, gamma, tonemap_mode,
            color_matrix, grayscale
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat3;

    #[test]
    fn test_simd_chunk_processing() {
        // Test data: 4 pixels RGBA
        let input: [f32; 16] = [
            1.0, 0.5, 0.2, 1.0,  // Pixel 1
            0.8, 0.6, 0.3, 1.0,  // Pixel 2
            0.4, 0.9, 0.1, 1.0,  // Pixel 3
            0.2, 0.3, 0.8, 1.0,  // Pixel 4
        ];
        let mut output: [Rgba8Pixel; 4] = [Rgba8Pixel { r: 0, g: 0, b: 0, a: 0 }; 4];

        process_simd_chunk_rgba(&input, &mut output, 0.0, 2.2, 0, None);

        // Verify all pixels were processed (non-zero values)
        for pixel in &output {
            assert!(pixel.r > 0 || pixel.g > 0 || pixel.b > 0);
            assert_eq!(pixel.a, 255); // Alpha should be 255
        }
    }

    #[test]
    fn test_color_matrix_simd() {
        let r = f32x4::from_array([1.0, 0.5, 0.2, 0.8]);
        let g = f32x4::from_array([0.3, 0.7, 0.9, 0.1]);
        let b = f32x4::from_array([0.4, 0.2, 0.6, 0.5]);
        
        let identity = Mat3::IDENTITY;
        let (rr, gg, bb) = apply_color_matrix_simd(r, g, b, identity);
        
        // With identity matrix, values should remain the same
        let r_out: [f32; 4] = rr.into();
        let g_out: [f32; 4] = gg.into();
        let b_out: [f32; 4] = bb.into();
        
        assert!((r_out[0] - 1.0).abs() < 0.001);
        assert!((g_out[1] - 0.7).abs() < 0.001);
        assert!((b_out[2] - 0.6).abs() < 0.001);
    }
}