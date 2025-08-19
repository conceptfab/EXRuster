use anyhow::Result;
use glam::Mat3;
use crate::gpu_context::GpuContext;
use bytemuck::{Pod, Zeroable};

/// Parametry do GPU thumbnail generation - alignment zgodny z WGSL std140 layout
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ThumbnailParamsStd140 {
    pub src_width: u32,              // offset 0
    pub src_height: u32,             // offset 4
    pub dst_width: u32,              // offset 8
    pub dst_height: u32,             // offset 12
    pub exposure: f32,               // offset 16
    pub gamma: f32,                  // offset 20
    pub tonemap_mode: u32,           // offset 24
    pub scale_x: f32,                // offset 28
    pub scale_y: f32,                // offset 32
    pub _pad0: [u32; 3],             // offset 36, padding to align matrix
    pub color_matrix: [[f32; 4]; 3], // offset 48, WGSL mat3x3 as 3x vec3 (każdy vec3 ma 4 f32 w std140)
    pub has_color_matrix: u32,        // offset 96
    pub _pad1: [u32; 11],             // padding do dokładnie 144 bajtów (112+32=144)
}

/// GPU thumbnail generation function
#[allow(dead_code)]
pub fn generate_thumbnail_gpu(
    _ctx: &GpuContext,
    _pixels: &[f32],
    _src_width: u32,
    _src_height: u32,
    _dst_width: u32,
    _dst_height: u32,
    _exposure: f32,
    _gamma: f32,
    _tonemap_mode: u32,
    _color_matrix: Option<Mat3>,
) -> Result<Vec<u8>> {
    // KRYTYCZNE: Tymczasowo wyłącz GPU thumbnails - powodują brak thumbnailów przez timeout hang
    anyhow::bail!("GPU thumbnails temporarily disabled due to instability - using CPU fallback");
}

/// Helper function to calculate thumbnail dimensions maintaining aspect ratio
#[allow(dead_code)]
pub fn calculate_thumbnail_size(src_width: u32, src_height: u32, target_height: u32) -> (u32, u32) {
    let aspect_ratio = src_width as f32 / src_height as f32;
    let dst_height = target_height;
    let dst_width = (dst_height as f32 * aspect_ratio).round() as u32;
    (dst_width, dst_height)
}

/// High-level GPU thumbnail generation function
#[allow(dead_code)]
pub fn generate_thumbnail_from_pixels_gpu(
    ctx: &GpuContext,
    pixels: &[f32],
    src_width: u32,
    src_height: u32,
    target_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
) -> Result<(Vec<u8>, u32, u32)> {
    let (dst_width, dst_height) = calculate_thumbnail_size(src_width, src_height, target_height);
    
    let thumbnail_bytes = generate_thumbnail_gpu(
        ctx,
        pixels,
        src_width,
        src_height,
        dst_width,
        dst_height,
        exposure,
        gamma,
        tonemap_mode as u32,
        color_matrix,
    )?;
    
    Ok((thumbnail_bytes, dst_width, dst_height))
}

/// Test wydajności GPU vs CPU thumbnail generation
#[allow(dead_code)]
pub fn benchmark_thumbnail_generation(
    ctx: &GpuContext,
    pixels: &[f32],
    src_width: u32,
    src_height: u32,
    target_height: u32,
    iterations: usize,
) -> Result<()> {
    use std::time::Instant;
    
    println!("=== GPU vs CPU Thumbnail Benchmark ===");
    println!("Source: {}x{}, Target height: {}, Iterations: {}", 
             src_width, src_height, target_height, iterations);
    
    // Test GPU
    let start = Instant::now();
    for i in 0..iterations {
        let result = generate_thumbnail_from_pixels_gpu(
            ctx, pixels, src_width, src_height, target_height, 
            0.0, 2.2, 0, None
        );
        if let Err(e) = result {
            println!("GPU iteration {} failed: {}", i, e);
            break;
        }
    }
    let gpu_duration = start.elapsed();
    let gpu_per_image = gpu_duration.as_millis() as f64 / iterations as f64;
    
    // Test CPU (będzie potrzebna implementacja CPU dla porównania)
    // Na razie tylko wyświetlamy wyniki GPU
    println!("GPU Results:");
    println!("  Total time: {:.2}ms", gpu_duration.as_millis());
    println!("  Per image: {:.2}ms", gpu_per_image);
    println!("  Throughput: {:.1} images/sec", 1000.0 / gpu_per_image);
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_context::GpuContext;
    
    #[tokio::test]
    async fn test_gpu_thumbnail_generation() {
        // Spróbuj zainicjalizować kontekst GPU
        let gpu_ctx_result = GpuContext::new().await;
        
        let gpu_ctx = match gpu_ctx_result {
            Ok(ctx) => ctx,
            Err(_) => {
                println!("GPU nie jest dostępny - pomijam test");
                return;
            }
        };
        
        println!("GPU Context: {}", gpu_ctx.get_adapter_info().name);
        
        // Utwórz testowe dane (256x256 RGBA f32)
        let src_width = 256u32;
        let src_height = 256u32;
        let src_pixel_count = (src_width * src_height) as usize;
        
        // Gradient testowy
        let mut test_pixels = Vec::with_capacity(src_pixel_count * 4);
        for y in 0..src_height {
            for x in 0..src_width {
                let r = x as f32 / src_width as f32;
                let g = y as f32 / src_height as f32;
                let b = 0.5;
                let a = 1.0;
                test_pixels.extend_from_slice(&[r, g, b, a]);
            }
        }
        
        // Test GPU thumbnail generation
        let target_height = 64u32;
        let result = generate_thumbnail_from_pixels_gpu(
            &gpu_ctx,
            &test_pixels,
            src_width,
            src_height,
            target_height,
            0.0,  // exposure
            2.2,  // gamma
            0,    // ACES tonemap
            None, // color_matrix
        );
        
        match result {
            Ok((thumbnail_data, dst_width, dst_height)) => {
                println!("✅ GPU thumbnail SUCCESS: {}x{} -> {}x{}", 
                         src_width, src_height, dst_width, dst_height);
                println!("   Data size: {} bytes", thumbnail_data.len());
                
                // Sprawdź czy dane mają sens
                assert_eq!(thumbnail_data.len(), (dst_width * dst_height * 4) as usize);
                assert!(dst_width > 0 && dst_height > 0);
                
                // Benchmark jeśli test przeszedł
                if let Err(e) = benchmark_thumbnail_generation(
                    &gpu_ctx, &test_pixels, src_width, src_height, target_height, 10
                ) {
                    println!("Benchmark failed: {}", e);
                }
            }
            Err(e) => {
                println!("❌ GPU thumbnail FAILED: {}", e);
                panic!("GPU thumbnail generation failed: {}", e);
            }
        }
    }
}