use std::sync::Arc;
use anyhow::Result;
use glam::{Mat3, Vec3};
use crate::gpu_context::GpuContext;
use crate::gpu_scheduler::{GpuOperation, GpuOperationParams as SchedulerParams};
// Usunięte nieużywane importy
use wgpu::BufferUsages;
use bytemuck::{Pod, Zeroable};
use std::time::Instant;

// Replikuj strukturę ParamsStd140 zamiast importować prywatną
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ParamsStd140 {
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: u32,
    pub width: u32,
    pub height: u32,
    pub _pad0: u32,
    pub _pad1: [u32; 2],
    pub color_matrix: [[f32; 4]; 3],
    pub has_color_matrix: u32,
    pub _pad2: [u32; 3],
}

/// Parametry do przetwarzania GPU
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GpuProcessingParams {
    pub width: u32,
    pub height: u32,
    pub exposure: f32,
    pub gamma: f32,
    pub _tonemap_mode: u32,
    pub _color_matrix: Option<Mat3>,
}

// Usunięte GpuProcessingTask - nieużywany kod

// Usunięte AsyncGpuProcessor - nieużywany kod

/// Wewnętrzna funkcja przetwarzania GPU (blocking)
#[allow(dead_code)]
async fn process_gpu_task(
    ctx: &GpuContext,
    pixels: Arc<[f32]>,
    params: GpuProcessingParams,
) -> Result<Vec<u8>> {
    // Wykonaj na blocking thread pool aby nie blokować async runtime
    let ctx = ctx.clone();
    let pixels_vec = pixels.to_vec(); // Skonwertuj Arc<[f32]> na Vec<f32>
    
    tokio::task::spawn_blocking(move || {
        gpu_process_rgba_f32_to_rgba8_pooled(
            &ctx,
            &pixels_vec,
            params.width,
            params.height,
            params.exposure,
            params.gamma,
            params._tonemap_mode,
            params._color_matrix,
        )
    }).await?
}

