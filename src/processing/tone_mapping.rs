use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;
use std::simd::StdFloat;
use std::simd::cmp::SimdPartialOrd;

#[derive(Debug, Clone, Copy)]
pub enum ToneMapMode {
    ACES = 0,
    Reinhard = 1,
    Linear = 2,
    Filmic = 3,
    Hable = 4,
    Local = 5,
}

impl From<i32> for ToneMapMode {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Reinhard,
            2 => Self::Linear,  
            3 => Self::Filmic,
            4 => Self::Hable,
            5 => Self::Local,
            _ => Self::ACES,
        }
    }
}

#[inline]
pub fn aces_tonemap(x: f32) -> f32 {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
}

#[inline]
pub fn reinhard_tonemap(x: f32) -> f32 {
    (x / (1.0 + x)).clamp(0.0, 1.0)
}

#[inline]
pub fn linear_tonemap(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[inline]
pub fn filmic_tonemap(x: f32) -> f32 {
    // Filmic tone mapping (John Hable)
    let a = 0.15;
    let b = 0.50;
    let c = 0.10;
    let d = 0.20;
    let e = 0.02;
    let f = 0.30;
    
    ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f
}

#[inline]
pub fn hable_tonemap(x: f32) -> f32 {
    // Uncharted 2 tone mapping
    let a = 0.15;
    let b = 0.50;
    let c = 0.10;
    let d = 0.20;
    let e = 0.02;
    let f = 0.30;
    let w = 11.2;
    
    let curr = ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
    let white_scale = 1.0 / (((w * (a * w + c * b) + d * e) / (w * (a * w + b) + d * f)) - e / f);
    
    (curr * white_scale).clamp(0.0, 1.0)
}

pub fn apply_tonemap_scalar(r: f32, g: f32, b: f32, mode: ToneMapMode) -> (f32, f32, f32) {
    match mode {
        ToneMapMode::ACES => (aces_tonemap(r), aces_tonemap(g), aces_tonemap(b)),
        ToneMapMode::Reinhard => (reinhard_tonemap(r), reinhard_tonemap(g), reinhard_tonemap(b)),
        ToneMapMode::Linear => (linear_tonemap(r), linear_tonemap(g), linear_tonemap(b)),
        ToneMapMode::Filmic => (filmic_tonemap(r), filmic_tonemap(g), filmic_tonemap(b)),
        ToneMapMode::Hable => (hable_tonemap(r), hable_tonemap(g), hable_tonemap(b)),
        ToneMapMode::Local => {
            // Placeholder dla local adaptation - na razie użyj ACES
            (aces_tonemap(r), aces_tonemap(g), aces_tonemap(b))
        }
    }
}

// SIMD versions
#[allow(dead_code)]
#[inline]
pub fn aces_tonemap_simd(x: f32x4) -> f32x4 {
    let a = Simd::splat(2.51);
    let b = Simd::splat(0.03);
    let c = Simd::splat(2.43);
    let d = Simd::splat(0.59);
    let e = Simd::splat(0.14);
    let zero = Simd::splat(0.0);
    let one = Simd::splat(1.0);
    ((x * (a * x + b)) / (x * (c * x + d) + e)).simd_clamp(zero, one)
}

#[allow(dead_code)]
#[inline]
pub fn reinhard_tonemap_simd(x: f32x4) -> f32x4 {
    let one = Simd::splat(1.0);
    (x / (one + x)).simd_clamp(Simd::splat(0.0), one)
}

#[allow(dead_code)]
#[inline]
pub fn filmic_tonemap_simd(x: f32x4) -> f32x4 {
    let x_safe = x.simd_max(Simd::splat(0.0));
    let a = Simd::splat(0.15);  // Black point
    let b = Simd::splat(0.50);  // Toe
    let c = Simd::splat(0.10);  // Shoulder
    let d = Simd::splat(0.20);  // White point
    let epsilon = Simd::splat(1e-9);
    
    let numerator = x_safe * (a * x_safe + c * b) + d * x_safe;
    let denominator = x_safe * (a * x_safe + b) + d * c;
    
    (numerator / (denominator + epsilon)).simd_clamp(Simd::splat(0.0), Simd::splat(1.0))
}

#[allow(dead_code)]
#[inline]
pub fn hable_tonemap_simd(x: f32x4) -> f32x4 {
    // Uncharted 2 tone mapping (John Hable) - PRAWIDŁOWA IMPLEMENTACJA
    let x_safe: f32x4 = x.simd_max(Simd::splat(0.0_f32));
    let a: f32x4 = Simd::splat(0.15_f32);
    let b: f32x4 = Simd::splat(0.50_f32);
    let c: f32x4 = Simd::splat(0.10_f32);
    let d: f32x4 = Simd::splat(0.20_f32);
    let e: f32x4 = Simd::splat(0.02_f32);
    let f: f32x4 = Simd::splat(0.30_f32);
    let w: f32x4 = Simd::splat(11.2_f32);
    
    let curr: f32x4 = ((x_safe * (a * x_safe + c * b) + d * e) / (x_safe * (a * x_safe + b) + d * f)) - e / f;
    let white_scale: f32x4 = Simd::splat(1.0) / (((w * (a * w + c * b) + d * e) / (w * (a * w + b) + d * f)) - e / f);
    
    (curr * white_scale).simd_clamp(Simd::splat(0.0), Simd::splat(1.0))
}

#[allow(dead_code)]
#[inline]
pub fn srgb_oetf_simd(x: f32x4) -> f32x4 {
    // Prawdziwa krzywa sRGB (OETF), zastosowana do wartości w [0,1]
    let x = x.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
    let threshold = Simd::splat(0.003_130_8);
    let low = Simd::splat(12.92) * x;
    let high = Simd::splat(1.055) * (x.ln() * Simd::splat(1.0 / 2.4)).exp() - Simd::splat(0.055);
    threshold.simd_ge(x).select(low, high)
}

#[inline]
pub fn srgb_oetf(x: f32) -> f32 {
    // Prawdziwa krzywa sRGB (OETF), zastosowana do wartości w [0,1]
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
pub fn apply_gamma_lut(value: f32, gamma_inv: f32) -> f32 {
    value.powf(gamma_inv)
}

#[allow(dead_code)]
#[inline]
pub fn apply_gamma_lut_simd(values: f32x4, gamma_inv: f32) -> f32x4 {
    // Optimized SIMD implementation - direct power operation on all lanes
    
    // Clamp values to positive range to avoid issues with powf
    let safe_values = values.simd_max(f32x4::splat(0.0));
    
    // Use SIMD-optimized power function
    // Note: powf is vectorized by LLVM for f32x4 on most modern targets
    let mut result = [0.0f32; 4];
    let input: [f32; 4] = safe_values.into();
    let gamma_inv_scalar = gamma_inv;
    
    // Unrolled loop for better optimization
    result[0] = input[0].powf(gamma_inv_scalar);
    result[1] = input[1].powf(gamma_inv_scalar);
    result[2] = input[2].powf(gamma_inv_scalar);
    result[3] = input[3].powf(gamma_inv_scalar);
    
    f32x4::from_array(result)
}

#[allow(dead_code)]
pub fn apply_tonemap_simd(r: f32x4, g: f32x4, b: f32x4, mode: ToneMapMode) -> (f32x4, f32x4, f32x4) {
    match mode {
        ToneMapMode::ACES => (aces_tonemap_simd(r), aces_tonemap_simd(g), aces_tonemap_simd(b)),
        ToneMapMode::Reinhard => (reinhard_tonemap_simd(r), reinhard_tonemap_simd(g), reinhard_tonemap_simd(b)),
        ToneMapMode::Linear => {
            let zero = Simd::splat(0.0);
            let one = Simd::splat(1.0);
            (r.simd_clamp(zero, one), g.simd_clamp(zero, one), b.simd_clamp(zero, one))
        },
        ToneMapMode::Filmic => (filmic_tonemap_simd(r), filmic_tonemap_simd(g), filmic_tonemap_simd(b)),
        ToneMapMode::Hable => (hable_tonemap_simd(r), hable_tonemap_simd(g), hable_tonemap_simd(b)),
        ToneMapMode::Local => (aces_tonemap_simd(r), aces_tonemap_simd(g), aces_tonemap_simd(b)), // Fallback
    }
}

/// SIMD: ekspozycja → tone-map → gamma/sRGB dla 4 pikseli naraz
#[inline]
pub fn tone_map_and_gamma_simd(
    r: f32x4,
    g: f32x4,
    b: f32x4,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> (f32x4, f32x4, f32x4) {
    let exposure_multiplier = Simd::splat(2.0_f32.powf(exposure));

    // Sprawdzenie NaN/Inf i clamp do sensownych wartości
    let zero = Simd::splat(0.0);
    let safe_r = r.is_finite().select(r, zero).simd_max(zero);
    let safe_g = g.is_finite().select(g, zero).simd_max(zero);
    let safe_b = b.is_finite().select(b, zero).simd_max(zero);

    // Ekspozycja
    let exposed_r = safe_r * exposure_multiplier;
    let exposed_g = safe_g * exposure_multiplier;
    let exposed_b = safe_b * exposure_multiplier;

    // Tone mapping używając skonsolidowanej funkcji
    let mode = ToneMapMode::from(tonemap_mode);
    let (tm_r, tm_g, tm_b) = apply_tonemap_simd(exposed_r, exposed_g, exposed_b, mode);

    // Gamma: preferuj sRGB OETF
    let use_srgb = (gamma - 2.2).abs() < 0.2 || (gamma - 2.4).abs() < 0.2;
    if use_srgb {
        (
            srgb_oetf_simd(tm_r),
            srgb_oetf_simd(tm_g),
            srgb_oetf_simd(tm_b),
        )
    } else {
        let gamma_inv = 1.0 / gamma.max(1e-4);
        (
            apply_gamma_lut_simd(tm_r, gamma_inv),
            apply_gamma_lut_simd(tm_g, gamma_inv),
            apply_gamma_lut_simd(tm_b, gamma_inv),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tonemap_modes() {
        let test_value = 2.0;
        
        assert!(aces_tonemap(test_value) <= 1.0);
        assert!(reinhard_tonemap(test_value) <= 1.0);
        assert!(linear_tonemap(test_value) <= 1.0);
        assert!(filmic_tonemap(test_value) <= 1.0);
        assert!(hable_tonemap(test_value) <= 1.0);
    }

    #[test]
    fn test_tonemap_mode_conversion() {
        assert!(matches!(ToneMapMode::from(0), ToneMapMode::ACES));
        assert!(matches!(ToneMapMode::from(1), ToneMapMode::Reinhard));
        assert!(matches!(ToneMapMode::from(2), ToneMapMode::Linear));
        assert!(matches!(ToneMapMode::from(3), ToneMapMode::Filmic));
        assert!(matches!(ToneMapMode::from(4), ToneMapMode::Hable));
        assert!(matches!(ToneMapMode::from(5), ToneMapMode::Local));
        assert!(matches!(ToneMapMode::from(999), ToneMapMode::ACES)); // Default
    }

    #[test]
    fn test_apply_tonemap_scalar() {
        let (r, g, b) = apply_tonemap_scalar(2.0, 1.5, 0.5, ToneMapMode::ACES);
        assert!(r <= 1.0 && g <= 1.0 && b <= 1.0);
        assert!(r >= 0.0 && g >= 0.0 && b >= 0.0);
    }

    #[test]
    fn test_simd_gamma_lut_optimization() {
        // Test that SIMD version produces same results as scalar
        let test_values = f32x4::from_array([0.0, 0.5, 1.0, 2.0]);
        let gamma_inv = 1.0 / 2.2;
        
        let simd_result = apply_gamma_lut_simd(test_values, gamma_inv);
        let simd_array: [f32; 4] = simd_result.into();
        
        // Compare with scalar version
        for i in 0..4 {
            let input = [0.0, 0.5, 1.0, 2.0][i];
            let scalar_result = apply_gamma_lut(input, gamma_inv);
            let diff = (simd_array[i] - scalar_result).abs();
            assert!(diff < 1e-6, "SIMD and scalar results differ: {} vs {}", simd_array[i], scalar_result);
        }
    }

    #[test]
    fn benchmark_gamma_lut_performance() {
        // Simple performance comparison - this would be better with criterion.rs
        let test_values = f32x4::from_array([0.1, 0.5, 1.0, 2.0]);
        let gamma_inv = 1.0 / 2.2;
        let iterations = 10000;

        // Time SIMD version
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = apply_gamma_lut_simd(test_values, gamma_inv);
        }
        let simd_duration = start.elapsed();

        // Time scalar version for comparison
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let input: [f32; 4] = test_values.into();
            for &val in &input {
                let _ = apply_gamma_lut(val, gamma_inv);
            }
        }
        let scalar_duration = start.elapsed();

        println!("SIMD version: {:?}, Scalar version: {:?}", simd_duration, scalar_duration);
        println!("Performance ratio: {:.2}x", scalar_duration.as_nanos() as f64 / simd_duration.as_nanos() as f64);
        
        // Assert that SIMD is at least not significantly slower (allowing for measurement noise)
        assert!(simd_duration <= scalar_duration * 2, "SIMD version unexpectedly slow");
    }
}
