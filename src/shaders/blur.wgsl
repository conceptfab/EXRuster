// GPU Gaussian Blur Shader
// Implementuje: Gaussian blur, Box blur, Motion blur

// Parametry blur
struct BlurParams {
    width: u32,               // Szerokość obrazu
    height: u32,              // Wysokość obrazu
    blur_type: u32,           // Typ blur: 0=Gaussian, 1=Box, 2=Motion
    blur_radius: u32,         // Promień blur (1-32)
    blur_strength: f32,       // Siła blur (0.0-1.0)
    blur_direction_x: f32,    // Kierunek blur X (dla motion blur)
    blur_direction_y: f32,    // Kierunek blur Y (dla motion blur)
    _pad0: u32,               // Padding
}

// Wszystkie bindingi w jednej grupie (group 0)
@group(0) @binding(0) var<uniform> params: BlurParams;

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

// Gaussian kernel weights (uproszczone)
fn gaussian_weight(distance: f32, sigma: f32) -> f32 {
    let sigma_sq = sigma * sigma;
    return exp(-(distance * distance) / (2.0 * sigma_sq));
}

// Gaussian blur
fn gaussian_blur(center_x: u32, center_y: u32) -> vec4<f32> {
    let radius = params.blur_radius;
    let sigma = f32(radius) * 0.5;
    var total_weight: f32 = 0.0;
    var weighted_sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    
    for (var dy = -i32(radius); dy <= i32(radius); dy++) {
        for (var dx = -i32(radius); dx <= i32(radius); dx++) {
            let sample_x = i32(center_x) + dx;
            let sample_y = i32(center_y) + dy;
            let distance = sqrt(f32(dx * dx + dy * dy));
            let weight = gaussian_weight(distance, sigma);
            
            let sample_pixel = get_pixel_safe(sample_x, sample_y);
            weighted_sum += sample_pixel * weight;
            total_weight += weight;
        }
    }
    
    return weighted_sum / total_weight;
}

// Box blur (prostszy, szybszy)
fn box_blur(center_x: u32, center_y: u32) -> vec4<f32> {
    let radius = params.blur_radius;
    var total_sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var sample_count: u32 = 0u;
    
    for (var dy = -i32(radius); dy <= i32(radius); dy++) {
        for (var dx = -i32(radius); dx <= i32(radius); dx++) {
            let sample_x = i32(center_x) + dx;
            let sample_y = i32(center_y) + dy;
            
            if (sample_x >= 0i && sample_y >= 0i && 
                sample_x < i32(params.width) && sample_y < i32(params.height)) {
                let sample_pixel = get_pixel_safe(sample_x, sample_y);
                total_sum += sample_pixel;
                sample_count += 1u;
            }
        }
    }
    
    return total_sum / f32(sample_count);
}

// Motion blur (w kierunku)
fn motion_blur(center_x: u32, center_y: u32) -> vec4<f32> {
    let radius = params.blur_radius;
    let dir_x = params.blur_direction_x;
    let dir_y = params.blur_direction_y;
    var total_sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var sample_count: u32 = 0u;
    
    for (var i = -i32(radius); i <= i32(radius); i++) {
        let sample_x = i32(center_x) + i32(f32(i) * dir_x);
        let sample_y = i32(center_y) + i32(f32(i) * dir_y);
        
        if (sample_x >= 0i && sample_y >= 0i && 
            sample_x < i32(params.width) && sample_y < i32(params.height)) {
            let sample_pixel = get_pixel_safe(sample_x, sample_y);
            total_sum += sample_pixel;
            sample_count += 1u;
        }
    }
    
    return total_sum / f32(sample_count);
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
    
    // Zastosuj blur wg typu
    var blurred_pixel: vec4<f32>;
    
    if (params.blur_type == 1u) {
        // Box blur
        blurred_pixel = box_blur(global_id.x, global_id.y);
    } else if (params.blur_type == 2u) {
        // Motion blur
        blurred_pixel = motion_blur(global_id.x, global_id.y);
    } else {
        // Gaussian blur (domyślny)
        blurred_pixel = gaussian_blur(global_id.x, global_id.y);
    }
    
    // Zastosuj siłę blur (mieszanie z oryginalnym pikselem)
    let original_pixel = input_pixels[pixel_index];
    let strength = params.blur_strength;
    let final_pixel = mix(original_pixel, blurred_pixel, strength);
    
    // Zapisz wynik do bufora wyjściowego
    output_pixels[pixel_index] = final_pixel;
}
