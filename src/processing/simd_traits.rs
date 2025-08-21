use std::simd::{f32x4, Simd, num::SimdFloat, cmp::SimdPartialOrd};
use crate::processing::tone_mapping::ToneMapMode;

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

/// Generic trait for SIMD-capable processing operations
/// T can be f32 (scalar) or f32x4 (SIMD vector)
pub trait SimdProcessable<T> {
    /// Apply exposure correction
    fn apply_exposure(&self, value: T, exposure_multiplier: T) -> T;
    
    /// Apply tone mapping operator
    fn apply_tonemap(&self, value: T, mode: ToneMapMode) -> T;
    
    /// Apply gamma correction or sRGB OETF
    fn apply_gamma(&self, value: T, gamma_inv: f32, use_srgb: bool) -> T;
    
    /// Clamp values to valid range [0, 1]
    fn clamp_unit(&self, value: T) -> T;
    
    /// Check if values are finite (no NaN/Inf)
    fn is_finite(&self, value: T) -> T;
    
    /// Select values based on condition (finite check)
    fn select_finite(&self, value: T, fallback: T) -> T;
    
    /// Create a splat (broadcast) value
    fn splat(&self, value: f32) -> T;
    
    /// Get zero value
    fn zero(&self) -> T;
    
    /// Get one value
    fn one(&self) -> T;
    
    /// Convert to u8 range [0, 255]
    fn to_u8_clamped(&self, value: T) -> T;
}

/// Scalar implementation for f32
pub struct ScalarProcessor;

impl SimdProcessable<f32> for ScalarProcessor {
    fn apply_exposure(&self, value: f32, exposure_multiplier: f32) -> f32 {
        value * exposure_multiplier
    }
    
    fn apply_tonemap(&self, value: f32, mode: ToneMapMode) -> f32 {
        use ToneMapMode::*;
        match mode {
            ACES => aces_tonemap_scalar(value),
            Reinhard => reinhard_tonemap_scalar(value),
            Filmic => filmic_tonemap_scalar(value),
            Hable => hable_tonemap_scalar(value),
            Linear => value.clamp(0.0, 1.0),
            Local => value.clamp(0.0, 1.0), // Fallback for local tone mapping
        }
    }
    
    fn apply_gamma(&self, value: f32, gamma_inv: f32, use_srgb: bool) -> f32 {
        if use_srgb {
            srgb_oetf_scalar(value)
        } else {
            apply_gamma_scalar(value, gamma_inv)
        }
    }
    
    fn clamp_unit(&self, value: f32) -> f32 {
        value.clamp(0.0, 1.0)
    }
    
    fn is_finite(&self, value: f32) -> f32 {
        if value.is_finite() { 1.0 } else { 0.0 }
    }
    
    fn select_finite(&self, value: f32, fallback: f32) -> f32 {
        if value.is_finite() && value >= 0.0 { value } else { fallback }
    }
    
    fn splat(&self, value: f32) -> f32 {
        value
    }
    
    fn zero(&self) -> f32 {
        0.0
    }
    
    fn one(&self) -> f32 {
        1.0
    }
    
    fn to_u8_clamped(&self, value: f32) -> f32 {
        (value * 255.0).round().clamp(0.0, 255.0)
    }
}

/// SIMD implementation for f32x4
pub struct SimdProcessor;

impl SimdProcessable<f32x4> for SimdProcessor {
    fn apply_exposure(&self, value: f32x4, exposure_multiplier: f32x4) -> f32x4 {
        value * exposure_multiplier
    }
    
    fn apply_tonemap(&self, value: f32x4, mode: ToneMapMode) -> f32x4 {
        use ToneMapMode::*;
        match mode {
            ACES => aces_tonemap_simd(value),
            Reinhard => reinhard_tonemap_simd(value),
            Filmic => filmic_tonemap_simd(value), 
            Hable => hable_tonemap_simd(value),
            Linear => value.simd_clamp(Simd::splat(0.0), Simd::splat(1.0)),
            Local => value.simd_clamp(Simd::splat(0.0), Simd::splat(1.0)), // Fallback for local tone mapping
        }
    }
    
    fn apply_gamma(&self, value: f32x4, gamma_inv: f32, use_srgb: bool) -> f32x4 {
        if use_srgb {
            srgb_oetf_simd(value)
        } else {
            apply_gamma_simd(value, gamma_inv)
        }
    }
    
