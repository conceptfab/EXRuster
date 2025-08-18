// GPU MIP Generation Shader
// Implementuje: 2x2 average downsampling dla generowania poziomów MIP

// Parametry MIP generation
struct MipParams {
    src_width: u32,           // Szerokość źródłowego poziomu MIP
    src_height: u32,          // Wysokość źródłowego poziomu MIP
    dst_width: u32,           // Szerokość docelowego poziomu MIP
    dst_height: u32,          // Wysokość docelowego poziomu MIP
    mip_level: u32,           // Aktualny poziom MIP (0 = oryginalny)
    filter_mode: u32,         // Tryb filtrowania: 0=Average, 1=Max, 2=Min
    preserve_alpha: u32,      // Zachowaj kanał alpha (0 lub 1)
    _pad0: u32,               // Padding dla wyrównania 16-byte
}

// Wszystkie bindingi w jednej grupie (group 0)
@group(0) @binding(0) var<uniform> params: MipParams;

// Bufor wejściowy (piksele HDR jako vec4<f32>)
@group(0) @binding(1) var<storage, read> input_pixels: array<vec4<f32>>;

// Bufor wyjściowy (piksele HDR jako vec4<f32>)
@group(0) @binding(2) var<storage, read_write> output_pixels: array<vec4<f32>>;

// Bezpieczne pobieranie piksela z kontrolą granic
fn get_pixel_safe(x: u32, y: u32) -> vec4<f32> {
    if (x >= params.src_width || y >= params.src_height) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let index = y * params.src_width + x;
    return input_pixels[index];
}

// 2x2 average downsampling
fn downsample_2x2_average(src_x: u32, src_y: u32) -> vec4<f32> {
    // Pobierz 4 piksele z bloku 2x2
    let p00 = get_pixel_safe(src_x, src_y);
    let p10 = get_pixel_safe(src_x + 1u, src_y);
    let p01 = get_pixel_safe(src_x, src_y + 1u);
    let p11 = get_pixel_safe(src_x + 1u, src_y + 1u);
    
    // Oblicz średnią arytmetyczną
    let avg_color = (p00 + p10 + p01 + p11) * 0.25;
    
    // Obsłuż kanał alpha
    var final_alpha: f32;
    if (params.preserve_alpha != 0u) {
        // Zachowaj maksymalną wartość alpha z bloku 2x2
        final_alpha = max(max(p00.a, p10.a), max(p01.a, p11.a));
    } else {
        // Średnia z alpha
        final_alpha = avg_color.a;
    }
    
    return vec4<f32>(avg_color.rgb, final_alpha);
}

// 2x2 maximum downsampling (zachowuje najjaśniejszy piksel)
fn downsample_2x2_max(src_x: u32, src_y: u32) -> vec4<f32> {
    let p00 = get_pixel_safe(src_x, src_y);
    let p10 = get_pixel_safe(src_x + 1u, src_y);
    let p01 = get_pixel_safe(src_x, src_y + 1u);
    let p11 = get_pixel_safe(src_x + 1u, src_y + 1u);
    
    // Oblicz luminancję dla każdego piksela (używamy prostej formuły)
    let lum00 = p00.r * 0.299 + p00.g * 0.587 + p00.b * 0.114;
    let lum10 = p10.r * 0.299 + p10.g * 0.587 + p10.b * 0.114;
    let lum01 = p01.r * 0.299 + p01.g * 0.587 + p01.b * 0.114;
    let lum11 = p11.r * 0.299 + p11.g * 0.587 + p11.b * 0.114;
    
    // Znajdź piksel z maksymalną luminancją
    var max_pixel = p00;
    var max_lum = lum00;
    
    if (lum10 > max_lum) {
        max_pixel = p10;
        max_lum = lum10;
    }
    if (lum01 > max_lum) {
        max_pixel = p01;
        max_lum = lum01;
    }
    if (lum11 > max_lum) {
        max_pixel = p11;
        max_lum = lum11;
    }
    
    return max_pixel;
}

// 2x2 minimum downsampling (zachowuje najciemniejszy piksel)
fn downsample_2x2_min(src_x: u32, src_y: u32) -> vec4<f32> {
    let p00 = get_pixel_safe(src_x, src_y);
    let p10 = get_pixel_safe(src_x + 1u, src_y);
    let p01 = get_pixel_safe(src_x, src_y + 1u);
    let p11 = get_pixel_safe(src_x + 1u, src_y + 1u);
    
    // Oblicz luminancję dla każdego piksela
    let lum00 = p00.r * 0.299 + p00.g * 0.587 + p00.b * 0.114;
    let lum10 = p10.r * 0.299 + p10.g * 0.587 + p10.b * 0.114;
    let lum01 = p01.r * 0.299 + p01.g * 0.587 + p01.b * 0.114;
    let lum11 = p11.r * 0.299 + p11.g * 0.587 + p11.b * 0.114;
    
    // Znajdź piksel z minimalną luminancją
    var min_pixel = p00;
    var min_lum = lum00;
    
    if (lum10 < min_lum) {
        min_pixel = p10;
        min_lum = lum10;
    }
    if (lum01 < min_lum) {
        min_pixel = p01;
        min_lum = lum01;
    }
    if (lum11 < min_lum) {
        min_pixel = p11;
        min_lum = lum11;
    }
    
    return min_pixel;
}

// Funkcja sanityzacji danych wejściowych (usuwanie NaN/Inf)
fn sanitize_pixel(pixel: vec4<f32>) -> vec4<f32> {
    var result: vec4<f32>;
    
    // Sprawdź każdy komponent osobno
    result.r = select(0.0, pixel.r, pixel.r == pixel.r && abs(pixel.r) != 1e38);
    result.g = select(0.0, pixel.g, pixel.g == pixel.g && abs(pixel.g) != 1e38);
    result.b = select(0.0, pixel.b, pixel.b == pixel.b && abs(pixel.b) != 1e38);
    result.a = select(1.0, clamp(pixel.a, 0.0, 1.0), pixel.a == pixel.a);
    
    // Clamp do sensownych wartości HDR (0 do 1000.0)
    result.r = clamp(result.r, 0.0, 1000.0);
    result.g = clamp(result.g, 0.0, 1000.0);
    result.b = clamp(result.b, 0.0, 1000.0);
    
    return result;
}

// Główna funkcja compute shadera
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Sprawdź czy piksel jest w granicach docelowego obrazu MIP
    if (global_id.x >= params.dst_width || global_id.y >= params.dst_height) {
        return;
    }
    
    // Oblicz współrzędne w źródłowym poziomie MIP (każdy piksel docelowy = blok 2x2 źródłowy)
    let src_x = global_id.x * 2u;
    let src_y = global_id.y * 2u;
    
    // Wybierz algorytm downsamplingu na podstawie filter_mode
    var downsampled_pixel: vec4<f32>;
    
    if (params.filter_mode == 1u) {
        // Max filtering
        downsampled_pixel = downsample_2x2_max(src_x, src_y);
    } else if (params.filter_mode == 2u) {
        // Min filtering  
        downsampled_pixel = downsample_2x2_min(src_x, src_y);
    } else {
        // Average filtering (domyślny)
        downsampled_pixel = downsample_2x2_average(src_x, src_y);
    }
    
    // Sanityzacja wynikowego piksela
    let final_pixel = sanitize_pixel(downsampled_pixel);
    
    // Zapisz wynik do bufora wyjściowego
    let output_index = global_id.y * params.dst_width + global_id.x;
    output_pixels[output_index] = final_pixel;
}