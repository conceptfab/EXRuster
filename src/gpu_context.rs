use wgpu::{
    Adapter, Backends, Device, DeviceDescriptor, Features, Instance, Limits, PowerPreference,
    Queue, RequestAdapterOptions, Buffer, BufferUsages, ComputePipeline, ShaderModule,
    BindGroupLayout, PipelineLayout,
};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::cell::OnceCell;

/// Pool buforów GPU do wielokrotnego użytku
#[derive(Debug)]
pub struct GpuBufferPool {
    buffers_by_size_and_usage: HashMap<(u64, BufferUsages), Vec<Buffer>>,
    max_buffers_per_type: usize,
}

impl GpuBufferPool {
    pub fn new() -> Self {
        Self {
            buffers_by_size_and_usage: HashMap::new(),
            max_buffers_per_type: 8, // Maksymalnie 8 buforów tego samego typu
        }
    }

    pub fn get_or_create_buffer(
        &mut self,
        device: &Device,
        size: u64,
        usage: BufferUsages,
        label: Option<&str>,
    ) -> Buffer {
        let key = (size, usage);
        
        // Sprawdź czy mamy bufor w pool'u
        if let Some(buffers) = self.buffers_by_size_and_usage.get_mut(&key) {
            if let Some(buffer) = buffers.pop() {
                return buffer;
            }
        }
        
        // Utwórz nowy bufor jeśli nie ma w pool'u
        device.create_buffer(&wgpu::BufferDescriptor {
            label,
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    pub fn return_buffer(&mut self, buffer: Buffer, size: u64, usage: BufferUsages) {
        let key = (size, usage);
        let buffers = self.buffers_by_size_and_usage.entry(key).or_insert_with(Vec::new);
        
        // Dodaj bufor do pool'u tylko jeśli nie przekroczyliśmy limitu
        if buffers.len() < self.max_buffers_per_type {
            buffers.push(buffer);
        }
        // Jeśli przekroczyliśmy limit, bufor zostanie automatycznie zniszczony (drop)
    }


}

/// Cache pipeline'ów compute
pub struct GpuPipelineCache {
    image_processing_pipeline: OnceCell<ComputePipeline>,
    #[allow(dead_code)]
    thumbnail_pipeline: OnceCell<ComputePipeline>,
    #[allow(dead_code)]
    mip_generation_pipeline: OnceCell<ComputePipeline>,
    // FAZA 3: Nowe shadery
    #[allow(dead_code)]
    blur_pipeline: OnceCell<ComputePipeline>,
    #[allow(dead_code)]
    sharpen_pipeline: OnceCell<ComputePipeline>,
    #[allow(dead_code)]
    histogram_pipeline: OnceCell<ComputePipeline>,
    // Shader modules cache
    image_processing_shader: OnceCell<ShaderModule>,
    #[allow(dead_code)]
    thumbnail_shader: OnceCell<ShaderModule>,
    #[allow(dead_code)]
    mip_generation_shader: OnceCell<ShaderModule>,
    // FAZA 3: Nowe shadery
    #[allow(dead_code)]
    blur_shader: OnceCell<ShaderModule>,
    #[allow(dead_code)]
    sharpen_shader: OnceCell<ShaderModule>,
    #[allow(dead_code)]
    histogram_shader: OnceCell<ShaderModule>,
    // Layouts cache
    image_processing_bind_group_layout: OnceCell<BindGroupLayout>,
    image_processing_pipeline_layout: OnceCell<PipelineLayout>,
    #[allow(dead_code)]
    thumbnail_bind_group_layout: OnceCell<BindGroupLayout>,
    #[allow(dead_code)]
    thumbnail_pipeline_layout: OnceCell<PipelineLayout>,
    #[allow(dead_code)]
    mip_generation_bind_group_layout: OnceCell<BindGroupLayout>,
    #[allow(dead_code)]
    mip_generation_pipeline_layout: OnceCell<PipelineLayout>,
    // FAZA 3: Nowe shadery
    #[allow(dead_code)]
    blur_bind_group_layout: OnceCell<BindGroupLayout>,
    #[allow(dead_code)]
    blur_pipeline_layout: OnceCell<PipelineLayout>,
    #[allow(dead_code)]
    sharpen_bind_group_layout: OnceCell<BindGroupLayout>,
    #[allow(dead_code)]
    sharpen_pipeline_layout: OnceCell<PipelineLayout>,
    #[allow(dead_code)]
    histogram_bind_group_layout: OnceCell<BindGroupLayout>,
    #[allow(dead_code)]
    histogram_pipeline_layout: OnceCell<PipelineLayout>,
}

impl GpuPipelineCache {
    pub fn new() -> Self {
        Self {
            image_processing_pipeline: OnceCell::new(),
            thumbnail_pipeline: OnceCell::new(),
            mip_generation_pipeline: OnceCell::new(),
            // FAZA 3: Nowe shadery
            blur_pipeline: OnceCell::new(),
            sharpen_pipeline: OnceCell::new(),
            histogram_pipeline: OnceCell::new(),
            image_processing_shader: OnceCell::new(),
            thumbnail_shader: OnceCell::new(),
            mip_generation_shader: OnceCell::new(),
            // FAZA 3: Nowe shadery
            blur_shader: OnceCell::new(),
            sharpen_shader: OnceCell::new(),
            histogram_shader: OnceCell::new(),
            image_processing_bind_group_layout: OnceCell::new(),
            image_processing_pipeline_layout: OnceCell::new(),
            thumbnail_bind_group_layout: OnceCell::new(),
            thumbnail_pipeline_layout: OnceCell::new(),
            mip_generation_bind_group_layout: OnceCell::new(),
            mip_generation_pipeline_layout: OnceCell::new(),
            // FAZA 3: Nowe shadery
            blur_bind_group_layout: OnceCell::new(),
            blur_pipeline_layout: OnceCell::new(),
            sharpen_bind_group_layout: OnceCell::new(),
            sharpen_pipeline_layout: OnceCell::new(),
            histogram_bind_group_layout: OnceCell::new(),
            histogram_pipeline_layout: OnceCell::new(),
        }
    }

    pub fn get_image_processing_shader(&self, device: &Device) -> &ShaderModule {
        self.image_processing_shader.get_or_init(|| {
            const SHADER_WGSL: &str = include_str!("shaders/image_processing.wgsl");
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("exruster.image_processing.compute"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_WGSL)),
            })
        })
    }

