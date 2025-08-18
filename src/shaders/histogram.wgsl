// Histogram compute shader dla EXRuster
// Oblicza histogram dla obrazów RGBA w czasie rzeczywistym

struct HistogramParams {
    width: u32,
    height: u32,
    bin_count: u32,
    min_value: f32,
    max_value: f32,
    _pad: u32,
}

struct HistogramBins {
    red_bins: array<u32, 256>,
    green_bins: array<u32, 256>,
    blue_bins: array<u32, 256>,
    luminance_bins: array<u32, 256>,
}

@group(0) @binding(0) var<uniform> params: HistogramParams;
@group(0) @binding(1) var<storage, read> input_pixels: array<f32>;
@group(0) @binding(2) var<storage, read_write> histogram_bins: HistogramBins;

@compute @workgroup_size(256)
fn compute_histogram(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel_index = global_id.x;
    let total_pixels = params.width * params.height;
    
    if (pixel_index >= total_pixels) {
        return;
    }
    
    let base_index = pixel_index * 4u;
    let r = input_pixels[base_index];
    let g = input_pixels[base_index + 1u];
    let b = input_pixels[base_index + 2u];
    let a = input_pixels[base_index + 3u];
    
    // Oblicz luminance (ITU-R BT.709)
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    
    // Mapuj wartości do binów (0-255)
    let bin_r = u32(clamp((r - params.min_value) / (params.max_value - params.min_value) * f32(params.bin_count - 1u), 0.0, f32(params.bin_count - 1u)));
    let bin_g = u32(clamp((g - params.min_value) / (params.max_value - params.min_value) * f32(params.bin_count - 1u), 0.0, f32(params.bin_count - 1u)));
    let bin_b = u32(clamp((b - params.min_value) / (params.max_value - params.min_value) * f32(params.bin_count - 1u), 0.0, f32(params.bin_count - 1u)));
    let bin_l = u32(clamp((luminance - params.min_value) / (params.max_value - params.min_value) * f32(params.bin_count - 1u), 0.0, f32(params.bin_count - 1u)));
    
    // Atomowo inkrementuj biny
    atomicAdd(&histogram_bins.red_bins[bin_r], 1u);
    atomicAdd(&histogram_bins.green_bins[bin_g], 1u);
    atomicAdd(&histogram_bins.blue_bins[bin_b], 1u);
    atomicAdd(&histogram_bins.luminance_bins[bin_l], 1u);
}
