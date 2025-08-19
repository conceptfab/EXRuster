use wgpu::{self, *};
use anyhow::Result;
use std::sync::Arc;
use crate::gpu_context::GpuContext;

/// Struktura do przetwarzania obrazów na GPU
#[allow(dead_code)]
pub struct GpuProcessor {
    context: Arc<GpuContext>,
}

#[allow(dead_code)]
impl GpuProcessor {
    /// Tworzy nowy procesor GPU
    pub fn new(context: Arc<GpuContext>) -> Self {
        Self { context }
    }

    /// Przetwarza dane obrazu na GPU
    pub async fn process_image(&self, input: &[f32], width: u32, height: u32) -> Result<Vec<f32>> {
        let pixel_count = (width * height) as usize;
        let input_size = (pixel_count * 4 * std::mem::size_of::<f32>()) as u64;
        let output_size = input_size;

        // Utwórz buffery
        let input_buffer = self.context.get_or_create_buffer(
            input_size,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
            Some("gpu_processing_input"),
        );

        let output_buffer = self.context.get_or_create_buffer(
            output_size,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            Some("gpu_processing_output"),
        );

        // Kopiuj dane wejściowe
        self.context.queue.write_buffer(&input_buffer, 0, bytemuck::cast_slice(input));

        // Pobierz pipeline
        let pipeline = self.context.get_image_processing_pipeline()
            .ok_or_else(|| anyhow::anyhow!("Failed to get image processing pipeline"))?;

        let bind_group_layout = self.context.get_image_processing_bind_group_layout()
            .ok_or_else(|| anyhow::anyhow!("Failed to get bind group layout"))?;

        // Utwórz uniform buffer z parametrami
        let params = ImageProcessingParams {
            width,
            height,
            _padding: [0; 2],
        };

        let uniform_buffer = self.context.get_or_create_buffer(
            std::mem::size_of::<ImageProcessingParams>() as u64,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            Some("gpu_processing_uniform"),
        );

        self.context.queue.write_buffer(&uniform_buffer, 0, bytemuck::cast_slice(&[params]));

        // Utwórz bind group
        let bind_group = self.context.device.create_bind_group(&BindGroupDescriptor {
            label: Some("gpu_processing_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: input_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });

        // Wykonaj compute pass
        let mut encoder = self.context.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("gpu_processing_encoder"),
        });

        {
            let mut compute_pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("gpu_processing_pass"),
                timestamp_writes: None,
            });

            compute_pass.set_pipeline(&pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            
            let workgroup_size = 64;
            let num_workgroups = (pixel_count + workgroup_size - 1) / workgroup_size;
            compute_pass.dispatch_workgroups(num_workgroups as u32, 1, 1);
        }

        // Utwórz staging buffer dla odczytu wyników
        let staging_buffer = self.context.device.create_buffer(&BufferDescriptor {
            label: Some("gpu_processing_staging"),
            size: output_size,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size);

        self.context.queue.submit(Some(encoder.finish()));

        // Odczytaj wyniki z timeout (wzorowane na image_cache.rs)
        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
        
        // Użyj timeout zamiast polling - zgodnie z wzorcem z image_cache.rs
        let recv_result = receiver.recv_timeout(std::time::Duration::from_secs(5));
        match recv_result {
            Ok(Ok(_)) => {
                // Buffer mapping successful
            },
            Ok(Err(e)) => {
                return Err(anyhow::anyhow!("GPU buffer mapping failed: {:?}", e));
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(anyhow::anyhow!("GPU buffer mapping timeout after 5 seconds"));
            },
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow::anyhow!("GPU buffer mapping callback channel disconnected"));
            },
        }
        
        let data = buffer_slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();

        // Zwróć buffery do pool
        self.context.return_buffer(input_buffer, input_size, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        self.context.return_buffer(output_buffer, output_size, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        self.context.return_buffer(uniform_buffer, std::mem::size_of::<ImageProcessingParams>() as u64, BufferUsages::UNIFORM | BufferUsages::COPY_DST);

        Ok(result)
    }
}

/// Parametry dla przetwarzania obrazów na GPU
#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct ImageProcessingParams {
    width: u32,
    height: u32,
    _padding: [u32; 2],
}

unsafe impl bytemuck::Pod for ImageProcessingParams {}
unsafe impl bytemuck::Zeroable for ImageProcessingParams {}