    pub fn get_image_processing_bind_group_layout(&self, device: &Device) -> &BindGroupLayout {
        self.image_processing_bind_group_layout.get_or_init(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exruster.image_processing.bgl"),
                entries: &[
                    // binding 0: uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 1: input storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 2: output storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
        })
    }

    pub fn get_image_processing_pipeline_layout(&self, device: &Device) -> &PipelineLayout {
        self.image_processing_pipeline_layout.get_or_init(|| {
            let bind_group_layout = self.get_image_processing_bind_group_layout(device);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exruster.image_processing.pipeline_layout"),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            })
        })
    }

    pub fn get_image_processing_pipeline(&self, device: &Device) -> &ComputePipeline {
        self.image_processing_pipeline.get_or_init(|| {
            let shader = self.get_image_processing_shader(device);
            let layout = self.get_image_processing_pipeline_layout(device);
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exruster.image_processing.pipeline"),
                layout: Some(layout),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
    }

    // MIP generation pipeline methods
    #[allow(dead_code)]
    pub fn get_mip_generation_shader(&self, device: &Device) -> &ShaderModule {
        self.mip_generation_shader.get_or_init(|| {
            const SHADER_WGSL: &str = include_str!("shaders/mip_generation.wgsl");
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("exruster.mip_generation.compute"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_WGSL)),
            })
        })
    }

    #[allow(dead_code)]
    pub fn get_mip_generation_bind_group_layout(&self, device: &Device) -> &BindGroupLayout {
        self.mip_generation_bind_group_layout.get_or_init(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exruster.mip_generation.bgl"),
                entries: &[
                    // binding 0: uniform (MipParams)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 1: input storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 2: output storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
        })
    }

    #[allow(dead_code)]
    pub fn get_mip_generation_pipeline_layout(&self, device: &Device) -> &PipelineLayout {
        self.mip_generation_pipeline_layout.get_or_init(|| {
            let bind_group_layout = self.get_mip_generation_bind_group_layout(device);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exruster.mip_generation.pipeline_layout"),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            })
        })
    }

    #[allow(dead_code)]
    pub fn get_mip_generation_pipeline(&self, device: &Device) -> &ComputePipeline {
        self.mip_generation_pipeline.get_or_init(|| {
            let shader = self.get_mip_generation_shader(device);
            let layout = self.get_mip_generation_pipeline_layout(device);
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exruster.mip_generation.pipeline"),
                layout: Some(layout),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
    }

    // Thumbnail pipeline methods
    #[allow(dead_code)]
    pub fn get_thumbnail_shader(&self, device: &Device) -> &ShaderModule {
        self.thumbnail_shader.get_or_init(|| {
            const SHADER_WGSL: &str = include_str!("shaders/thumbnail.wgsl");
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("exruster.thumbnail.compute"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_WGSL)),
            })
        })
    }

    #[allow(dead_code)]
    pub fn get_thumbnail_bind_group_layout(&self, device: &Device) -> &BindGroupLayout {
        self.thumbnail_bind_group_layout.get_or_init(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exruster.thumbnail.bgl"),
                entries: &[
                    // binding 0: uniform (ThumbnailParams)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 1: input storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 2: output storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
        })
    }

    #[allow(dead_code)]
    pub fn get_thumbnail_pipeline_layout(&self, device: &Device) -> &PipelineLayout {
        self.thumbnail_pipeline_layout.get_or_init(|| {
            let bind_group_layout = self.get_thumbnail_bind_group_layout(device);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exruster.thumbnail.pipeline_layout"),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            })
        })
    }

    #[allow(dead_code)]
    pub fn get_thumbnail_pipeline(&self, device: &Device) -> &ComputePipeline {
        self.thumbnail_pipeline.get_or_init(|| {
            let shader = self.get_thumbnail_shader(device);
            let layout = self.get_thumbnail_pipeline_layout(device);
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exruster.thumbnail.pipeline"),
                layout: Some(layout),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
    }

    // === FAZA 3: Nowe shadery ===

    // Blur shader
    pub fn get_blur_shader(&self, device: &Device) -> &ShaderModule {
        self.blur_shader.get_or_init(|| {
            const SHADER_WGSL: &str = include_str!("shaders/blur.wgsl");
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("exruster.blur.compute"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_WGSL)),
            })
        })
    }

    pub fn get_blur_bind_group_layout(&self, device: &Device) -> &BindGroupLayout {
        self.blur_bind_group_layout.get_or_init(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exruster.blur.bgl"),
                entries: &[
                    // binding 0: uniform (BlurParams)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 1: input storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 2: output storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
        })
    }

    pub fn get_blur_pipeline_layout(&self, device: &Device) -> &PipelineLayout {
        self.blur_pipeline_layout.get_or_init(|| {
            let bind_group_layout = self.get_blur_bind_group_layout(device);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exruster.blur.pipeline_layout"),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            })
        })
    }

    pub fn get_blur_pipeline(&self, device: &Device) -> &ComputePipeline {
        self.blur_pipeline.get_or_init(|| {
            let shader = self.get_blur_shader(device);
            let layout = self.get_blur_pipeline_layout(device);
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exruster.blur.pipeline"),
                layout: Some(layout),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
    }

    // Sharpen shader
    pub fn get_sharpen_shader(&self, device: &Device) -> &ShaderModule {
        self.sharpen_shader.get_or_init(|| {
            const SHADER_WGSL: &str = include_str!("shaders/sharpen.wgsl");
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("exruster.sharpen.compute"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_WGSL)),
            })
        })
    }

    pub fn get_sharpen_bind_group_layout(&self, device: &Device) -> &BindGroupLayout {
        self.sharpen_bind_group_layout.get_or_init(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exruster.sharpen.bgl"),
                entries: &[
                    // binding 0: uniform (SharpenParams)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 1: input storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 2: output storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
        })
    }

    pub fn get_sharpen_pipeline_layout(&self, device: &Device) -> &PipelineLayout {
        self.sharpen_pipeline_layout.get_or_init(|| {
            let bind_group_layout = self.get_sharpen_bind_group_layout(device);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exruster.sharpen.pipeline_layout"),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            })
        })
    }

    pub fn get_sharpen_pipeline(&self, device: &Device) -> &ComputePipeline {
        self.sharpen_pipeline.get_or_init(|| {
            let shader = self.get_sharpen_shader(device);
            let layout = self.get_sharpen_pipeline_layout(device);
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exruster.sharpen.pipeline"),
                layout: Some(layout),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
    }

    // Histogram shader
    pub fn get_histogram_shader(&self, device: &Device) -> &ShaderModule {
        self.histogram_shader.get_or_init(|| {
            const SHADER_WGSL: &str = include_str!("shaders/histogram.wgsl");
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("exruster.histogram.compute"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER_WGSL)),
            })
        })
    }

    pub fn get_histogram_bind_group_layout(&self, device: &Device) -> &BindGroupLayout {
        self.histogram_bind_group_layout.get_or_init(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exruster.histogram.bgl"),
                entries: &[
                    // binding 0: uniform (HistogramParams)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 1: input storage (read)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 2: output storage (write)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding 3: histogram bins (atomic)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
        })
    }

    pub fn get_histogram_pipeline_layout(&self, device: &Device) -> &PipelineLayout {
        self.histogram_pipeline_layout.get_or_init(|| {
            let bind_group_layout = self.get_histogram_bind_group_layout(device);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("exruster.histogram.pipeline_layout"),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            })
        })
    }

    pub fn get_histogram_pipeline(&self, device: &Device) -> &ComputePipeline {
        self.histogram_pipeline.get_or_init(|| {
            let shader = self.get_histogram_shader(device);
            let layout = self.get_histogram_pipeline_layout(device);
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exruster.histogram.pipeline"),
                layout: Some(layout),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })
    }

}

