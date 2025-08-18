use anyhow::Result;
use crate::gpu_context::GpuContext;
use wgpu::BufferUsages;
use bytemuck::{Pod, Zeroable};

/// Parametry do GPU MIP generation
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MipParamsStd140 {
    pub src_width: u32,
    pub src_height: u32,
    pub dst_width: u32,
    pub dst_height: u32,
    pub mip_level: u32,
    pub filter_mode: u32,
    pub preserve_alpha: u32,
    pub _pad0: u32,
}

/// Tryb filtrowania dla MIP generation
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum MipFilterMode {
    Average = 0,  // Średnia z bloku 2x2
}

/// Konfiguracja MIP generation
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MipConfig {
    pub filter_mode: MipFilterMode,
    pub preserve_alpha: bool,
    pub max_levels: Option<u32>,
}

impl Default for MipConfig {
    fn default() -> Self {
        Self {
            filter_mode: MipFilterMode::Average,
            preserve_alpha: true,
            max_levels: None,
        }
    }
}

/// Generuje jeden poziom MIP na GPU
#[allow(dead_code)]
pub fn generate_mip_level_gpu(
    ctx: &GpuContext,
    src_pixels: &[f32],
    src_width: u32,
    src_height: u32,
    mip_level: u32,
    config: &MipConfig,
) -> Result<(Vec<f32>, u32, u32)> {
    use anyhow::Context as _;

    // Oblicz wymiary docelowego poziomu MIP
    let dst_width = (src_width / 2).max(1);
    let dst_height = (src_height / 2).max(1);
    
    let src_pixel_count = (src_width as usize) * (src_height as usize);
    let dst_pixel_count = (dst_width as usize) * (dst_height as usize);
    
    if src_pixels.len() < src_pixel_count * 4 {
        anyhow::bail!("Input pixel buffer too small for MIP generation");
    }

    // Bufor wejściowy (RGBA f32) - użyj buffer pool
    let input_bytes: &[u8] = bytemuck::cast_slice(src_pixels);
    let input_size = input_bytes.len() as u64;
    let limits = ctx.device.limits();
    if input_size > limits.max_storage_buffer_binding_size.into() {
        anyhow::bail!(
            "Input image too large for GPU MIP generation (size: {} > max: {})",
            input_size,
            limits.max_storage_buffer_binding_size
        );
    }
    let input_buffer = ctx.get_or_create_buffer(
        input_size,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
        Some("exruster.mip.input_rgba_f32"),
    );
    
    // Skopiuj dane do bufora wejściowego
    ctx.queue.write_buffer(&input_buffer, 0, input_bytes);

    // Bufor wyjściowy (RGBA f32) - użyj buffer pool
    let output_size: u64 = (dst_pixel_count as u64) * 4 * 4; // 4 komponenty * 4 bajty
    let output_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        Some("exruster.mip.output_rgba_f32"),
    );

    // Params buffer - użyj buffer pool
    let params = MipParamsStd140 {
        src_width,
        src_height,
        dst_width,
        dst_height,
        mip_level,
        filter_mode: config.filter_mode as u32,
        preserve_alpha: if config.preserve_alpha { 1 } else { 0 },
        _pad0: 0,
    };
    
    let params_bytes = bytemuck::bytes_of(&params);
    let params_size = params_bytes.len() as u64;
    let params_buffer = ctx.get_or_create_buffer(
        params_size,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        Some("exruster.mip.params"),
    );
    ctx.queue.write_buffer(&params_buffer, 0, params_bytes);

    // Staging buffer do odczytu - użyj buffer pool
    let staging_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        Some("exruster.mip.staging_readback"),
    );

    // Użyj cached pipeline i bind group layout
    let pipeline = ctx.get_mip_generation_pipeline()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached MIP generation pipeline"))?;
    let bgl = ctx.get_mip_generation_bind_group_layout()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached MIP generation bind group layout"))?;

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("exruster.mip.bind_group"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output_buffer.as_entire_binding() },
        ],
    });

    // Dispatch
    let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { 
        label: Some("exruster.mip.encoder") 
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { 
            label: Some("exruster.mip.compute"), 
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
    rx.recv().context("GPU MIP map_async callback failed to deliver")??;
    let data = slice.get_mapped_range();

    // Skopiuj do Vec<f32>
    let output_floats: Vec<f32> = bytemuck::cast_slice(&data).to_vec();

    drop(data);
    staging_buffer.unmap();

    // Zwróć buffery do pool'u dla przyszłego użycia
    ctx.return_buffer(input_buffer, input_size, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    ctx.return_buffer(output_buffer, output_size, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    ctx.return_buffer(params_buffer, params_size, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    ctx.return_buffer(staging_buffer, output_size, BufferUsages::MAP_READ | BufferUsages::COPY_DST);

    Ok((output_floats, dst_width, dst_height))
}

/// Generuje kompletny łańcuch MIP na GPU
#[allow(dead_code)]
pub fn build_mip_chain_gpu(
    ctx: &GpuContext,
    base_pixels: &[f32],
    base_width: u32,
    base_height: u32,
    config: &MipConfig,
) -> Result<Vec<(Vec<f32>, u32, u32)>> {
    let mut mip_levels = Vec::new();
    
    // Poziom 0 (oryginalny obraz) nie jest przetwarzany
    mip_levels.push((base_pixels.to_vec(), base_width, base_height));
    
    let mut current_pixels = base_pixels.to_vec();
    let mut current_width = base_width;
    let mut current_height = base_height;
    let mut mip_level = 1;
    
    // Generuj kolejne poziomy MIP aż do rozmiaru 1x1 lub max_levels
    loop {
        // Sprawdź warunki zakończenia
        if current_width == 1 && current_height == 1 {
            break;
        }
        
        if let Some(max_levels) = config.max_levels {
            if mip_level >= max_levels {
                break;
            }
        }
        
        // Generuj następny poziom MIP
        let (next_pixels, next_width, next_height) = generate_mip_level_gpu(
            ctx,
            &current_pixels,
            current_width,
            current_height,
            mip_level,
            config,
        )?;
        
        mip_levels.push((next_pixels.clone(), next_width, next_height));
        
        // Przygotuj się do następnej iteracji
        current_pixels = next_pixels;
        current_width = next_width;
        current_height = next_height;
        mip_level += 1;
    }
    
    Ok(mip_levels)
}

