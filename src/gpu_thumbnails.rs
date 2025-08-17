use crate::gpu_context::GpuContext;
use anyhow::Result;
use wgpu::{ComputePipeline, Buffer};
use wgpu::util::DeviceExt;

/// GPU compute shader dla przetwarzania miniaturek
const THUMBNAIL_COMPUTE_SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> input_pixels: array<f32>;
@group(0) @binding(1) var<storage, read_write> output_pixels: array<u32>;
@group(0) @binding(2) var<uniform> params: ThumbnailParams;

struct ThumbnailParams {
    exposure: f32,
    gamma: f32,
    tonemap_mode: u32,
    input_width: u32,
    input_height: u32,
    output_width: u32,
    output_height: u32,
    color_matrix: mat3x3<f32>,
    has_color_matrix: u32,
}

@compute @workgroup_size(8, 8, 1)
fn process_thumbnail(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let x = global_id.x;
    let y = global_id.y;
    
    if (x >= params.output_width || y >= params.output_height) {
        return;
    }
    
    // Oblicz współczynniki skalowania
    let scale_x = f32(params.input_width) / f32(params.output_width);
    let scale_y = f32(params.input_height) / f32(params.output_height);

    // Współrzędne centralnego piksela w obrazie źródłowym
    let src_cx = (f32(x) + 0.5) * scale_x;
    let src_cy = (f32(y) + 0.5) * scale_y;

    // Współrzędne dla 4 próbek (2x2) wokół centrum
    let x0 = u32(floor(src_cx - 0.5));
    let x1 = x0 + 1u;
    let y0 = u32(floor(src_cy - 0.5));
    let y1 = y0 + 1u;

    // Funkcja pomocnicza do bezpiecznego pobierania piksela
    fn get_pixel(px: u32, py: u32) -> vec4<f32> {
        let safe_x = min(px, params.input_width - 1u);
        let safe_y = min(py, params.input_height - 1u);
        let idx = (safe_y * params.input_width + safe_x) * 4u;
        return vec4<f32>(input_pixels[idx], input_pixels[idx+1], input_pixels[idx+2], input_pixels[idx+3]);
    }

    // Pobierz 4 piksele i uśrednij
    let p00 = get_pixel(x0, y0);
    let p10 = get_pixel(x1, y0);
    let p01 = get_pixel(x0, y1);
    let p11 = get_pixel(x1, y1);

    let pixel = (p00 + p10 + p01 + p11) * 0.25;

    let r = pixel.x;
    let g = pixel.y;
    let b = pixel.z;
    let a = pixel.w;
    
    var final_r = r;
    var final_g = g;
    var final_b = b;
    
    // Zastosuj macierz kolorów jeśli dostępna
    if (params.has_color_matrix != 0u) {
        let color_vec = vec3<f32>(r, g, b);
        let transformed = params.color_matrix * color_vec;
        final_r = transformed.x;
        final_g = transformed.y;
        final_b = transformed.z;
    }
    
    // Zastosuj ekspozycję PRZED tone mappingiem (jak w CPU)
    let exposure_mult = pow(2.0, params.exposure);
    let exposed_r = final_r * exposure_mult;
    let exposed_g = final_g * exposure_mult;
    let exposed_b = final_b * exposure_mult;
    
    // Tone mapping
    var mapped_r = exposed_r;
    var mapped_g = exposed_g;
    var mapped_b = exposed_b;
    
    if (params.tonemap_mode == 0u) {
        // ACES
        mapped_r = aces_tonemap(exposed_r);
        mapped_g = aces_tonemap(exposed_g);
        mapped_b = aces_tonemap(exposed_b);
    } else if (params.tonemap_mode == 1u) {
        // Reinhard
        mapped_r = reinhard_tonemap(exposed_r);
        mapped_g = reinhard_tonemap(exposed_g);
        mapped_b = reinhard_tonemap(exposed_b);
    } else if (params.tonemap_mode == 2u) {
        // Linear: tylko clamp do [0,1] po ekspozycji
        mapped_r = clamp(exposed_r, 0.0, 1.0);
        mapped_g = clamp(exposed_g, 0.0, 1.0);
        mapped_b = clamp(exposed_b, 0.0, 1.0);
    }
    
    // Gamma correction
    let inv_gamma = 1.0 / params.gamma;
    mapped_r = pow(mapped_r, inv_gamma);
    mapped_g = pow(mapped_g, inv_gamma);
    mapped_b = pow(mapped_b, inv_gamma);
    
    // Clamp do [0, 1]
    mapped_r = clamp(mapped_r, 0.0, 1.0);
    mapped_g = clamp(mapped_g, 0.0, 1.0);
    mapped_b = clamp(mapped_b, 0.0, 1.0);
    
    // Konwertuj do RGBA8
    let r8 = u32(mapped_r * 255.0);
    let g8 = u32(mapped_g * 255.0);
    let b8 = u32(mapped_b * 255.0);
    let a8 = u32(a * 255.0);
    
    // Pakuj do u32 (RGBA8)
    let rgba = (a8 << 24u) | (b8 << 16u) | (g8 << 8u) | r8;
    
    // Indeks w buforze wyjściowym
    let dst_idx = y * params.output_width + x;
    output_pixels[dst_idx] = rgba;
}