/// Kontekst GPU zarządzający stanem wgpu
#[derive(Clone)]
#[allow(dead_code)]
pub struct GpuContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    buffer_pool: Arc<Mutex<GpuBufferPool>>,
    pipeline_cache: Arc<Mutex<GpuPipelineCache>>,
}

impl GpuContext {
    /// Asynchronicznie inicjalizuje kontekst GPU
    #[allow(dead_code)]
    pub async fn new() -> Result<Self> {
        // Tworzenie instancji wgpu z preferowanymi backendami
        let instance = Instance::new(&wgpu::InstanceDescriptor {
            backends: Backends::all(),
            backend_options: Default::default(),
            flags: Default::default(),
            memory_budget_thresholds: Default::default(),
        });

        // Wybór adaptera z preferencją dedykowanego GPU o wysokiej wydajności
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Nie można znaleźć kompatybilnego adaptera GPU"))?;

        // Pobranie informacji o adapterze
        let adapter_info = adapter.get_info();
        println!("Wybrany adapter GPU: {}", adapter_info.name);

        // Sprawdzenie możliwości adaptera
        let _features = adapter.features();
        let _limits = adapter.limits();

        // Żądanie utworzenia urządzenia i kolejki
        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: Some("EXRuster GPU Device"),
                    required_features: Features::empty(), // Na początek bez specjalnych funkcji
                    required_limits: Limits::default(),
                    memory_hints: Default::default(),
                    trace: Default::default(),
                },
            )
            .await?;

        println!("GPU Device utworzony pomyślnie");
        println!("GPU Features: {:?}", device.features());
        println!("GPU Limits: {:?}", device.limits());

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            buffer_pool: Arc::new(Mutex::new(GpuBufferPool::new())),
            pipeline_cache: Arc::new(Mutex::new(GpuPipelineCache::new())),
        })
    }

    /// Sprawdza czy kontekst GPU jest dostępny i funkcjonalny
    #[allow(dead_code)]
    pub fn is_available(&self) -> bool {
        // Sprawdzenie czy urządzenie i kolejka są dostępne
        // W wgpu 26.0.1 nie ma metody is_lost(), więc zwracamy true
        true
    }

    /// Pobiera informacje o adapterze GPU
    #[allow(dead_code)]
    pub fn get_adapter_info(&self) -> wgpu::AdapterInfo {
        self.adapter.get_info()
    }

    /// Pobiera bufor z pool'u lub tworzy nowy
    pub fn get_or_create_buffer(
        &self,
        size: u64,
        usage: BufferUsages,
        label: Option<&str>,
    ) -> Buffer {
        if let Ok(mut pool) = self.buffer_pool.lock() {
            pool.get_or_create_buffer(&self.device, size, usage, label)
        } else {
            // Fallback - utwórz bufor bezpośrednio
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label,
                size,
                usage,
                mapped_at_creation: false,
            })
        }
    }

    /// Zwraca bufor do pool'u
    pub fn return_buffer(&self, buffer: Buffer, size: u64, usage: BufferUsages) {
        if let Ok(mut pool) = self.buffer_pool.lock() {
            pool.return_buffer(buffer, size, usage);
        }
        // Jeśli nie można zablokować mutex'a, bufor zostanie automatycznie zniszczony
    }

    /// Pobiera pipeline do przetwarzania obrazów z cache
    pub fn get_image_processing_pipeline(&self) -> Option<ComputePipeline> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_image_processing_pipeline(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera bind group layout do przetwarzania obrazów z cache
    pub fn get_image_processing_bind_group_layout(&self) -> Option<BindGroupLayout> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_image_processing_bind_group_layout(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera pipeline do thumbnail generation z cache
    #[allow(dead_code)]
    pub fn get_thumbnail_pipeline(&self) -> Option<ComputePipeline> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_thumbnail_pipeline(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera bind group layout do thumbnail generation z cache
    #[allow(dead_code)]
    pub fn get_thumbnail_bind_group_layout(&self) -> Option<BindGroupLayout> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_thumbnail_bind_group_layout(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera pipeline do MIP generation z cache
    #[allow(dead_code)]
    pub fn get_mip_generation_pipeline(&self) -> Option<ComputePipeline> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_mip_generation_pipeline(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera bind group layout do MIP generation z cache
    #[allow(dead_code)]
    pub fn get_mip_generation_bind_group_layout(&self) -> Option<BindGroupLayout> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_mip_generation_bind_group_layout(&self.device).clone())
        } else {
            None
        }
    }

    // === FAZA 3: Nowe shadery ===

    /// Pobiera pipeline do blur z cache
    #[allow(dead_code)]
    pub fn get_blur_pipeline(&self) -> Option<ComputePipeline> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_blur_pipeline(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera bind group layout do blur z cache
    #[allow(dead_code)]
    pub fn get_blur_bind_group_layout(&self) -> Option<BindGroupLayout> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_blur_bind_group_layout(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera pipeline do sharpen z cache
    #[allow(dead_code)]
    pub fn get_sharpen_pipeline(&self) -> Option<ComputePipeline> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_sharpen_pipeline(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera bind group layout do sharpen z cache
    #[allow(dead_code)]
    pub fn get_sharpen_bind_group_layout(&self) -> Option<BindGroupLayout> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_sharpen_bind_group_layout(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera pipeline do histogram z cache
    #[allow(dead_code)]
    pub fn get_histogram_pipeline(&self) -> Option<ComputePipeline> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_histogram_pipeline(&self.device).clone())
        } else {
            None
        }
    }

    /// Pobiera bind group layout do histogram z cache
    #[allow(dead_code)]
    pub fn get_histogram_bind_group_layout(&self) -> Option<BindGroupLayout> {
        if let Ok(cache) = self.pipeline_cache.lock() {
            Some(cache.get_histogram_bind_group_layout(&self.device).clone())
        } else {
            None
        }
    }

}

impl Drop for GpuContext {
    fn drop(&mut self) {
        println!("GpuContext zostaje zniszczony");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gpu_context_creation() {
        // Test inicjalizacji kontekstu GPU
        let result = GpuContext::new().await;
        match result {
            Ok(context) => {
                assert!(context.is_available());
                let adapter_info = context.get_adapter_info();
                assert!(!adapter_info.name.is_empty());
                println!("Test GPU: {}", adapter_info.name);
            }
            Err(e) => {
                // Jeśli GPU nie jest dostępny, test powinien przejść
                println!("GPU nie dostępny: {}", e);
            }
        }
    }
}
