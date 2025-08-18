use slint::Rgba8Pixel;
use std::cell::RefCell;
use core::simd::{f32x4, Simd};
use std::simd::prelude::{SimdFloat, SimdPartialOrd};
use std::simd::StdFloat;

// Thread-local cache LUT dla gammy, aby bezpiecznie działać z Rayon
thread_local! {
    static GAMMA_LUT_CACHE: RefCell<Option<(f32, [f32; 1024])>> = RefCell::new(None);
}

#[inline]
fn apply_gamma_lut(value: f32, gamma_inv: f32) -> f32 {
    // Zakładamy wejście w [0,1] po tone-mappingu; clamp dla pewności
    let v = value.clamp(0.0, 1.0);

    // Szybkie ścieżki dla typowych przypadków
    if (gamma_inv - 1.0).abs() < 1e-6 {
        return v;
    }

    // Pobierz lub zbuduj LUT dla danej wartości gamma_inv
    let y = GAMMA_LUT_CACHE.with(|cell| {
        let mut opt = cell.borrow_mut();
        let need_rebuild = match *opt {
            Some((stored_inv, _)) => (stored_inv - gamma_inv).abs() > 1e-6,
            None => true,
        };

        if need_rebuild {
            let mut table = [0.0_f32; 1024];
            let denom = (table.len() - 1) as f32;
            for (i, slot) in table.iter_mut().enumerate() {
                let x = (i as f32) / denom;
                *slot = x.powf(gamma_inv);
            }
            *opt = Some((gamma_inv, table));
        }

        // Interpolacja liniowa z LUT
        if let Some((_, table)) = *opt {
            let max_idx = (table.len() - 1) as f32;
            let fidx = v * max_idx;
            let lo = fidx.floor() as usize;
            let hi = fidx.ceil() as usize;
            if lo == hi {
                table[lo]
            } else {
                let t = fidx - lo as f32;
                let a = table[lo];
                let b = table[hi];
                a + (b - a) * t
            }
        } else {
            // Nie powinno się zdarzyć, fallback
            v.powf(gamma_inv)
        }
    });

    y
}

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



#[inline]
fn srgb_oetf(x: f32) -> f32 {
    // Prawdziwa krzywa sRGB (OETF), zastosowana do wartości w [0,1]
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

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
    let mode = crate::tone_mapping::ToneMapMode::from(tonemap_mode);
    let (tm_r, tm_g, tm_b) = crate::tone_mapping::apply_tonemap_scalar(exposed_r, exposed_g, exposed_b, mode);

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

#[inline]
fn aces_tonemap_simd(x: f32x4) -> f32x4 {
    let a = Simd::splat(2.51);
    let b = Simd::splat(0.03);
    let c = Simd::splat(2.43);
    let d = Simd::splat(0.59);
    let e = Simd::splat(0.14);
    let zero = Simd::splat(0.0);
    let one = Simd::splat(1.0);
    ((x * (a * x + b)) / (x * (c * x + d) + e)).simd_clamp(zero, one)
}

#[inline]
fn reinhard_tonemap_simd(x: f32x4) -> f32x4 {
    let one = Simd::splat(1.0);
    (x / (one + x)).simd_clamp(Simd::splat(0.0), one)
}

#[inline]
fn filmic_tonemap_simd(x: f32x4) -> f32x4 {
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

#[inline]
fn hable_tonemap_simd(x: f32x4) -> f32x4 {
    let x_safe = x.simd_max(Simd::splat(0.0));
    let a = Simd::splat(0.15);
    let b = Simd::splat(0.50);
    let c = Simd::splat(0.10);
    let d = Simd::splat(0.20);
    let e = Simd::splat(0.02);
    let w = Simd::splat(11.2);
    let epsilon = Simd::splat(1e-9);
    
    let numerator = (x_safe * (a * x_safe + c * b) + d * e) * (x_safe * (a * x_safe + b) + d * c);
    let denominator = (x_safe * (a * x_safe + b) + d * c) * (x_safe * (a * x_safe + c * b) + d * e);
    
    let white_scale = Simd::splat(1.0) / (((w * (a * w + c * b) + d * e) * (w * (a * w + b) + d * c)) / ((w * (a * w + b) + d * c) * (w * (a * w + c * b) + d * e)));
    
    (numerator / (denominator + epsilon) * white_scale).simd_clamp(Simd::splat(0.0), Simd::splat(1.0))
}

#[inline]
fn srgb_oetf_simd(x: f32x4) -> f32x4 {
    // Prawdziwa krzywa sRGB (OETF), zastosowana do wartości w [0,1]
    let x = x.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
    let threshold = Simd::splat(0.003_130_8);
    let low = Simd::splat(12.92) * x;
    let high = Simd::splat(1.055) * (x.ln() * Simd::splat(1.0 / 2.4)).exp() - Simd::splat(0.055);
    threshold.simd_gt(x).select(low, high)
}

#[inline]
fn apply_gamma_lut_simd(values: f32x4, gamma_inv: f32) -> f32x4 {
    // Użyj istniejącej LUT per-lane (szybko i bezpiecznie na stable)
    let mut arr = [0.0f32; 4];
    let v: [f32; 4] = values.into();
    for i in 0..4 {
        arr[i] = apply_gamma_lut(v[i], gamma_inv);
    }
    f32x4::from_array(arr)
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

    // Tone mapping
    let (tm_r, tm_g, tm_b) = match tonemap_mode {
        1 => (
            reinhard_tonemap_simd(exposed_r),
            reinhard_tonemap_simd(exposed_g),
            reinhard_tonemap_simd(exposed_b),
        ),
        2 => (
            exposed_r.simd_clamp(zero, Simd::splat(1.0)),
            exposed_g.simd_clamp(zero, Simd::splat(1.0)),
            exposed_b.simd_clamp(zero, Simd::splat(1.0)),
        ),
        3 => (
            filmic_tonemap_simd(exposed_r),
            filmic_tonemap_simd(exposed_g),
            filmic_tonemap_simd(exposed_b),
        ),
        4 => (
            hable_tonemap_simd(exposed_r),
            hable_tonemap_simd(exposed_g),
            hable_tonemap_simd(exposed_b),
        ),
        5 => {
            // Local Adaptation - uproszczona wersja bez lokalnego próbkowania
            // Użyj globalnej średniej zamiast lokalnej
            let global_avg = (exposed_r + exposed_g + exposed_b) / Simd::splat(3.0);
            let local_adaptation_tonemap = |x: f32x4, avg: f32x4| {
                let x_safe = x.simd_max(zero);
                let avg_safe = avg.simd_max(Simd::splat(1e-6));
                let local_contrast = x_safe / avg_safe;
                (local_contrast / (Simd::splat(1.0) + local_contrast)).simd_clamp(zero, Simd::splat(1.0))
            };
            (
                local_adaptation_tonemap(exposed_r, global_avg),
                local_adaptation_tonemap(exposed_g, global_avg),
                local_adaptation_tonemap(exposed_b, global_avg),
            )
        },
        _ => (
            aces_tonemap_simd(exposed_r),
            aces_tonemap_simd(exposed_g),
            aces_tonemap_simd(exposed_b),
        ),
    };

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

