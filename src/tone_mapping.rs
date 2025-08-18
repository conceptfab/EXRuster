use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;

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
pub fn apply_tonemap_simd(r: f32x4, g: f32x4, b: f32x4, mode: ToneMapMode) -> (f32x4, f32x4, f32x4) {
    match mode {
        ToneMapMode::ACES => (aces_tonemap_simd(r), aces_tonemap_simd(g), aces_tonemap_simd(b)),
        ToneMapMode::Reinhard => (reinhard_tonemap_simd(r), reinhard_tonemap_simd(g), reinhard_tonemap_simd(b)),
        ToneMapMode::Linear => {
            let zero = Simd::splat(0.0);
            let one = Simd::splat(1.0);
            (r.simd_clamp(zero, one), g.simd_clamp(zero, one), b.simd_clamp(zero, one))
        },
        _ => (aces_tonemap_simd(r), aces_tonemap_simd(g), aces_tonemap_simd(b)), // Fallback
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
}