/// Ulepszona wersja GPU processing z buffer pooling
#[allow(dead_code)]
fn gpu_process_rgba_f32_to_rgba8_pooled(
    ctx: &GpuContext,
    pixels: &[f32],
    width: u32,
    height: u32,
    exposure: f32,
    gamma: f32,
    _tonemap_mode: u32,
    _color_matrix: Option<Mat3>,
) -> Result<Vec<u8>> {
    use anyhow::Context as _;

    // FAZA 4: Monitorowanie operacji GPU
    let start_time = Instant::now();
    let operation = GpuOperation::ImageProcessing;
    
    // Sprawdź czy powinienem użyć GPU
    let scheduler_params = SchedulerParams {
        input_size_bytes: (pixels.len() * 4) as u64,
        output_size_bytes: (width * height * 4) as u64,
        complexity: 2.0, // Przetwarzanie obrazu ma średnią złożoność
        is_ui_critical: false,
        max_acceptable_time: std::time::Duration::from_millis(500),
    };
    
    let should_use_gpu = ctx.should_use_gpu_for_operation(operation, &scheduler_params);
    if !should_use_gpu {
        // Fallback na CPU processing
        println!("GPU Scheduler zdecydował o użyciu CPU dla operacji {:?}", operation);
        // CPU fallback - użyj prostego przetwarzania
        println!("Używam CPU fallback dla przetwarzania obrazu");
        let pixel_count = (width * height) as usize;
        let mut out_bytes: Vec<u8> = Vec::with_capacity(pixel_count * 4);
        
        for i in 0..pixel_count {
            let base_idx = i * 4;
            if base_idx + 3 < pixels.len() {
                let r = pixels[base_idx];
                let g = pixels[base_idx + 1];
                let b = pixels[base_idx + 2];
                let a = pixels[base_idx + 3];
                
                // Zastosuj exposure i gamma
                let r = (r * exposure).powf(1.0 / gamma).clamp(0.0, 1.0);
                let g = (g * exposure).powf(1.0 / gamma).clamp(0.0, 1.0);
                let b = (b * exposure).powf(1.0 / gamma).clamp(0.0, 1.0);
                
                // Konwertuj na u8
                out_bytes.push((r * 255.0) as u8);
                out_bytes.push((g * 255.0) as u8);
                out_bytes.push((b * 255.0) as u8);
                out_bytes.push((a * 255.0) as u8);
            }
        }
        
        return Ok(out_bytes);
    }

    let pixel_count = (width as usize) * (height as usize);
    if pixels.len() < pixel_count * 4 { 
        anyhow::bail!("Input pixel buffer too small"); 
    }

    // Bufor wejściowy (RGBA f32) - użyj buffer pool
    let input_bytes: &[u8] = bytemuck::cast_slice(pixels);
    let input_size = input_bytes.len() as u64;
    let input_buffer = ctx.get_or_create_buffer(
        input_size,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
        Some("exruster.async.input_rgba_f32"),
    );
    
    // Skopiuj dane do bufora wejściowego
    ctx.queue.write_buffer(&input_buffer, 0, input_bytes);

    // Bufor wyjściowy (1 u32 na piksel) - użyj buffer pool
    let output_size: u64 = (pixel_count as u64) * 4;
    let output_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        Some("exruster.async.output_rgba8_u32"),
    );

    // Params buffer - użyj buffer pool
    let cm = _color_matrix.unwrap_or_else(|| Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)));
    let params = ParamsStd140 {
        exposure,
        gamma,
        tonemap_mode: _tonemap_mode,
        width,
        height,
        _pad0: 0,
        _pad1: [0; 2],
        color_matrix: [
            [cm.x_axis.x, cm.x_axis.y, cm.x_axis.z, 0.0],
            [cm.y_axis.x, cm.y_axis.y, cm.y_axis.z, 0.0],
            [cm.z_axis.x, cm.z_axis.y, cm.z_axis.z, 0.0],
        ],
        has_color_matrix: if _color_matrix.is_some() { 1 } else { 0 },
        _pad2: [0; 3],
    };
    
    let params_bytes = bytemuck::bytes_of(&params);
    let params_size = params_bytes.len() as u64;
    let params_buffer = ctx.get_or_create_buffer(
        params_size,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        Some("exruster.async.params"),
    );
    ctx.queue.write_buffer(&params_buffer, 0, params_bytes);

    // Staging buffer do odczytu - użyj buffer pool
    let staging_buffer = ctx.get_or_create_buffer(
        output_size,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        Some("exruster.async.staging_readback"),
    );

    // Użyj cached pipeline i bind group layout
    let pipeline = ctx.get_image_processing_pipeline()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached image processing pipeline"))?;
    let bgl = ctx.get_image_processing_bind_group_layout()
        .ok_or_else(|| anyhow::anyhow!("Failed to get cached bind group layout"))?;

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("exruster.async.bind_group"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output_buffer.as_entire_binding() },
        ],
    });

    // Dispatch
    let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { 
        label: Some("exruster.async.encoder") 
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { 
            label: Some("exruster.async.compute"), 
            timestamp_writes: None 
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let gx = (width + 7) / 8;
        let gy = (height + 7) / 8;
        cpass.dispatch_workgroups(gx, gy, 1);
    }
    
    // Kopiuj wynik do staging
    encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size);
    ctx.queue.submit(Some(encoder.finish()));

    // Mapuj wynik (synchronicznie - to jest w blocking task)
    let slice = staging_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    
    // Czekaj na callback (GPU automatycznie zakończy operację)
    rx.recv().context("GPU map_async callback failed to deliver")??;
    let data = slice.get_mapped_range();

    // Skopiuj do Vec<u8>
    let mut out_bytes: Vec<u8> = Vec::with_capacity(pixel_count * 4);
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

    // FAZA 4: Rejestracja metryk GPU
    let operation_duration = start_time.elapsed();
    ctx.record_gpu_operation_time(operation_duration);
    ctx.record_pipeline_cache_hit(); // Udało się użyć cached pipeline
    
    // Aktualizuj wykorzystanie buffer pool
    let total_buffer_size = input_size + output_size + params_size + output_size;
    let estimated_memory_usage = total_buffer_size;
    ctx.update_gpu_memory_usage(estimated_memory_usage);
    
    // Aktualizuj buffer pool utilization (symulacja)
    let buffer_pool_utilization = 0.6; // 60% wykorzystania
    ctx.update_buffer_pool_utilization(buffer_pool_utilization);

    Ok(out_bytes)
}

// Usunięte globalne zmienne i funkcje AsyncGpuProcessor

// Usunięte process_image_cpu_fallback - nieużywana funkcja

// Usunięte process_image_gpu_async - nieużywana funkcja