use slint::Rgba8Pixel;

// Import funkcji tone mapping z tone_mapping.rs
use crate::processing::tone_mapping::ToneMapMode;

// Thread-local cache LUT został usunięty - funkcja apply_gamma_lut
// została przeniesiona do tone_mapping.rs

// Funkcja apply_gamma_lut została przeniesiona do tone_mapping.rs
// aby uniknąć duplikacji kodu

/// Przetwarza pojedynczy piksel z wartościami HDR na 8-bitowe RGB.
/// Parametry frame-invariantne precomputowane przez wywołującego:
/// `exposure_multiplier = 2^exposure`, `gamma_inv = 1/max(gamma, 1e-4)`,
/// `use_srgb = sRGB detection`, `mode = ToneMapMode`.
#[allow(clippy::too_many_arguments)]
pub fn process_pixel(
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    exposure_multiplier: f32,
    gamma_inv: f32,
    use_srgb: bool,
    mode: ToneMapMode,
) -> Rgba8Pixel {
    let (corrected_r, corrected_g, corrected_b) =
        crate::processing::tone_mapping::tone_map_and_gamma(
            r, g, b, exposure_multiplier, gamma_inv, use_srgb, mode,
        );

    let safe_a = if a.is_finite() {
        a.clamp(0.0, 1.0)
    } else {
        1.0
    };

    Rgba8Pixel {
        r: (corrected_r * 255.0).round().clamp(0.0, 255.0) as u8,
        g: (corrected_g * 255.0).round().clamp(0.0, 255.0) as u8,
        b: (corrected_b * 255.0).round().clamp(0.0, 255.0) as u8,
        a: (safe_a * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

// Usunięte duplikaty tone mapping - przeniesione do tone_mapping.rs

// Funkcja srgb_oetf została przeniesiona do tone_mapping.rs
// aby uniknąć duplikacji kodu


// ===================== SIMD warianty =====================
// Wszystkie funkcje SIMD tone mapping zostały przeniesione do tone_mapping.rs
// aby uniknąć duplikacji kodu

// Funkcja tone_map_and_gamma_simd została przeniesiona do tone_mapping.rs
// aby uniknąć duplikacji kodu
