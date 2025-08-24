use slint::Rgba8Pixel;

// Import funkcji tone mapping z tone_mapping.rs
use crate::processing::tone_mapping::{ToneMapMode, ToneMapModeId};

// Thread-local cache LUT został usunięty - funkcja apply_gamma_lut
// została przeniesiona do tone_mapping.rs

// Funkcja apply_gamma_lut została przeniesiona do tone_mapping.rs
// aby uniknąć duplikacji kodu

/// Przetwarza pojedynczy piksel z wartościami HDR na 8-bitowe RGB
/// tonemap_mode: 0 = ACES, 1 = Reinhard, 2 = Linear (brak tone-map)
pub fn process_pixel(
    r: f32,
    g: f32,
    b: f32,
    a: f32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> Rgba8Pixel {
    let (corrected_r, corrected_g, corrected_b) =
        tone_map_and_gamma(r, g, b, exposure, gamma, tonemap_mode);

    let safe_a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 1.0 };

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

/// Wspólny pipeline: ekspozycja → tone-map (wg trybu) → gamma/sRGB
/// Zwraca wartości w [0, 1] po korekcji gamma.
/// Wykorzystuje skonsolidowaną implementację z tone_mapping.rs
/// New type-safe API using ToneMapModeId
#[allow(dead_code)]
#[inline]
pub fn tone_map_and_gamma_safe(
    r: f32,
    g: f32,
    b: f32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: ToneMapModeId,
) -> (f32, f32, f32) {
    crate::processing::tone_mapping::tone_map_and_gamma_safe(r, g, b, exposure, gamma, tonemap_mode)
}

/// Legacy API for backward compatibility - consider using tone_map_and_gamma_safe instead
pub fn tone_map_and_gamma(
    r: f32,
    g: f32,
    b: f32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> (f32, f32, f32) {
    let mode = ToneMapMode::from(tonemap_mode);
    crate::processing::tone_mapping::tone_map_and_gamma(r, g, b, exposure, gamma, mode)
}

// ===================== SIMD warianty =====================
// Wszystkie funkcje SIMD tone mapping zostały przeniesione do tone_mapping.rs
// aby uniknąć duplikacji kodu

// Funkcja tone_map_and_gamma_simd została przeniesiona do tone_mapping.rs
// aby uniknąć duplikacji kodu

