use slint::Rgba8Pixel;

// Import funkcji tone mapping z tone_mapping.rs
use crate::processing::tone_mapping::{
    apply_gamma_lut,
    srgb_oetf,
};

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
#[inline]
pub fn tone_map_and_gamma(
    r: f32,
    g: f32,
    b: f32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> (f32, f32, f32) {
    let exposure_multiplier = 2.0_f32.powf(exposure);

    // Sprawdzenie NaN/Inf i clamp do sensownych wartości
    let safe_r = if r.is_finite() { r.max(0.0) } else { 0.0 };
    let safe_g = if g.is_finite() { g.max(0.0) } else { 0.0 };
    let safe_b = if b.is_finite() { b.max(0.0) } else { 0.0 };

    // Zastosowanie ekspozycji
    let exposed_r = safe_r * exposure_multiplier;
    let exposed_g = safe_g * exposure_multiplier;
    let exposed_b = safe_b * exposure_multiplier;

    // Tone mapping używając skonsolidowanej funkcji
    let mode = crate::processing::tone_mapping::ToneMapMode::from(tonemap_mode);
    let (tm_r, tm_g, tm_b) = crate::processing::tone_mapping::apply_tonemap_scalar(exposed_r, exposed_g, exposed_b, mode);

    // Korekcja wyjściowa: preferuj prawdziwą krzywą sRGB (OETF) dla gamma ~2.2/2.4; w innym wypadku użyj niestandardowej gammy
    let use_srgb = (gamma - 2.2).abs() < 0.2 || (gamma - 2.4).abs() < 0.2;
    if use_srgb {
        (
            srgb_oetf(tm_r),
            srgb_oetf(tm_g),
            srgb_oetf(tm_b),
        )
    } else {
        let gamma_inv = 1.0 / gamma.max(1e-4);
        (
            apply_gamma_lut(tm_r, gamma_inv),
            apply_gamma_lut(tm_g, gamma_inv),
            apply_gamma_lut(tm_b, gamma_inv),
        )
    }
}

// ===================== SIMD warianty =====================
// Wszystkie funkcje SIMD tone mapping zostały przeniesione do tone_mapping.rs
// aby uniknąć duplikacji kodu

// Funkcja tone_map_and_gamma_simd została przeniesiona do tone_mapping.rs
// aby uniknąć duplikacji kodu

