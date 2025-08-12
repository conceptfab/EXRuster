use slint::Rgba8Pixel;
use std::cell::RefCell;

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
pub fn process_pixel(r: f32, g: f32, b: f32, a: f32, exposure: f32, gamma: f32) -> Rgba8Pixel {
    let exposure_multiplier = 2.0_f32.powf(exposure);
    
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
    
    // Korekcja wyjściowa: preferuj prawdziwą krzywą sRGB (OETF) dla gamma ~2.2/2.4; w innym wypadku użyj niestandardowej gammy
    let use_srgb = (gamma - 2.2).abs() < 0.2 || (gamma - 2.4).abs() < 0.2;
    let (corrected_r, corrected_g, corrected_b) = if use_srgb {
        (
            srgb_oetf(tone_mapped_r),
            srgb_oetf(tone_mapped_g),
            srgb_oetf(tone_mapped_b),
        )
    } else {
        let gamma_inv = 1.0 / gamma.max(1e-4);
        (
            apply_gamma_lut(tone_mapped_r, gamma_inv),
            apply_gamma_lut(tone_mapped_g, gamma_inv),
            apply_gamma_lut(tone_mapped_b, gamma_inv),
        )
    };
    
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

// (usunięto) apply_gamma_fast – zastąpione przez szybsze `apply_gamma_lut`

// usunięto nieużywaną funkcję read_exr_to_slint_image

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
