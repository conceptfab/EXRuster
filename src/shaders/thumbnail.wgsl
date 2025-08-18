// GPU Thumbnail Generation Shader
// Implementuje: downsampling + ekspozycja + tone mapping + gamma correction

// Bind Group 0: Parametry thumbnail generation
struct ThumbnailParams {
    src_width: u32,           // Szerokość źródłowego obrazu
    src_height: u32,          // Wysokość źródłowego obrazu
    dst_width: u32,           // Szerokość docelowej miniaturki
    dst_height: u32,          // Wysokość docelowej miniaturki
    exposure: f32,            // Korekcja ekspozycji
    gamma: f32,               // Wartość gamma
    tonemap_mode: u32,        // Tryb tone mapping: 0=ACES, 1=Reinhard, 2=Linear
    scale_x: f32,             // Skala X (src_width / dst_width)
    scale_y: f32,             // Skala Y (src_height / dst_height)
    _pad0: vec3<u32>,         // Padding
    color_matrix: mat3x3<f32>, // Macierz transformacji kolorów
    has_color_matrix: u32,    // Flaga użycia macierzy (0 lub 1)
    _pad1: vec3<u32>,         // Padding
}

// Wszystkie bindingi w jednej grupie (group 0)
@group(0) @binding(0) var<uniform> params: ThumbnailParams;

// Bufor wejściowy (piksele HDR jako vec4<f32>)
@group(0) @binding(1) var<storage, read> input_pixels: array<vec4<f32>>;

// Bufor wyjściowy (piksele jako u32 - packed RGBA)
@group(0) @binding(2) var<storage, read_write> output_pixels: array<u32>;

// ACES tone mapping
fn aces_tonemap(x: f32) -> f32 {
    let x_safe = max(x, 0.0);
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let numerator = x_safe * (a * x_safe + b);
    let denominator = x_safe * (c * x_safe + d) + e;
    return clamp(numerator / (denominator + 1e-9), 0.0, 1.0);
}

// Reinhard tone mapping
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

// Prawdziwa krzywa sRGB (OETF)
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

// Bilinear sampling z input texture
fn sample_bilinear(src_x: f32, src_y: f32) -> vec4<f32> {
    // Współrzędne pikseli dla bilinear interpolation
    let x0 = u32(floor(src_x));
    let y0 = u32(floor(src_y));
    let x1 = min(x0 + 1u, params.src_width - 1u);
    let y1 = min(y0 + 1u, params.src_height - 1u);
    
    // Wagi interpolacji
    let fx = src_x - floor(src_x);
    let fy = src_y - floor(src_y);
    
    // Pobierz 4 sąsiednie piksele
    let p00 = input_pixels[y0 * params.src_width + x0];
    let p10 = input_pixels[y0 * params.src_width + x1];
    let p01 = input_pixels[y1 * params.src_width + x0];
    let p11 = input_pixels[y1 * params.src_width + x1];
    
    // Bilinear interpolation
    let top = mix(p00, p10, fx);
    let bottom = mix(p01, p11, fx);
    return mix(top, bottom, fy);
}

