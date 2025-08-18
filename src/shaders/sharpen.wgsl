// GPU Sharpen Shader
// Implementuje: Unsharp mask, High-pass filter, Edge enhancement

// Parametry sharpen
struct SharpenParams {
    width: u32,               // Szerokość obrazu
    height: u32,              // Wysokość obrazu
    sharpen_type: u32,        // Typ sharpen: 0=Unsharp, 1=HighPass, 2=Edge
    sharpen_strength: f32,    // Siła sharpen (0.0-2.0)
    sharpen_radius: u32,      // Promień blur dla unsharp mask (1-8)
    sharpen_threshold: f32,   // Próg dla edge detection (0.0-1.0)
    sharpen_amount: f32,      // Ilość sharpen (0.0-1.0)
    _pad0: u32,               // Padding
}

// Wszystkie bindingi w jednej grupie (group 0)
@group(0) @binding(0) var<uniform> params: SharpenParams;

// Bufor wejściowy (piksele jako vec4<f32>)
@group(0) @binding(1) var<storage, read> input_pixels: array<vec4<f32>>;

// Bufor wyjściowy (piksele jako vec4<f32>)
@group(0) @binding(2) var<storage, read_write> output_pixels: array<vec4<f32>>;

// Bezpieczne pobieranie piksela z kontrolą granic
fn get_pixel_safe(x: i32, y: i32) -> vec4<f32> {
    if (x < 0i || y < 0i || x >= i32(params.width) || y >= i32(params.height)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let index = u32(y) * params.width + u32(x);
    return input_pixels[index];
}

// Unsharp mask - odejmuje rozmyty obraz od oryginalnego
fn unsharp_mask(center_x: u32, center_y: u32) -> vec4<f32> {
    let radius = params.sharpen_radius;
    var blurred_sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var sample_count: u32 = 0u;
    
    // Prosty box blur dla unsharp mask
    for (var dy = -i32(radius); dy <= i32(radius); dy++) {
        for (var dx = -i32(radius); dx <= i32(radius); dx++) {
            let sample_x = i32(center_x) + dx;
            let sample_y = i32(center_y) + dy;
            
            if (sample_x >= 0i && sample_y >= 0i && 
                sample_x < i32(params.width) && sample_y < i32(params.height)) {
                let sample_pixel = get_pixel_safe(sample_x, sample_y);
                blurred_sum += sample_pixel;
                sample_count += 1u;
            }
        }
    }
    
    let blurred_pixel = blurred_sum / f32(sample_count);
    let original_pixel = input_pixels[center_y * params.width + center_x];
    
    // Unsharp mask formula: original + amount * (original - blurred)
    let sharpened = original_pixel + params.sharpen_amount * (original_pixel - blurred_pixel);
    
    return clamp(sharpened, vec4<f32>(0.0), vec4<f32>(1.0));
}

// High-pass filter - wzmacnia wysokie częstotliwości
fn high_pass_filter(center_x: u32, center_y: u32) -> vec4<f32> {
    let radius = 1u; // 3x3 kernel
    var low_freq_sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var sample_count: u32 = 0u;
    
    // Suma niskich częstotliwości (średnia z sąsiednich pikseli)
    for (var dy = -1i; dy <= 1i; dy++) {
        for (var dx = -1i; dx <= 1i; dx++) {
            let sample_x = i32(center_x) + dx;
            let sample_y = i32(center_y) + dy;
            
            if (sample_x >= 0i && sample_y >= 0i && 
                sample_x < i32(params.width) && sample_y < i32(params.height)) {
                let sample_pixel = get_pixel_safe(sample_x, sample_y);
                low_freq_sum += sample_pixel;
                sample_count += 1u;
            }
        }
    }
    
    let low_freq_pixel = low_freq_sum / f32(sample_count);
    let original_pixel = input_pixels[center_y * params.width + center_x];
    
    // High-pass: original - low_freq
    let high_freq = original_pixel - low_freq_pixel;
    
    // Zastosuj siłę i dodaj do oryginalnego
    let sharpened = original_pixel + params.sharpen_strength * high_freq;
    
    return clamp(sharpened, vec4<f32>(0.0), vec4<f32>(1.0));
}

// Edge enhancement - wzmacnia krawędzie
fn edge_enhancement(center_x: u32, center_y: u32) -> vec4<f32> {
    let original_pixel = input_pixels[center_y * params.width + center_x];
    
    // Sobel edge detection kernel (uproszczony)
    let sobel_x = vec3<f32>(-1.0, 0.0, 1.0);
    let sobel_y = vec3<f32>(-1.0, 0.0, 1.0);
    
    var grad_x = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var grad_y = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    
    // Oblicz gradient X
    for (var i = -1i; i <= 1i; i++) {
        let sample_x = i32(center_x) + i;
        let sample_y = i32(center_y);
        
        if (sample_x >= 0i && sample_x < i32(params.width) && 
            sample_y >= 0i && sample_y < i32(params.height)) {
            let sample_pixel = get_pixel_safe(sample_x, sample_y);
            grad_x += sample_pixel * sobel_x[i + 1];
        }
    }
    
    // Oblicz gradient Y
    for (var i = -1i; i <= 1i; i++) {
        let sample_x = i32(center_x);
        let sample_y = i32(center_y) + i;
        
        if (sample_x >= 0i && sample_x < i32(params.width) && 
            sample_y >= 0i && sample_y < i32(params.height)) {
            let sample_pixel = get_pixel_safe(sample_x, sample_y);
            grad_y += sample_pixel * sobel_y[i + 1];
        }
    }
    
    // Magnitude gradientu
    let edge_strength = sqrt(grad_x * grad_x + grad_y * grad_y);
    
    // Zastosuj próg
    let edge_mask = select(vec4<f32>(0.0), vec4<f32>(1.0), 
                          edge_strength > params.sharpen_threshold);
    
    // Wzmocnij krawędzie
    let enhanced = original_pixel + params.sharpen_strength * edge_mask;
    
    return clamp(enhanced, vec4<f32>(0.0), vec4<f32>(1.0));
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
    
    // Zastosuj sharpen wg typu
    var sharpened_pixel: vec4<f32>;
    
    if (params.sharpen_type == 1u) {
        // High-pass filter
        sharpened_pixel = high_pass_filter(global_id.x, global_id.y);
    } else if (params.sharpen_type == 2u) {
        // Edge enhancement
        sharpened_pixel = edge_enhancement(global_id.x, global_id.y);
    } else {
        // Unsharp mask (domyślny)
        sharpened_pixel = unsharp_mask(global_id.x, global_id.y);
    }
    
    // Zapisz wynik do bufora wyjściowego
    output_pixels[pixel_index] = sharpened_pixel;
}
