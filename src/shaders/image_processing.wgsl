// Compute shader do przetwarzania obrazów EXR na GPU
// Implementuje: ekspozycja → tone mapping → gamma correction

// Bind Group 0: Uniformy (parametry przetwarzania)
struct Params {
    exposure: f32,           // Korekcja ekspozycji
    gamma: f32,              // Wartość gamma
    tonemap_mode: u32,       // Tryb tone mapping: 0=ACES, 1=Reinhard, 2=Linear, 3=Filmic, 4=Hable, 5=LocalAdaptation
    width: u32,              // Szerokość obrazu
    height: u32,             // Wysokość obrazu
    color_matrix: mat3x3<f32>, // Nowe pole: macierz transformacji kolorów
    has_color_matrix: u32,    // Nowe pole: flaga użycia macierzy (0 lub 1)
    local_adaptation_radius: u32, // Promień dla local adaptation (domyślnie 16)
    _pad0: u32,              // Padding dla wyrównania
}

// Wszystkie bindingi w jednej grupie (group 0)
// Bufor wejściowy (piksele HDR jako vec4<f32>)
@group(0) @binding(1) var<storage, read> input_pixels: array<vec4<f32>>;

// Bufor wyjściowy (piksele jako u32 - NAPRAWIONE: bardziej kompatybilne)
@group(0) @binding(2) var<storage, read_write> output_pixels: array<u32>;

// Uniformy
@group(0) @binding(0) var<uniform> params: Params;

// Filmic tone mapping - bardziej zaawansowana krzywa filmowa
fn filmic_tonemap(x: f32) -> f32 {
    let x_safe = max(x, 0.0);
    let a = 0.15;  // Black point
    let b = 0.50;  // Toe
    let c = 0.10;  // Shoulder
    let d = 0.20;  // White point
    
    let numerator = (x_safe * (a * x_safe + c * b) + d * x_safe);
    let denominator = (x_safe * (a * x_safe + b) + d * c);
    
    return clamp(numerator / (denominator + 1e-9), 0.0, 1.0);
}

// Hable tone mapping - Uncharted 2 style
fn hable_tonemap(x: f32) -> f32 {
    let x_safe = max(x, 0.0);
    let A = 0.15;
    let B = 0.50;
    let C = 0.10;
    let D = 0.20;
    let E = 0.02;
    let F = 0.30;
    let W = 11.2;
    
    let numerator = ((x_safe * (A * x_safe + C * B) + D * E) * (x_safe * (A * x_safe + B) + D * C));
    let denominator = ((x_safe * (A * x_safe + B) + D * C) * (x_safe * (A * x_safe + C * B) + D * E));
    
    let white_scale = 1.0 / (((W * (A * W + C * B) + D * E) * (W * (A * W + B) + D * C)) / ((W * (A * W + B) + D * C) * (W * (A * W + C * B) + D * E)));
    
    return clamp(numerator / (denominator + 1e-9) * white_scale, 0.0, 1.0);
}

// Local adaptation tone mapping - uwzględnia lokalny kontrast
fn local_adaptation_tonemap(x: f32, local_avg: f32) -> f32 {
    let x_safe = max(x, 0.0);
    let local_avg_safe = max(local_avg, 1e-6);
    
    // Oblicz lokalny kontrast
    let local_contrast = x_safe / local_avg_safe;
    
    // Zastosuj adaptive tone mapping
    let adapted_value = local_contrast / (1.0 + local_contrast);
    
    return clamp(adapted_value, 0.0, 1.0);
}

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
    if (x >= 1e10) { // Treat large numbers as infinity
        return 1.0;
    }
    if (x != x || x < 0.0) {
        return 0.0; // NaN or negative -> 0
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
    tonemap_mode: u32,
    current_x: u32,
    current_y: u32
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
    } else if (tonemap_mode == 3u) {
        // Filmic
        tm_r = filmic_tonemap(exposed_r);
        tm_g = filmic_tonemap(exposed_g);
        tm_b = filmic_tonemap(exposed_b);
    } else if (tonemap_mode == 4u) {
        // Hable
        tm_r = hable_tonemap(exposed_r);
        tm_g = hable_tonemap(exposed_g);
        tm_b = hable_tonemap(exposed_b);
         } else if (tonemap_mode == 5u) {
         // Local Adaptation - oblicz średnią lokalną w promieniu
         let radius = params.local_adaptation_radius;
         var local_sum_r: f32 = 0.0;
         var local_sum_g: f32 = 0.0;
         var local_sum_b: f32 = 0.0;
         var sample_count: u32 = 0u;
         
         // Próbkowanie w promieniu (uproszczone - tylko 9 punktów)
         for (var dy = -1i; dy <= 1i; dy++) {
             for (var dx = -1i; dx <= 1i; dx++) {
                 let sample_x = i32(current_x) + dx;
                 let sample_y = i32(current_y) + dy;
                 
                 if (sample_x >= 0i && sample_x < i32(params.width) && 
                     sample_y >= 0i && sample_y < i32(params.height)) {
                     let sample_index = u32(sample_y) * params.width + u32(sample_x);
                     let sample_pixel = input_pixels[sample_index];
                     local_sum_r += sample_pixel.r;
                     local_sum_g += sample_pixel.g;
                     local_sum_b += sample_pixel.b;
                     sample_count += 1u;
                 }
             }
         }
         
         let local_avg_r = local_sum_r / f32(sample_count);
         let local_avg_g = local_sum_g / f32(sample_count);
         let local_avg_b = local_sum_b / f32(sample_count);

         tm_r = local_adaptation_tonemap(exposed_r, local_avg_r);
         tm_g = local_adaptation_tonemap(exposed_g, local_avg_g);
         tm_b = local_adaptation_tonemap(exposed_b, local_avg_b);
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

// KRYTYCZNA OPTYMALIZACJA RTX 4070: 32x8 workgroup dla lepszego memory coalescing
@compute @workgroup_size(32, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Sprawdź czy piksel jest w granicach obrazu
    if (global_id.x >= params.width || global_id.y >= params.height) {
        return;
    }

    // Przelicz ID na indeks piksela
    let pixel_index = global_id.y * params.width + global_id.x;
    
    // Wczytaj piksel HDR z bufora wejściowego
    let input_pixel = input_pixels[pixel_index];
    var color = input_pixel.rgb;

    // Zastosuj macierz kolorów jeśli dostępna
    if (params.has_color_matrix != 0u) {
        color = params.color_matrix * color;
    }
    
    // Przetwórz piksel przez pipeline: ekspozycja → tone mapping → gamma
    let processed_color = tone_map_and_gamma(
        color.r,
        color.g,
        color.b,
        params.exposure,
        params.gamma,
        params.tonemap_mode,
        global_id.x,
        global_id.y
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
