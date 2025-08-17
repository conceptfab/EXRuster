// Compute shader do przetwarzania obrazów EXR na GPU
// Implementuje: ekspozycja → tone mapping → gamma correction

// Bind Group 0: Uniformy (parametry przetwarzania)
struct Params {
    exposure: f32,           // Korekcja ekspozycji
    gamma: f32,              // Wartość gamma
    tonemap_mode: u32,       // Tryb tone mapping: 0=ACES, 1=Reinhard, 2=Linear
    width: u32,              // Szerokość obrazu
    height: u32,             // Wysokość obrazu
    // Opcjonalna macierz transformacji kolorów (może być dodana później)
    // color_matrix: mat3x3<f32>,
}

// Wszystkie bindingi w jednej grupie (group 0)
// Bufor wejściowy (piksele HDR jako vec4<f32>)
@group(0) @binding(1) var<storage, read> input_pixels: array<vec4<f32>>;

// Bufor wyjściowy (piksele jako u32 - NAPRAWIONE: bardziej kompatybilne)
@group(0) @binding(2) var<storage, write> output_pixels: array<u32>;

// Uniformy
@group(0) @binding(0) var<uniform> params: Params;

// ACES tone mapping - znacznie lepszy od Reinhard (bezpieczniejsza wersja)
fn aces_tonemap(x: f32) -> f32 {
    let x_safe = max(x, 0.0); // Zabezpiecza przed wartościami ujemnymi
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let numerator = x_safe * (a * x_safe + b);
    let denominator = x_safe * (c * x_safe + d) + e;
    // Dodaj epsilon, aby uniknąć dzielenia przez zero
    return clamp(numerator / (denominator + 1e-9), 0.0, 1.0);
}

// Reinhard tone mapping: x / (1 + x) (bezpieczniejsza wersja)
fn reinhard_tonemap(x: f32) -> f32 {
    // Sprawdź czy wejście jest poprawne
    if (x != x || x < 0.0) {
        return 0.0; // NaN lub ujemne -> 0
    }
    
    let result = x / (1.0 + x);
    return clamp(result, 0.0, 1.0);
}

// Prawdziwa krzywa sRGB (OETF), zastosowana do wartości w [0,1]
fn srgb_oetf(x: f32) -> f32 {
    let x_clamped = clamp(x, 0.0, 1.0);
    let linear_part = 12.92 * x_clamped;
    let nonlinear_part = 1.055 * pow(x_clamped, 1.0 / 2.4) - 0.055;
    return select(nonlinear_part, linear_part, x_clamped <= 0.0031308);
}

// Niestandardowa korekcja gamma
fn apply_gamma(x: f32, gamma_inv: f32) -> f32 {
    return pow(clamp(x, 0.0, 1.0), gamma_inv);
}

// Wspólny pipeline: ekspozycja → tone-map (wg trybu) → gamma/sRGB
// Zwraca wartości w [0, 1] po korekcji gamma
fn tone_map_and_gamma(
    r: f32,
    g: f32,
    b: f32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32
) -> vec3<f32> {
    let exposure_multiplier = pow(2.0, exposure);

    // Sprawdzenie NaN/Inf i clamp do sensownych wartości (bezpieczniejsze)
    let safe_r = select(0.0, max(r, 0.0), r == r && r != r * 0.5); // sprawdź czy nie NaN
    let safe_g = select(0.0, max(g, 0.0), g == g && g != g * 0.5);
    let safe_b = select(0.0, max(b, 0.0), b == b && b != b * 0.5);

    // Zastosowanie ekspozycji
    let exposed_r = safe_r * exposure_multiplier;
    let exposed_g = safe_g * exposure_multiplier;
    let exposed_b = safe_b * exposure_multiplier;

    // Tone mapping wg trybu
    var tm_r: f32;
    var tm_g: f32;
    var tm_b: f32;

    if (tonemap_mode == 1u) {
        // Reinhard
        tm_r = reinhard_tonemap(exposed_r);
        tm_g = reinhard_tonemap(exposed_g);
        tm_b = reinhard_tonemap(exposed_b);
    } else if (tonemap_mode == 2u) {
        // Linear: brak tone-map, tylko clamp do [0,1] po ekspozycji
        tm_r = select(0.0, select(1.0, exposed_r, exposed_r < 1.0), exposed_r > 0.0);
        tm_g = select(0.0, select(1.0, exposed_g, exposed_g < 1.0), exposed_g > 0.0);
        tm_b = select(0.0, select(1.0, exposed_b, exposed_b < 1.0), exposed_b > 0.0);
    } else {
        // ACES (domyślny)
        tm_r = aces_tonemap(exposed_r);
        tm_g = aces_tonemap(exposed_g);
        tm_b = aces_tonemap(exposed_b);
    }

    // Korekcja wyjściowa: preferuj prawdziwą krzywą sRGB (OETF) dla gamma ~2.2/2.4
    let use_srgb = (abs(gamma - 2.2) < 0.2) || (abs(gamma - 2.4) < 0.2);
    
    if (use_srgb) {
        return vec3<f32>(
            srgb_oetf(tm_r),
            srgb_oetf(tm_g),
            srgb_oetf(tm_b)
        );
    } else {
        let gamma_inv = 1.0 / max(gamma, 1e-4);
        return vec3<f32>(
            apply_gamma(tm_r, gamma_inv),
            apply_gamma(tm_g, gamma_inv),
            apply_gamma(tm_b, gamma_inv)
        );
    }
}

// Główna funkcja compute shadera
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Sprawdź czy piksel jest w granicach obrazu
    if (global_id.x >= params.width || global_id.y >= params.height) {
        return;
    }

    // Przelicz ID na indeks piksela
    let pixel_index = global_id.y * params.width + global_id.x;
    
    // Wczytaj piksel HDR z bufora wejściowego
    let input_pixel = input_pixels[pixel_index];
    
    // Przetwórz piksel przez pipeline: ekspozycja → tone mapping → gamma
    let processed_color = tone_map_and_gamma(
        input_pixel.r,
        input_pixel.g,
        input_pixel.b,
        params.exposure,
        params.gamma,
        params.tonemap_mode
    );
    
    // NAPRAWIONE: Konwersja do u32 (4 bajty RGBA packed)
    let r_u8 = u32(clamp(processed_color.r * 255.0, 0.0, 255.0));
    let g_u8 = u32(clamp(processed_color.g * 255.0, 0.0, 255.0));
    let b_u8 = u32(clamp(processed_color.b * 255.0, 0.0, 255.0));
    let a_u8 = u32(clamp(input_pixel.a * 255.0, 0.0, 255.0));
    
    // Pakuj 4 bajty do u32: RGBA
    let packed_color = (r_u8) | (g_u8 << 8u) | (b_u8 << 16u) | (a_u8 << 24u);
    
    // Zapisz wynik do bufora wyjściowego
    output_pixels[pixel_index] = packed_color;
}