    fn clamp_unit(&self, value: f32x4) -> f32x4 {
        value.simd_clamp(Simd::splat(0.0), Simd::splat(1.0))
    }
    
    fn is_finite(&self, value: f32x4) -> f32x4 {
        value.is_finite().select(Simd::splat(1.0), Simd::splat(0.0))
    }
    
    fn select_finite(&self, value: f32x4, fallback: f32x4) -> f32x4 {
        let is_valid = value.is_finite() & value.simd_ge(Simd::splat(0.0));
        is_valid.select(value, fallback)
    }
    
    fn splat(&self, value: f32) -> f32x4 {
        Simd::splat(value)
    }
    
    fn zero(&self) -> f32x4 {
        Simd::splat(0.0)
    }
    
    fn one(&self) -> f32x4 {
        Simd::splat(1.0)
    }
    
    fn to_u8_clamped(&self, value: f32x4) -> f32x4 {
        (value * Simd::splat(255.0)).simd_clamp(Simd::splat(0.0), Simd::splat(255.0))
    }
}

/// Unified processing function that works with both scalar and SIMD types
pub fn process_pixel_unified<T: Copy, P: SimdProcessable<T>>(
    processor: &P,
    r: T, g: T, b: T, a: T,
    params: &ProcessParams,
) -> (T, T, T, T) {
    let exposure_multiplier = processor.splat(2.0_f32.powf(params.exposure));
    let zero = processor.zero();
    
    // Clean up inputs (handle NaN/Inf)
    let clean_r = processor.select_finite(r, zero);
    let clean_g = processor.select_finite(g, zero);
    let clean_b = processor.select_finite(b, zero);
    let clean_a = processor.select_finite(a, processor.one());
    
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

/// Generate both scalar and SIMD versions of tone mapping functions
macro_rules! generate_tonemap_functions {
    ($name:ident, $formula:expr) => {
        paste::paste! {
            /// Scalar version
            pub fn [<$name _scalar>](x: f32) -> f32 {
                let formula = $formula;
                formula(x)
            }
            
            /// SIMD version  
            pub fn [<$name _simd>](x: f32x4) -> f32x4 {
                let formula = $formula;
                // Apply formula element-wise
                let arr: [f32; 4] = x.into();
                let result: [f32; 4] = [
                    formula(arr[0]),
                    formula(arr[1]), 
                    formula(arr[2]),
                    formula(arr[3])
                ];
                Simd::from_array(result)
            }
        }
    };
}

// Generate tone mapping functions using the macro
generate_tonemap_functions!(aces_tonemap, |x: f32| {
    let a = 2.51;
    let b = 0.03;  
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
});

generate_tonemap_functions!(reinhard_tonemap, |x: f32| {
    x / (1.0 + x)
});

generate_tonemap_functions!(filmic_tonemap, |x: f32| {
    let x_max = 0.22 * 11.2;
    let linear = 0.22 * x;
    let squared = 0.1 * x * x;
    ((linear + squared) / (1.0 + linear + squared)).min(x_max)
});

generate_tonemap_functions!(hable_tonemap, |x: f32| {
    let a = 0.15;
    let b = 0.50;
    let c = 0.10;
    let d = 0.20;
    let e = 0.02;
    let f = 0.30;
    let w = 11.2;
    
    let curr = ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
    let white_scale = ((w * (a * w + c * b) + d * e) / (w * (a * w + b) + d * f)) - e / f;
    (curr / white_scale).clamp(0.0, 1.0)
});

/// Scalar sRGB OETF
pub fn srgb_oetf_scalar(linear: f32) -> f32 {
    if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// SIMD sRGB OETF - reuse existing implementation
pub fn srgb_oetf_simd(linear: f32x4) -> f32x4 {
    crate::processing::tone_mapping::srgb_oetf_simd(linear)
}

/// Scalar gamma correction
pub fn apply_gamma_scalar(value: f32, gamma_inv: f32) -> f32 {
    value.powf(gamma_inv).clamp(0.0, 1.0)
}

/// SIMD gamma correction - reuse existing implementation
pub fn apply_gamma_simd(values: f32x4, gamma_inv: f32) -> f32x4 {
    crate::processing::tone_mapping::apply_gamma_lut_simd(values, gamma_inv)
}

/// High-level processing functions

/// Process scalar pixels using unified approach
pub fn process_pixels_scalar(
    input: &[f32],
    output: &mut [u8],
    params: &ProcessParams,
) {
    let processor = ScalarProcessor;
    let pixel_count = input.len() / 4;
    
    for i in 0..pixel_count {
        let base = i * 4;
        let r = input[base];
        let g = input[base + 1];
        let b = input[base + 2];
        let a = input[base + 3];
        
        let (final_r, final_g, final_b, final_a) = process_pixel_unified(&processor, r, g, b, a, params);
        
        let out_base = i * 4;
        output[out_base] = processor.to_u8_clamped(final_r) as u8;
        output[out_base + 1] = processor.to_u8_clamped(final_g) as u8;
        output[out_base + 2] = processor.to_u8_clamped(final_b) as u8;
        output[out_base + 3] = processor.to_u8_clamped(final_a) as u8;
    }
}

/// Process SIMD pixels using unified approach  
pub fn process_pixels_simd(
    input: &[f32],
    output: &mut [u8],
    params: &ProcessParams,
) {
    let processor = SimdProcessor;
    let simd_count = input.len() / 16; // 4 pixels * 4 channels = 16 values
    
    for i in 0..simd_count {
        let base = i * 16;
        
        // Load 4 pixels worth of data
        let r = f32x4::from_array([input[base], input[base + 4], input[base + 8], input[base + 12]]);
        let g = f32x4::from_array([input[base + 1], input[base + 5], input[base + 9], input[base + 13]]);
        let b = f32x4::from_array([input[base + 2], input[base + 6], input[base + 10], input[base + 14]]);
        let a = f32x4::from_array([input[base + 3], input[base + 7], input[base + 11], input[base + 15]]);
        
        let (final_r, final_g, final_b, final_a) = process_pixel_unified(&processor, r, g, b, a, params);
        
        // Store results
        let r_bytes = processor.to_u8_clamped(final_r);
        let g_bytes = processor.to_u8_clamped(final_g); 
        let b_bytes = processor.to_u8_clamped(final_b);
        let a_bytes = processor.to_u8_clamped(final_a);
        
        let r_arr: [f32; 4] = r_bytes.into();
        let g_arr: [f32; 4] = g_bytes.into();
        let b_arr: [f32; 4] = b_bytes.into();
        let a_arr: [f32; 4] = a_bytes.into();
        
        let out_base = i * 16;
        for j in 0..4 {
            let pixel_out = out_base + j * 4;
            output[pixel_out] = r_arr[j] as u8;
            output[pixel_out + 1] = g_arr[j] as u8;
            output[pixel_out + 2] = b_arr[j] as u8;
            output[pixel_out + 3] = a_arr[j] as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_scalar_vs_simd_consistency() {
        let params = ProcessParams::new(0.0, 2.2, 0); // ACES tone mapping
        
        // Test pixel values
        let r = 2.0;
        let g = 1.5; 
        let b = 0.8;
        let a = 1.0;
        
        // Scalar processing
        let scalar_proc = ScalarProcessor;
        let (sr, sg, sb, sa) = process_pixel_unified(&scalar_proc, r, g, b, a, &params);
        
        // SIMD processing (single pixel replicated)
        let simd_proc = SimdProcessor;
        let r_simd = Simd::splat(r);
        let g_simd = Simd::splat(g);
        let b_simd = Simd::splat(b);
        let a_simd = Simd::splat(a);
        
        let (vr, vg, vb, va) = process_pixel_unified(&simd_proc, r_simd, g_simd, b_simd, a_simd, &params);
        let vr_arr: [f32; 4] = vr.into();
        let vg_arr: [f32; 4] = vg.into();
        let vb_arr: [f32; 4] = vb.into();
        let va_arr: [f32; 4] = va.into();
        
        // Results should be nearly identical (allowing for floating point precision)
        assert!((sr - vr_arr[0]).abs() < 1e-6);
        assert!((sg - vg_arr[0]).abs() < 1e-6);
        assert!((sb - vb_arr[0]).abs() < 1e-6);
        assert!((sa - va_arr[0]).abs() < 1e-6);
    }
    
    #[test]
    fn test_generated_tonemap_functions() {
        let test_value = 0.5;
        
        // Test ACES
        let scalar_result = aces_tonemap_scalar(test_value);
        let simd_input = Simd::splat(test_value);
        let simd_result = aces_tonemap_simd(simd_input);
        let simd_arr: [f32; 4] = simd_result.into();
        
        assert!((scalar_result - simd_arr[0]).abs() < 1e-6);
        assert!(scalar_result >= 0.0 && scalar_result <= 1.0);
    }
}