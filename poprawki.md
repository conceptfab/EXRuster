Problem 1: Brak interpolacji w GPU shader
Plik: src/gpu_thumbnails.rs

W funkcji process_thumbnail używacie prostego mapowania pikseli:

rust
// Współrzędne źródłowe
let src_x = u32(f32(x) * scale_x);
let src_y = u32(f32(y) * scale_y);
Proponowana zmiana: Dodaj bilinearną interpolację w shaderze

wgsl
// W shaderze THUMBNAIL_COMPUTE_SHADER, zamień sekcję pobierania pikseli:

// Oblicz współrzędne źródłowe z częścią ułamkową
let src_x_f = f32(x) * scale_x;
let src_y_f = f32(y) * scale_y;

let src_x0 = u32(floor(src_x_f));
let src_y0 = u32(floor(src_y_f));
let src_x1 = min(src_x0 + 1u, params.input_width - 1u);
let src_y1 = min(src_y0 + 1u, params.input_height - 1u);

// Wagi interpolacji
let fx = fract(src_x_f);
let fy = fract(src_y_f);

// Pobierz 4 sąsiednie piksele
let idx00 = (src_y0 * params.input_width + src_x0) * 4u;
let idx10 = (src_y0 * params.input_width + src_x1) * 4u;
let idx01 = (src_y1 * params.input_width + src_x0) * 4u;
let idx11 = (src_y1 * params.input_width + src_x1) * 4u;

// Interpolacja bilinearna dla każdego kanału
let p00 = vec4<f32>(input_pixels[idx00], input_pixels[idx00 + 1], input_pixels[idx00 + 2], input_pixels[idx00 + 3]);
let p10 = vec4<f32>(input_pixels[idx10], input_pixels[idx10 + 1], input_pixels[idx10 + 2], input_pixels[idx10 + 3]);
let p01 = vec4<f32>(input_pixels[idx01], input_pixels[idx01 + 1], input_pixels[idx01 + 2], input_pixels[idx01 + 3]);
let p11 = vec4<f32>(input_pixels[idx11], input_pixels[idx11 + 1], input_pixels[idx11 + 2], input_pixels[idx11 + 3]);

let p0 = mix(p00, p10, fx);
let p1 = mix(p01, p11, fx);
let pixel = mix(p0, p1, fy);

let r = pixel.x;
let g = pixel.y;
let b = pixel.z;
let a = pixel.w;
Problem 2: Brak interpolacji w CPU fallback
Plik: src/thumbnails.rs

W funkcji generate_single_exr_thumbnail_work:

rust
let src_x = ((x as f32 / scale) as u32).min(width.saturating_sub(1));
let src_y = ((y as f32 / scale) as u32).min(height.saturating_sub(1));
Proponowana zmiana: Dodaj interpolację bilinearną

rust
// W funkcji generate_single_exr_thumbnail_work, zamień sekcję parallel processing:

