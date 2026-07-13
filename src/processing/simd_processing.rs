use crate::processing::image_processing::process_pixel;
use core::simd::{f32x4, Simd};
use glam::{Mat3, Vec3};
use slint::Rgba8Pixel;
use std::simd::prelude::SimdFloat;

/// Optimized SIMD processing functions for image processing
/// Separates SIMD and scalar code paths for better performance

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
    exposure_mult_splat: f32x4,
    gamma_inv: f32,
    use_srgb: bool,
    mode: crate::processing::tone_mapping::ToneMapMode,
    color_matrix: Option<Mat3>,
) {
    // Load RGBA channels into SIMD registers
    let (mut r, mut g, mut b, a) = load_rgba_simd(input);

    // Apply color matrix transformation if provided
    if let Some(mat) = color_matrix {
        (r, g, b) = apply_color_matrix_simd(r, g, b, mat);
    }

    // Apply tone mapping and gamma correction
    let (r8, g8, b8) = crate::processing::tone_mapping::tone_map_and_gamma_simd(
        r,
        g,
        b,
        exposure_mult_splat,
        gamma_inv,
        use_srgb,
        mode,
    );
    let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

    // Convert to u8 and store
    store_rgba_simd(r8, g8, b8, a8, output);
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

/// Scalar processing for remainder pixels that don't fit in SIMD chunks
#[allow(clippy::too_many_arguments)]
pub fn process_scalar_pixels(
    input: &[f32],
    output: &mut [Rgba8Pixel],
    exposure_multiplier: f32,
    gamma_inv: f32,
    use_srgb: bool,
    mode: crate::processing::tone_mapping::ToneMapMode,
    color_matrix: Option<Mat3>,
) {
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
            r = v.x;
            g = v.y;
            b = v.z;
        }

        output[i] = process_pixel(r, g, b, a0, exposure_multiplier, gamma_inv, use_srgb, mode);
    }
}

/// Tone-map + gamma an interleaved RGBA f32 buffer into 8-bit pixels.
/// SIMD for whole chunks of 4 pixels, scalar for the remainder.
#[allow(clippy::too_many_arguments)]
pub fn process_rgba_chunk_optimized(
    input: &[f32],
    output: &mut [Rgba8Pixel],
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
    parallel: bool,
) {
    let total_pixels = input.len() / 4;
    let simd_pixels = (total_pixels / SIMD_PIXEL_COUNT) * SIMD_PIXEL_COUNT;
    let simd_elements = simd_pixels * 4;

    // Frame-constant precomputes hoisted out of the chunk loop
    let params = crate::processing::tone_mapping::ToneMapParams::new(exposure, gamma, tonemap_mode);
    let exposure_mult_splat = Simd::splat(params.exposure_multiplier);
    let gamma_inv = params.gamma_inv;
    let use_srgb = params.use_srgb;
    let mode = params.mode;

    let render_chunk = |(in_chunk, out_chunk): (&[f32], &mut [Rgba8Pixel])| {
        process_simd_chunk_rgba(
            in_chunk,
            out_chunk,
            exposure_mult_splat,
            gamma_inv,
            use_srgb,
            mode,
            color_matrix,
        );
    };

    if simd_pixels > 0 {
        if parallel {
            use rayon::prelude::*;
            input[..simd_elements]
                .par_chunks_exact(SIMD_CHUNK_SIZE)
                .zip(output[..simd_pixels].par_chunks_exact_mut(SIMD_PIXEL_COUNT))
                .for_each(render_chunk);
        } else {
            input[..simd_elements]
                .chunks_exact(SIMD_CHUNK_SIZE)
                .zip(output[..simd_pixels].chunks_exact_mut(SIMD_PIXEL_COUNT))
                .for_each(render_chunk);
        }
    }

    // Remainder pixels (fewer than 4) go through the scalar path.
    if simd_elements < input.len() {
        process_scalar_pixels(
            &input[simd_elements..],
            &mut output[simd_pixels..],
            params.exposure_multiplier,
            gamma_inv,
            use_srgb,
            mode,
            color_matrix,
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
            1.0, 0.5, 0.2, 1.0, // Pixel 1
            0.8, 0.6, 0.3, 1.0, // Pixel 2
            0.4, 0.9, 0.1, 1.0, // Pixel 3
            0.2, 0.3, 0.8, 1.0, // Pixel 4
        ];
        let mut output: [Rgba8Pixel; 4] = [Rgba8Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }; 4];

        let params = crate::processing::tone_mapping::ToneMapParams::new(0.0, 2.2, 0);
        process_simd_chunk_rgba(
            &input,
            &mut output,
            Simd::splat(params.exposure_multiplier),
            params.gamma_inv,
            params.use_srgb,
            params.mode,
            None,
        );

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
