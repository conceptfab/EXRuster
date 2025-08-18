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
    use anyhow::Context as _;

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
    let input_buffer = ctx.get_or_create_buffer(
        input_size,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
        Some("exruster.thumbnail.input_rgba_f32"),
    );
    
    // Skopiuj dane do bufora wejściowego
    ctx.queue.write_buffer(&input_buffer, 0, input_bytes);

    // Bufor wyjściowy (1 u32 na piksel dst) - użyj buffer pool
    let output_size: u64 = (dst_pixel_count as u64) * 4;
    let output_buffer = ctx.get_or_create_buffer(
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
    let staging_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        Some("exruster.thumbnail.staging_readback"),
    );

    // Użyj cached pipeline i bind group layout
    let pipeline = ctx.get_thumbnail_pipeline()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached thumbnail pipeline"))?;
    let bgl = ctx.get_thumbnail_bind_group_layout()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached thumbnail bind group layout"))?;

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("exruster.thumbnail.bind_group"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output_buffer.as_entire_binding() },
        ],
    });

    // Dispatch
    let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { 
        label: Some("exruster.thumbnail.encoder") 
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { 
            label: Some("exruster.thumbnail.compute"), 
            timestamp_writes: None 
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let gx = (dst_width + 7) / 8;
        let gy = (dst_height + 7) / 8;
        cpass.dispatch_workgroups(gx, gy, 1);
    }
    
    // Kopiuj wynik do staging
    encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size);
    ctx.queue.submit(Some(encoder.finish()));

    // Mapuj wynik (synchronicznie)
    let slice = staging_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    
    // Czekaj na callback
    rx.recv().context("GPU thumbnail map_async callback failed to deliver")??;
    let data = slice.get_mapped_range();

    // Skopiuj do Vec<u8>
    let mut out_bytes: Vec<u8> = Vec::with_capacity(dst_pixel_count * 4);
    for chunk in data.chunks_exact(4) {
        let v = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let rgba = v.to_le_bytes();
        out_bytes.extend_from_slice(&rgba);
    }

    drop(data);
    staging_buffer.unmap();

    // Zwróć buffery do pool'u dla przyszłego użycia
    ctx.return_buffer(input_buffer, input_size, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    ctx.return_buffer(output_buffer, output_size, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    ctx.return_buffer(params_buffer, params_size, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    ctx.return_buffer(staging_buffer, output_size, BufferUsages::MAP_READ | BufferUsages::COPY_DST);

    Ok(out_bytes)
}

/// Helper function to calculate thumbnail dimensions maintaining aspect ratio
pub fn calculate_thumbnail_size(src_width: u32, src_height: u32, target_height: u32) -> (u32, u32) {
    let aspect_ratio = src_width as f32 / src_height as f32;
    let dst_height = target_height;
    let dst_width = (dst_height as f32 * aspect_ratio).round() as u32;
    (dst_width, dst_height)
}

/// High-level GPU thumbnail generation function
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