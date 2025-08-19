// CUDA Image Processing for EXRuster
// Real CUDA implementation for high-performance image processing

use crate::cuda_context::CudaContext;
use crate::cuda_kernels::CudaKernels;
use cudarc::driver::{CudaSlice, DeviceRepr};
use anyhow::Result;
use std::sync::{Arc, Mutex, OnceLock};

// Global kernels cache - initialized once per context
static KERNELS_CACHE: OnceLock<Arc<Mutex<Option<CudaKernels>>>> = OnceLock::new();

/// Initialize CUDA kernels (called once per context)
fn get_or_create_kernels(ctx: &CudaContext) -> Result<Arc<CudaKernels>> {
    let cache = KERNELS_CACHE.get_or_init(|| Arc::new(Mutex::new(None)));
    let mut cache_guard = cache.lock().unwrap();
    
    if cache_guard.is_none() {
        println!("CUDA: Creating kernels for the first time...");
        let kernels = CudaKernels::new(ctx.device().clone())?;
        *cache_guard = Some(kernels);
    }
    
    // Clone the Arc to return it
    let kernels_ref = cache_guard.as_ref().unwrap();
    // We need to create a new Arc since we can't clone from &CudaKernels
    // Instead, let's return a reference and change the API
    anyhow::bail!("Need to refactor kernels storage")
}

/// CUDA-accelerated tone mapping and color space conversion
pub async fn cuda_process_rgba_f32_to_rgba8(
    ctx: &CudaContext,
    pixels: &[f32],
    width: u32,
    height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32,
    color_matrix: Option<glam::Mat3>,
) -> Result<Vec<u8>> {
    let pixel_count = (width * height) as usize;
    
    if pixels.len() != pixel_count * 4 {
        anyhow::bail!("CUDA: Input pixel buffer size mismatch");
    }
    
    println!("CUDA: Processing {}x{} image (tone mapping)", width, height);
    
    // Create kernels
    let kernels = CudaKernels::new(ctx.device().clone())?;
    
    // Allocate GPU memory
    let mut input_gpu: CudaSlice<f32> = ctx.alloc_or_reuse(pixels.len())?;
    let mut output_gpu: CudaSlice<u8> = ctx.alloc_or_reuse(pixel_count * 4)?;
    
    // Copy input data to GPU
    ctx.device().htod_copy(pixels, &mut input_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy input to GPU: {:?}", e)
    })?;
    
    // Prepare color matrix if provided
    let color_matrix_gpu = if let Some(matrix) = color_matrix {
        let matrix_data = [
            matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z,
            matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z,
            matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z,
        ];
        let mut matrix_gpu: CudaSlice<f32> = ctx.alloc_or_reuse(9)?;
        ctx.device().htod_copy(&matrix_data, &mut matrix_gpu).map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to copy color matrix to GPU: {:?}", e)
        })?;
        Some(matrix_gpu)
    } else {
        None
    };
    
    // Launch kernel (resize to same size + tone mapping)
    kernels.launch_thumbnail_kernel(
        &input_gpu,
        &mut output_gpu,
        width,
        height,
        width,  // Same size - just tone mapping
        height,
        exposure,
        gamma,
        tonemap_mode as i32,
        color_matrix_gpu.as_ref(),
    )?;
    
    // Synchronize and copy result back
    ctx.synchronize()?;
    
    let mut result = vec![0u8; pixel_count * 4];
    ctx.device().dtoh_sync_copy(&output_gpu, &mut result).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy result from GPU: {:?}", e)
    })?;
    
    println!("CUDA: Successfully processed image");
    Ok(result)
}

