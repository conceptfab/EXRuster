use anyhow::Result;
use glam::{Mat3, Vec3};
use crate::gpu_context::GpuContext;
use wgpu::BufferUsages;
use bytemuck::{Pod, Zeroable};

/// Parametry do GPU thumbnail generation
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ThumbnailParamsStd140 {
    pub src_width: u32,
    pub src_height: u32,
    pub dst_width: u32,
    pub dst_height: u32,
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: u32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub _pad0: [u32; 3],           // Padding for 16-byte alignment
    pub color_matrix: [[f32; 4]; 3], // 3x4 matrix (3x3 + padding)
    pub has_color_matrix: u32,
    pub _pad1: [u32; 3],           // Final padding
}

/// GPU thumbnail generation function
#[allow(dead_code)]
pub fn generate_thumbnail_gpu(
    ctx: &GpuContext,
    pixels: &[f32],
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32,
    color_matrix: Option<Mat3>,
) -> Result<Vec<u8>> {

    let src_pixel_count = (src_width as usize) * (src_height as usize);
    let dst_pixel_count = (dst_width as usize) * (dst_height as usize);
    
    if pixels.len() < src_pixel_count * 4 { 
        anyhow::bail!("Input pixel buffer too small"); 
    }

    // Oblicz skale do downsampilng
    let scale_x = src_width as f32 / dst_width as f32;
    let scale_y = src_height as f32 / dst_height as f32;

    // Bufor wejściowy (RGBA f32) - użyj buffer pool
    let input_bytes: &[u8] = bytemuck::cast_slice(pixels);
    let input_size = input_bytes.len() as u64;
    let limits = ctx.device.limits();
    if input_size > limits.max_storage_buffer_binding_size.into() {
        anyhow::bail!(
            "Input image too large for GPU thumbnail (size: {} > max: {})",
            input_size,
            limits.max_storage_buffer_binding_size
        );
    }
    let input_buffer = ctx.get_or_create_buffer(
        input_size,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
        Some("exruster.thumbnail.input_rgba_f32"),
    );
    
    // Skopiuj dane do bufora wejściowego
    ctx.queue.write_buffer(&input_buffer, 0, input_bytes);

    // Bufor wyjściowy (1 u32 na piksel dst) - użyj buffer pool
    let output_size: u64 = (dst_pixel_count as u64) * 4;
    let _output_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        Some("exruster.thumbnail.output_rgba8_u32"),
    );

    // Params buffer - użyj buffer pool
    let cm = color_matrix.unwrap_or_else(|| Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)));
    let params = ThumbnailParamsStd140 {
        src_width,
        src_height,
        dst_width,
        dst_height,
        exposure,
        gamma,
        tonemap_mode,
        scale_x,
        scale_y,
        _pad0: [0; 3],
        color_matrix: [
            [cm.x_axis.x, cm.x_axis.y, cm.x_axis.z, 0.0],
            [cm.y_axis.x, cm.y_axis.y, cm.y_axis.z, 0.0],
            [cm.z_axis.x, cm.z_axis.y, cm.z_axis.z, 0.0],
        ],
        has_color_matrix: if color_matrix.is_some() { 1 } else { 0 },
        _pad1: [0; 3],
    };
    
    let params_bytes = bytemuck::bytes_of(&params);
    let params_size = params_bytes.len() as u64;
    let params_buffer = ctx.get_or_create_buffer(
        params_size,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        Some("exruster.thumbnail.params"),
    );
    ctx.queue.write_buffer(&params_buffer, 0, params_bytes);

    // Staging buffer do odczytu - użyj buffer pool
    let _staging_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        Some("exruster.thumbnail.staging_readback"),
    );

    // Thumbnail pipeline removed - fallback to CPU
    anyhow::bail!("GPU thumbnail generation removed, use CPU fallback instead")
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