pixels.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
    let x = (i as u32) % thumb_w;
    let y = (i as u32) / thumb_w;
    
    // Współrzędne źródłowe z częścią ułamkową
    let src_x_f = (x as f32) / scale;
    let src_y_f = (y as f32) / scale;
    
    let src_x0 = src_x_f.floor() as u32;
    let src_y0 = src_y_f.floor() as u32;
    let src_x1 = (src_x0 + 1).min(width.saturating_sub(1));
    let src_y1 = (src_y0 + 1).min(height.saturating_sub(1));
    
    // Wagi interpolacji
    let fx = src_x_f.fract();
    let fy = src_y_f.fract();
    
    // Pobierz 4 sąsiednie piksele
    let idx00 = (src_y0 as usize) * (width as usize) + (src_x0 as usize);
    let idx10 = (src_y0 as usize) * (width as usize) + (src_x1 as usize);
    let idx01 = (src_y1 as usize) * (width as usize) + (src_x0 as usize);
    let idx11 = (src_y1 as usize) * (width as usize) + (src_x1 as usize);
    
    let (r00, g00, b00, a00) = raw_pixels[idx00];
    let (r10, g10, b10, a10) = raw_pixels[idx10];
    let (r01, g01, b01, a01) = raw_pixels[idx01];
    let (r11, g11, b11, a11) = raw_pixels[idx11];
    
    // Interpolacja bilinearna
    let r0 = r00 * (1.0 - fx) + r10 * fx;
    let r1 = r01 * (1.0 - fx) + r11 * fx;
    let mut r = r0 * (1.0 - fy) + r1 * fy;
    
    let g0 = g00 * (1.0 - fx) + g10 * fx;
    let g1 = g01 * (1.0 - fx) + g11 * fx;
    let mut g = g0 * (1.0 - fy) + g1 * fy;
    
    let b0 = b00 * (1.0 - fx) + b10 * fx;
    let b1 = b01 * (1.0 - fx) + b11 * fx;
    let mut b = b0 * (1.0 - fy) + b1 * fy;
    
    let a0 = a00 * (1.0 - fx) + a10 * fx;
    let a1 = a01 * (1.0 - fx) + a11 * fx;
    let a = a0 * (1.0 - fy) + a1 * fy;
    
    // Reszta kodu pozostaje bez zmian (macierz kolorów i process_pixel)
    if let Some(mat) = m {
        let v = mat * Vec3::new(r, g, b);
        r = v.x; g = v.y; b = v.z;
    }
    let px = process_pixel(r, g, b, a, exposure, gamma, tonemap_mode);
    out[0] = px.r; out[1] = px.g; out[2] = px.b; out[3] = px.a;
});
Problem 3: CPU fallback w process_single_thumbnail_cpu
Plik: src/thumbnails.rs

Ta sama sytuacja - brak interpolacji:

rust
// W funkcji process_single_thumbnail_cpu, zamień pętlę przetwarzania:

for y in 0..work.target_height {
    for x in 0..work.target_width {
        let src_x_f = x as f32 * scale_x;
        let src_y_f = y as f32 * scale_y;
        
        let src_x0 = src_x_f.floor() as u32;
        let src_y0 = src_y_f.floor() as u32;
        let src_x1 = (src_x0 + 1).min(work.width.saturating_sub(1));
        let src_y1 = (src_y0 + 1).min(work.height.saturating_sub(1));
        
        let fx = src_x_f.fract();
        let fy = src_y_f.fract();
        
        // Indeksy dla 4 pikseli
        let idx00 = (src_y0 as usize * work.width as usize + src_x0 as usize) * 4;
        let idx10 = (src_y0 as usize * work.width as usize + src_x1 as usize) * 4;
        let idx01 = (src_y1 as usize * work.width as usize + src_x0 as usize) * 4;
        let idx11 = (src_y1 as usize * work.width as usize + src_x1 as usize) * 4;
        
        if idx11 + 3 < work.raw_pixels.len() {
            // Pobierz 4 piksele i interpoluj
            let r00 = work.raw_pixels[idx00];
            let g00 = work.raw_pixels[idx00 + 1];
            let b00 = work.raw_pixels[idx00 + 2];
            let a00 = work.raw_pixels[idx00 + 3];
            
            // ... (analogicznie dla pozostałych 3 pikseli)
            
            // Bilinearna interpolacja
            let r = lerp2d(r00, r10, r01, r11, fx, fy);
            let g = lerp2d(g00, g10, g01, g11, fx, fy);
            let b = lerp2d(b00, b10, b01, b11, fx, fy);
            let a = lerp2d(a00, a10, a01, a11, fx, fy);
            
            // Reszta przetwarzania...
        }
    }
}

// Funkcja pomocnicza:
fn lerp2d(v00: f32, v10: f32, v01: f32, v11: f32, fx: f32, fy: f32) -> f32 {
    let v0 = v00 * (1.0 - fx) + v10 * fx;
    let v1 = v01 * (1.0 - fx) + v11 * fx;
    v0 * (1.0 - fy) + v1 * fy
}
Te zmiany powinny znacząco poprawić jakość miniaturek poprzez zastosowanie interpolacji bilinearnej zamiast prostego "nearest neighbor" sampling. Rezultatem będą gładsze krawędzie i lepsza jakość wizualna miniaturek.





