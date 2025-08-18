use std::sync::Arc;
use anyhow::Result;
use glam::{Mat3, Vec3};
use crate::gpu_context::GpuContext;
use tokio::sync::{mpsc, oneshot};
use std::collections::VecDeque;
use wgpu::BufferUsages;
use bytemuck::{Pod, Zeroable};

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
pub struct GpuProcessingParams {
    pub width: u32,
    pub height: u32,
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: u32,
    pub color_matrix: Option<Mat3>,
}

/// Zadanie przetwarzania GPU
#[derive(Debug)]
pub struct GpuProcessingTask {
    pub pixels: Arc<[f32]>,
    pub params: GpuProcessingParams,
    pub response_tx: oneshot::Sender<Result<Vec<u8>>>,
}

/// Asynchroniczny procesor GPU z kolejką zadań
pub struct AsyncGpuProcessor {
    #[allow(dead_code)]
    task_tx: mpsc::UnboundedSender<GpuProcessingTask>,
    _handle: tokio::task::JoinHandle<()>,
}

impl AsyncGpuProcessor {
    /// Tworzy nowy asynchroniczny procesor GPU
    pub fn new(gpu_context: Arc<GpuContext>) -> Self {
        let (task_tx, mut task_rx) = mpsc::unbounded_channel::<GpuProcessingTask>();
        
        let handle = tokio::spawn(async move {
            let mut queue: VecDeque<GpuProcessingTask> = VecDeque::new();
            
            // Główna pętla przetwarzania
            while let Some(task) = task_rx.recv().await {
                queue.push_back(task);
                
                // Przetwórz wszystkie dostępne zadania w batch'u
                while let Some(current_task) = queue.pop_front() {
                    let result = process_gpu_task(&gpu_context, current_task.pixels, current_task.params).await;
                    
                    // Wyślij wynik przez oneshot channel (ignoruj błąd jeśli receiver został dropped)
                    let _ = current_task.response_tx.send(result);
                }
            }
        });

        Self {
            task_tx,
            _handle: handle,
        }
    }

    /// Async przetwarzanie obrazu na GPU
    #[allow(dead_code)]
    pub async fn process_image_async(
        &self,
        pixels: Arc<[f32]>,
        params: GpuProcessingParams,
    ) -> Result<Vec<u8>> {
        let (response_tx, response_rx) = oneshot::channel();
        
        let task = GpuProcessingTask {
            pixels,
            params,
            response_tx,
        };

        // Wyślij zadanie do kolejki
        self.task_tx.send(task)
            .map_err(|_| anyhow::anyhow!("GPU processor has been shut down"))?;

        // Czekaj na wynik
        response_rx.await
            .map_err(|_| anyhow::anyhow!("GPU processing task was cancelled"))?
    }
}

/// Wewnętrzna funkcja przetwarzania GPU (blocking)
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
            params.tonemap_mode,
            params.color_matrix,
        )
    }).await?
}

/// Ulepszona wersja GPU processing z buffer pooling
fn gpu_process_rgba_f32_to_rgba8_pooled(
    ctx: &GpuContext,
    pixels: &[f32],
    width: u32,
    height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32,
    color_matrix: Option<Mat3>,
) -> Result<Vec<u8>> {
    use anyhow::Context as _;

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
    let cm = color_matrix.unwrap_or_else(|| Mat3::from_diagonal(Vec3::new(1.0, 1.0, 1.0)));
    let params = ParamsStd140 {
        exposure,
        gamma,
        tonemap_mode,
        width,
        height,
        _pad0: 0,
        _pad1: [0; 2],
        color_matrix: [
            [cm.x_axis.x, cm.x_axis.y, cm.x_axis.z, 0.0],
            [cm.y_axis.x, cm.y_axis.y, cm.y_axis.z, 0.0],
            [cm.z_axis.x, cm.z_axis.y, cm.z_axis.z, 0.0],
        ],
        has_color_matrix: if color_matrix.is_some() { 1 } else { 0 },
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

    Ok(out_bytes)
}

/// Globalna instancja async GPU processor
static GPU_PROCESSOR: std::sync::LazyLock<std::sync::Mutex<Option<Arc<AsyncGpuProcessor>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

/// Inicjalizuje globalny async GPU processor
pub fn initialize_async_gpu_processor(gpu_context: Arc<GpuContext>) {
    let processor = Arc::new(AsyncGpuProcessor::new(gpu_context));
    if let Ok(mut guard) = GPU_PROCESSOR.lock() {
        *guard = Some(processor);
    }
}

/// Pobiera referencję do globalnego async GPU processor
#[allow(dead_code)]
pub fn get_async_gpu_processor() -> Option<Arc<AsyncGpuProcessor>> {
    if let Ok(guard) = GPU_PROCESSOR.lock() {
        guard.clone()
    } else {
        None
    }
}

/// Async wrapper dla łatwego użycia
#[allow(dead_code)]
pub async fn process_image_gpu_async(
    pixels: Arc<[f32]>,
    width: u32,
    height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32,
    color_matrix: Option<Mat3>,
) -> Result<Vec<u8>> {
    let processor = get_async_gpu_processor()
        .ok_or_else(|| anyhow::anyhow!("Async GPU processor not initialized"))?;

    let params = GpuProcessingParams {
        width,
        height,
        exposure,
        gamma,
        tonemap_mode,
        color_matrix,
    };

    processor.process_image_async(pixels, params).await
}