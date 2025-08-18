// GPU Histogram Computation Shader
// Implementuje: RGB histogram, Luminance histogram, Cumulative distribution

// Parametry histogram
struct HistogramParams {
    width: u32,               // Szerokość obrazu
    height: u32,              // Wysokość obrazu
    histogram_type: u32,      // Typ histogramu: 0=RGB, 1=Luminance, 2=All
    num_bins: u32,            // Liczba binów histogramu (256 dla 8-bit)
    min_value: f32,           // Minimalna wartość (0.0 dla HDR)
    max_value: f32,           // Maksymalna wartość (1.0 dla LDR, >1.0 dla HDR)
    _pad0: u32,               // Padding
}

// Wszystkie bindingi w jednej grupie (group 0)
@group(0) @binding(0) var<uniform> params: HistogramParams;

// Bufor wejściowy (piksele jako vec4<f32>)
@group(0) @binding(1) var<storage, read> input_pixels: array<vec4<f32>>;

// Bufor wyjściowy (histogram jako array<u32>)
@group(0) @binding(2) var<storage, read_write> histogram_output: array<u32>;

// Atomic counter dla każdego binu
@group(0) @binding(3) var<storage, read_write> histogram_bins: array<atomic<u32>>;

// Oblicz luminancję z RGB
fn calculate_luminance(r: f32, g: f32, b: f32) -> f32 {
    return 0.299 * r + 0.587 * g + 0.114 * b;
}

// Mapuj wartość na bin histogramu
fn value_to_bin(value: f32) -> u32 {
    let normalized = (value - params.min_value) / (params.max_value - params.min_value);
    let clamped = clamp(normalized, 0.0, 1.0);
    let bin_index = u32(clamped * f32(params.num_bins - 1u));
    return min(bin_index, params.num_bins - 1u);
}

// RGB histogram - osobne biny dla każdego kanału
fn rgb_histogram(pixel: vec4<f32>) {
    let r_bin = value_to_bin(pixel.r);
    let g_bin = value_to_bin(pixel.g);
    let b_bin = value_to_bin(pixel.b);
    
    // Użyj atomic operations dla thread-safe increment
    atomicAdd(&histogram_bins[r_bin], 1u);
    atomicAdd(&histogram_bins[g_bin + params.num_bins], 1u);
    atomicAdd(&histogram_bins[b_bin + 2u * params.num_bins], 1u);
}

// Luminance histogram
fn luminance_histogram(pixel: vec4<f32>) {
    let lum = calculate_luminance(pixel.r, pixel.g, pixel.b);
    let lum_bin = value_to_bin(lum);
    
    atomicAdd(&histogram_bins[lum_bin], 1u);
}

// Wszystkie histogramy jednocześnie
fn all_histograms(pixel: vec4<f32>) {
    let r_bin = value_to_bin(pixel.r);
    let g_bin = value_to_bin(pixel.g);
    let b_bin = value_to_bin(pixel.b);
    let lum_bin = value_to_bin(calculate_luminance(pixel.r, pixel.g, pixel.b));
    
    // RGB + Luminance w jednym buforze
    atomicAdd(&histogram_bins[r_bin], 1u);
    atomicAdd(&histogram_bins[g_bin + params.num_bins], 1u);
    atomicAdd(&histogram_bins[b_bin + 2u * params.num_bins], 1u);
    atomicAdd(&histogram_bins[lum_bin + 3u * params.num_bins], 1u);
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
    let pixel = input_pixels[pixel_index];
    
    // Zastosuj histogram wg typu
    if (params.histogram_type == 1u) {
        // Luminance histogram
        luminance_histogram(pixel);
    } else if (params.histogram_type == 2u) {
        // Wszystkie histogramy
        all_histograms(pixel);
    } else {
        // RGB histogram (domyślny)
        rgb_histogram(pixel);
    }
    
    // Dodatkowo zapisz informacje o pikselu do bufora wyjściowego
    // (może być używane do debugowania lub dalszego przetwarzania)
    let output_index = pixel_index * 4u; // 4 wartości na piksel
    if (output_index + 3u < arrayLength(&histogram_output)) {
        histogram_output[output_index] = u32(pixel.r * 255.0);
        histogram_output[output_index + 1u] = u32(pixel.g * 255.0);
        histogram_output[output_index + 2u] = u32(pixel.b * 255.0);
        histogram_output[output_index + 3u] = u32(pixel.a * 255.0);
    }
}