fn aces_tonemap(x: f32) -> f32 {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

fn reinhard_tonemap(x: f32) -> f32 {
    return x / (1.0 + x);
}
"#;

/// Struktura dla GPU processing miniaturek
#[allow(dead_code)]
pub struct GpuThumbnailProcessor {
    gpu_context: GpuContext,
    compute_pipeline: ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    // Nowe pola do reużywalnych buforów
    input_buffer: Option<Buffer>,
    output_buffer: Option<Buffer>,
    uniform_buffer: Option<Buffer>,
    staging_buffer: Option<Buffer>,
}

impl GpuThumbnailProcessor {
    /// Tworzy nowy procesor GPU dla miniaturek
    #[allow(dead_code)]
    pub fn new(gpu_context: GpuContext) -> Result<Self> {
        let device = &gpu_context.device;
        
        // Utwórz shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Thumbnail Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(THUMBNAIL_COMPUTE_SHADER.into()),
        });
        
        // Layout bind group
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Thumbnail Bind Group Layout"),
            entries: &[
                // Input pixels buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Output pixels buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Uniform parameters
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        
        // Pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Thumbnail Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        
        // Compute pipeline
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Thumbnail Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("process_thumbnail"),
            cache: None,
            compilation_options: Default::default(),
        });
        
        Ok(Self {
            gpu_context,
            compute_pipeline,
            bind_group_layout,
            // Zainicjalizuj bufory
            input_buffer: None,
            output_buffer: None,
            uniform_buffer: None,
            staging_buffer: None,
        })
    }
    
    /// Przetwarza miniaturkę na GPU
    #[allow(dead_code)]
    pub fn process_thumbnail(
        &mut self,
        input_pixels: &[f32],
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        exposure: f32,
        gamma: f32,
        tonemap_mode: u32,
        color_matrix: Option<[[f32; 3]; 3]>,
    ) -> Result<Vec<u32>> {
        let device = &self.gpu_context.device;
        let queue = &self.gpu_context.queue;
        
        // Parametry uniform
        #[repr(C)]
        #[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy)]
        struct ThumbnailParams {
            exposure: f32,
            gamma: f32,
            tonemap_mode: u32,
            input_width: u32,
            input_height: u32,
            output_width: u32,
            output_height: u32,
            color_matrix: [[f32; 3]; 3],
            has_color_matrix: u32,
        }
        
        let params = ThumbnailParams {
            exposure,
            gamma,
            tonemap_mode,
            input_width,
            input_height,
            output_width,
            output_height,
            color_matrix: color_matrix.unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            has_color_matrix: if color_matrix.is_some() { 1 } else { 0 },
        };
        
        // Sprawdź czy istniejące bufory są wystarczająco duże dla bieżącego zadania
        let input_size = input_pixels.len() as u64 * std::mem::size_of::<f32>() as u64;
        let output_size = output_width as u64 * output_height as u64 * std::mem::size_of::<u32>() as u64;
        
        // Sprawdź i utwórz/odtwórz bufor wejściowy
        let input_buffer = if let Some(ref buf) = self.input_buffer {
            if buf.size() >= input_size {
                // Reużyj istniejący bufor, ale zaktualizuj dane
                queue.write_buffer(buf, 0, bytemuck::cast_slice(input_pixels));
                buf
            } else {
                let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Input Pixels Buffer"),
                    contents: bytemuck::cast_slice(input_pixels),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                });
                self.input_buffer = Some(new_buffer.clone());
                &self.input_buffer.as_ref().unwrap()
            }
        } else {
            let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Input Pixels Buffer"),
                contents: bytemuck::cast_slice(input_pixels),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            self.input_buffer = Some(new_buffer.clone());
            &self.input_buffer.as_ref().unwrap()
        };
        
        // Sprawdź i utwórz/odtwórz bufor wyjściowy
        let output_buffer = if let Some(ref buf) = self.output_buffer {
            if buf.size() >= output_size {
                buf
            } else {
                let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Output Pixels Buffer"),
                    size: output_size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                self.output_buffer = Some(new_buffer.clone());
                &self.output_buffer.as_ref().unwrap()
            }
        } else {
            let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Output Pixels Buffer"),
                size: output_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            self.output_buffer = Some(new_buffer.clone());
            &self.output_buffer.as_ref().unwrap()
        };
        
        // Sprawdź i utwórz/odtwórz bufor uniformów
        let uniform_buffer = if let Some(ref buf) = self.uniform_buffer {
            // Uniform buffer zawsze ma ten sam rozmiar, więc możemy go reużyć
            // Zaktualizuj zawartość
            queue.write_buffer(buf, 0, bytemuck::cast_slice(&[params]));
            buf
        } else {
            let new_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Thumbnail Params Buffer"),
                contents: bytemuck::cast_slice(&[params]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            self.uniform_buffer = Some(new_buffer.clone());
            &self.uniform_buffer.as_ref().unwrap()
        };
        
        // Bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Thumbnail Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
        
        // Command encoder
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Thumbnail Command Encoder"),
        });
        
        // Compute pass
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Thumbnail Compute Pass"),
                timestamp_writes: None,
            });
            
            compute_pass.set_pipeline(&self.compute_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            
            // Oblicz liczbę grup roboczych
            let workgroup_size = 8;
            let workgroups_x = (output_width + workgroup_size - 1) / workgroup_size;
            let workgroups_y = (output_height + workgroup_size - 1) / workgroup_size;
            
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }
        
        // Sprawdź i utwórz/odtwórz staging buffer
        let staging_buffer = if let Some(ref buf) = self.staging_buffer {
            if buf.size() >= output_size {
                buf
            } else {
                let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Thumbnail Staging Buffer"),
                    size: output_size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.staging_buffer = Some(new_buffer.clone());
                &self.staging_buffer.as_ref().unwrap()
            }
        } else {
            let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Thumbnail Staging Buffer"),
                size: output_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.staging_buffer = Some(new_buffer.clone());
            &self.staging_buffer.as_ref().unwrap()
        };
        
        // Kopiuj wyniki do staging buffer
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, output_size);
        
        // Wykonaj komendy
        queue.submit(std::iter::once(encoder.finish()));
        
        // Synchronizuj i odczytaj wyniki
        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        
        let _ = device.poll(wgpu::PollType::Wait);
        let _ = rx.recv()??;
        
        // Odczytaj dane
        let data = buffer_slice.get_mapped_range();
        let result: Vec<u32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();
        
        Ok(result)
    }
}
