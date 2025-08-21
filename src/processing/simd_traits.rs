use crate::processing::tone_mapping::ToneMapMode;

/// Parameters for pixel processing operations
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct ProcessParams {
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: ToneMapMode,
}

impl ProcessParams {
    #[allow(dead_code)]
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
#[allow(dead_code)]
pub trait SimdProcessable<T> {
    /// Apply exposure correction
    fn apply_exposure(&self, value: T, exposure_multiplier: T) -> T;
    
    /// Apply tone mapping operator
    fn apply_tonemap(&self, value: T, mode: ToneMapMode) -> T;
    
    /// Apply gamma correction or sRGB OETF
    fn apply_gamma(&self, value: T, gamma_inv: f32, use_srgb: bool) -> T;
    
    /// Clamp values to valid range [0, 1]
    fn clamp_unit(&self, value: T) -> T;
    
    
    /// Select values based on condition (finite check)
    fn select_finite(&self, value: T, fallback: T) -> T;
    
    /// Create a splat (broadcast) value
    fn splat(&self, value: f32) -> T;
    
    /// Get zero value
    fn zero(&self) -> T;
    
    /// Get one value
    fn one(&self) -> T;
    
}

/// Scalar implementation for f32
#[allow(dead_code)]
pub struct ScalarProcessor;

impl SimdProcessable<f32> for ScalarProcessor {
    fn apply_exposure(&self, value: f32, exposure_multiplier: f32) -> f32 {
        value * exposure_multiplier
    }
    
    fn apply_tonemap(&self, value: f32, mode: ToneMapMode) -> f32 {
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
    
    fn apply_gamma(&self, value: f32, gamma_inv: f32, use_srgb: bool) -> f32 {
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
    
    fn clamp_unit(&self, value: f32) -> f32 {
        value.clamp(0.0, 1.0)
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
    
}


/// Unified processing function that works with scalar types
#[allow(dead_code)]
pub fn process_pixel_unified<P: SimdProcessable<f32>>(
    processor: &P,
    r: f32, g: f32, b: f32, a: f32,
    params: &ProcessParams,
) -> (f32, f32, f32, f32) {
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


#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_scalar_processing() {
        let params = ProcessParams::new(0.0, 2.2, 0); // ACES tone mapping
        
        // Test pixel values
        let r = 2.0;
        let g = 1.5; 
        let b = 0.8;
        let a = 1.0;
        
        // Scalar processing
        let scalar_proc = ScalarProcessor;
        let (sr, sg, sb, sa) = process_pixel_unified(&scalar_proc, r, g, b, a, &params);
        
        // Results should be valid
        assert!(sr >= 0.0 && sr <= 1.0);
        assert!(sg >= 0.0 && sg <= 1.0);
        assert!(sb >= 0.0 && sb <= 1.0);
        assert!(sa >= 0.0 && sa <= 1.0);
    }
}