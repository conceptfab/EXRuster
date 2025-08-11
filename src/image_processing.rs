use slint::Rgba8Pixel;

/// Przetwarza pojedynczy piksel z wartościami HDR na 8-bitowe RGB
pub fn process_pixel(r: f32, g: f32, b: f32, a: f32, exposure: f32, gamma: f32) -> Rgba8Pixel {
    let exposure_multiplier = 2.0_f32.powf(exposure);
    let gamma_inv = 1.0 / gamma.max(1e-4);
    
    // Sprawdzenie NaN/Inf i clamp do sensownych wartości
    let safe_r = if r.is_finite() { r.max(0.0) } else { 0.0 };
    let safe_g = if g.is_finite() { g.max(0.0) } else { 0.0 };
    let safe_b = if b.is_finite() { b.max(0.0) } else { 0.0 };
    let safe_a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 1.0 };
    
    // Zastosowanie ekspozycji
    let exposed_r = safe_r * exposure_multiplier;
    let exposed_g = safe_g * exposure_multiplier;
    let exposed_b = safe_b * exposure_multiplier;
    
    // ACES tone mapping (lepszy niż Reinhard)
    let tone_mapped_r = aces_tonemap(exposed_r);
    let tone_mapped_g = aces_tonemap(exposed_g);
    let tone_mapped_b = aces_tonemap(exposed_b);
    
    // Optymalizowana gamma correction
    let corrected_r = apply_gamma_fast(tone_mapped_r, gamma_inv);
    let corrected_g = apply_gamma_fast(tone_mapped_g, gamma_inv);
    let corrected_b = apply_gamma_fast(tone_mapped_b, gamma_inv);
    
    Rgba8Pixel {
        r: (corrected_r * 255.0).round().clamp(0.0, 255.0) as u8,
        g: (corrected_g * 255.0).round().clamp(0.0, 255.0) as u8,
        b: (corrected_b * 255.0).round().clamp(0.0, 255.0) as u8,
        a: (safe_a * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

/// ACES tone mapping - znacznie lepszy od Reinhard
#[inline]
fn aces_tonemap(x: f32) -> f32 {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
}

/// Szybka gamma correction z lookup table dla typowych wartości
#[inline]
fn apply_gamma_fast(value: f32, gamma_inv: f32) -> f32 {
    match gamma_inv {
        // Usunięto błędną optymalizację dla sRGB (1/2.2).
        // Teraz jest to obsługiwane przez poprawny, ogólny przypadek `powf`.
        x if (x - 0.5).abs() < 0.001 => {
            // Gamma 2.0
            value.sqrt()
        },
        x if (x - 1.0).abs() < 0.001 => {
            // Gamma 1.0 (linear)
            value
        },
        _ => value.powf(gamma_inv)
    }
}

// usunięto nieużywaną funkcję read_exr_to_slint_image