// Pipeline: downsampling + ekspozycja + tone-map + gamma
fn process_thumbnail_pixel(
    r: f32,
    g: f32,
    b: f32,
    a: f32
) -> vec4<f32> {
    var color = vec3<f32>(r, g, b);
    
    // Zastosuj macierz kolorów jeśli dostępna
    if (params.has_color_matrix != 0u) {
        color = params.color_matrix * color;
    }
    
    let exposure_multiplier = pow(2.0, params.exposure);
    
    // Sprawdzenie NaN/Inf i clamp do sensownych wartości
    let safe_r = select(0.0, max(color.r, 0.0), color.r == color.r && color.r != color.r * 0.5);
    let safe_g = select(0.0, max(color.g, 0.0), color.g == color.g && color.g != color.g * 0.5);
    let safe_b = select(0.0, max(color.b, 0.0), color.b == color.b && color.b != color.b * 0.5);
    let safe_a = select(1.0, clamp(a, 0.0, 1.0), a == a);
    
    // Zastosowanie ekspozycji
    let exposed_r = safe_r * exposure_multiplier;
    let exposed_g = safe_g * exposure_multiplier;
    let exposed_b = safe_b * exposure_multiplier;
    
    // Tone mapping wg trybu
    var tm_r: f32;
    var tm_g: f32;
    var tm_b: f32;
    
    if (params.tonemap_mode == 1u) {
        // Reinhard
        tm_r = reinhard_tonemap(exposed_r);
        tm_g = reinhard_tonemap(exposed_g);
        tm_b = reinhard_tonemap(exposed_b);
    } else if (params.tonemap_mode == 2u) {
        // Linear: brak tone-map, tylko clamp do [0,1] po ekspozycji
        tm_r = clamp(exposed_r, 0.0, 1.0);
        tm_g = clamp(exposed_g, 0.0, 1.0);
        tm_b = clamp(exposed_b, 0.0, 1.0);
    } else {
        // ACES (domyślny)
        tm_r = aces_tonemap(exposed_r);
        tm_g = aces_tonemap(exposed_g);
        tm_b = aces_tonemap(exposed_b);
    }
    
    // Korekcja wyjściowa: preferuj prawdziwą krzywą sRGB dla gamma ~2.2/2.4
    let use_srgb = (abs(params.gamma - 2.2) < 0.2) || (abs(params.gamma - 2.4) < 0.2);
    
    var final_color: vec3<f32>;
    if (use_srgb) {
        final_color = vec3<f32>(
            srgb_oetf(tm_r),
            srgb_oetf(tm_g),
            srgb_oetf(tm_b)
        );
    } else {
        let gamma_inv = 1.0 / max(params.gamma, 1e-4);
        final_color = vec3<f32>(
            apply_gamma(tm_r, gamma_inv),
            apply_gamma(tm_g, gamma_inv),
            apply_gamma(tm_b, gamma_inv)
        );
    }
    
    return vec4<f32>(final_color, safe_a);
}

// Główna funkcja compute shadera
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Sprawdź czy piksel jest w granicach docelowego obrazu
    if (global_id.x >= params.dst_width || global_id.y >= params.dst_height) {
        return;
    }
    
    // Przelicz współrzędne docelowe na źródłowe (z subpixel precision)
    let src_x = (f32(global_id.x) + 0.5) * params.scale_x - 0.5;
    let src_y = (f32(global_id.y) + 0.5) * params.scale_y - 0.5;
    
    // Clamp do granic źródłowego obrazu
    let clamped_x = clamp(src_x, 0.0, f32(params.src_width - 1u));
    let clamped_y = clamp(src_y, 0.0, f32(params.src_height - 1u));
    
    // Pobierz piksel z bilinear sampling
    let input_pixel = sample_bilinear(clamped_x, clamped_y);
    
    // Przetwórz piksel przez pipeline
    let processed_pixel = process_thumbnail_pixel(
        input_pixel.r,
        input_pixel.g,
        input_pixel.b,
        input_pixel.a
    );
    
    // Konwersja do u32 (4 bajty RGBA packed)
    let r_u8 = u32(clamp(processed_pixel.r * 255.0, 0.0, 255.0));
    let g_u8 = u32(clamp(processed_pixel.g * 255.0, 0.0, 255.0));
    let b_u8 = u32(clamp(processed_pixel.b * 255.0, 0.0, 255.0));
    let a_u8 = u32(clamp(processed_pixel.a * 255.0, 0.0, 255.0));
    
    // Pakuj 4 bajty do u32: RGBA
    let packed_color = (r_u8) | (g_u8 << 8u) | (b_u8 << 16u) | (a_u8 << 24u);
    
    // Zapisz wynik do bufora wyjściowego
    let output_index = global_id.y * params.dst_width + global_id.x;
    output_pixels[output_index] = packed_color;
}