/// CUDA-accelerated histogram computation
pub async fn cuda_compute_histogram(
    ctx: &CudaContext,
    pixels: &[f32],
    width: u32,
    height: u32,
) -> Result<crate::histogram::HistogramData> {
    let pixel_count = (width * height) as usize;
    
    if pixels.len() != pixel_count * 4 {
        anyhow::bail!("CUDA: Input pixel buffer size mismatch");
    }
    
    println!("CUDA: Computing histogram for {}x{} image", width, height);
    
    // Create kernels
    let kernels = CudaKernels::new(ctx.device().clone())?;
    
    // Find min/max values first (simplified - use fixed range for now)
    let min_val = 0.0f32;
    let max_val = 10.0f32; // Common EXR range
    let bin_count = 256;
    
    // Allocate GPU memory
    let mut input_gpu: CudaSlice<f32> = ctx.alloc_or_reuse(pixels.len())?;
    let mut red_bins_gpu: CudaSlice<i32> = ctx.alloc_or_reuse(bin_count)?;
    let mut green_bins_gpu: CudaSlice<i32> = ctx.alloc_or_reuse(bin_count)?;
    let mut blue_bins_gpu: CudaSlice<i32> = ctx.alloc_or_reuse(bin_count)?;
    let mut lum_bins_gpu: CudaSlice<i32> = ctx.alloc_or_reuse(bin_count)?;
    
    // Copy input data to GPU
    ctx.device().htod_copy(pixels, &mut input_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy input to GPU: {:?}", e)
    })?;
    
    // Zero the histogram bins
    ctx.device().memset_zeros(&mut red_bins_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to zero red bins: {:?}", e)
    })?;
    ctx.device().memset_zeros(&mut green_bins_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to zero green bins: {:?}", e)
    })?;
    ctx.device().memset_zeros(&mut blue_bins_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to zero blue bins: {:?}", e)
    })?;
    ctx.device().memset_zeros(&mut lum_bins_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to zero lum bins: {:?}", e)
    })?;
    
    // Launch histogram kernel
    kernels.launch_histogram_kernel(
        &input_gpu,
        &mut red_bins_gpu,
        &mut green_bins_gpu,
        &mut blue_bins_gpu,
        &mut lum_bins_gpu,
        width,
        height,
        min_val,
        max_val,
        bin_count as i32,
    )?;
    
    // Synchronize and copy results back
    ctx.synchronize()?;
    
    let mut red_bins = vec![0i32; bin_count];
    let mut green_bins = vec![0i32; bin_count];
    let mut blue_bins = vec![0i32; bin_count];
    let mut lum_bins = vec![0i32; bin_count];
    
    ctx.device().dtoh_sync_copy(&red_bins_gpu, &mut red_bins).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy red bins from GPU: {:?}", e)
    })?;
    ctx.device().dtoh_sync_copy(&green_bins_gpu, &mut green_bins).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy green bins from GPU: {:?}", e)
    })?;
    ctx.device().dtoh_sync_copy(&blue_bins_gpu, &mut blue_bins).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy blue bins from GPU: {:?}", e)
    })?;
    ctx.device().dtoh_sync_copy(&lum_bins_gpu, &mut lum_bins).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy lum bins from GPU: {:?}", e)
    })?;
    
    // Create histogram data
    let histogram = crate::histogram::HistogramData {
        red_bins: red_bins.into_iter().map(|x| x as u32).collect(),
        green_bins: green_bins.into_iter().map(|x| x as u32).collect(),
        blue_bins: blue_bins.into_iter().map(|x| x as u32).collect(),
        luminance_bins: lum_bins.into_iter().map(|x| x as u32).collect(),
        bin_count,
        min_value: min_val,
        max_value: max_val,
        total_pixels: pixel_count as u32,
    };
    
    println!("CUDA: Successfully computed histogram");
    Ok(histogram)
}

/// CUDA-accelerated thumbnail generation with resize + tone mapping
pub async fn cuda_generate_thumbnail_from_pixels(
    ctx: &CudaContext,
    pixels: &[f32],
    src_width: u32,
    src_height: u32,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<glam::Mat3>,
) -> Result<(Vec<u8>, u32, u32)> {
    let src_pixel_count = (src_width * src_height) as usize;
    
    if pixels.len() != src_pixel_count * 4 {
        anyhow::bail!("CUDA: Input pixel buffer size mismatch");
    }
    
    // Calculate thumbnail dimensions maintaining aspect ratio
    let aspect_ratio = src_width as f32 / src_height as f32;
    let thumb_width = (thumb_height as f32 * aspect_ratio).round() as u32;
    let thumb_pixel_count = (thumb_width * thumb_height) as usize;
    
    println!("CUDA: Generating thumbnail {}x{} -> {}x{}", 
             src_width, src_height, thumb_width, thumb_height);
    
    // Create kernels
    let kernels = CudaKernels::new(ctx.device().clone())?;
    
    // Allocate GPU memory
    let mut input_gpu: CudaSlice<f32> = ctx.alloc_or_reuse(pixels.len())?;
    let mut output_gpu: CudaSlice<u8> = ctx.alloc_or_reuse(thumb_pixel_count * 4)?;
    
    // Copy input data to GPU
    ctx.device().htod_copy(pixels, &mut input_gpu).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy input to GPU: {:?}", e)
    })?;
    
    // Prepare color matrix if provided
    let color_matrix_gpu = if let Some(matrix) = color_matrix {
        let matrix_data = [
            matrix.x_axis.x, matrix.x_axis.y, matrix.x_axis.z,
            matrix.y_axis.x, matrix.y_axis.y, matrix.y_axis.z,
            matrix.z_axis.x, matrix.z_axis.y, matrix.z_axis.z,
        ];
        let mut matrix_gpu: CudaSlice<f32> = ctx.alloc_or_reuse(9)?;
        ctx.device().htod_copy(&matrix_data, &mut matrix_gpu).map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to copy color matrix to GPU: {:?}", e)
        })?;
        Some(matrix_gpu)
    } else {
        None
    };
    
    // Launch combined resize + tone mapping kernel
    kernels.launch_thumbnail_kernel(
        &input_gpu,
        &mut output_gpu,
        src_width,
        src_height,
        thumb_width,
        thumb_height,
        exposure,
        gamma,
        tonemap_mode,
        color_matrix_gpu.as_ref(),
    )?;
    
    // Synchronize and copy result back
    ctx.synchronize()?;
    
    let mut result = vec![0u8; thumb_pixel_count * 4];
    ctx.device().dtoh_sync_copy(&output_gpu, &mut result).map_err(|e| {
        anyhow::anyhow!("CUDA: Failed to copy result from GPU: {:?}", e)
    })?;
    
    println!("CUDA: Successfully generated thumbnail");
    Ok((result, thumb_width, thumb_height))
}