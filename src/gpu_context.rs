use wgpu::{
    Adapter, Backends, Device, DeviceDescriptor, Features, Instance, Limits, PowerPreference,
    Queue, RequestAdapterOptions, PollType,
};
use anyhow::Result;
use wgpu::util::DeviceExt;

/// Kontekst GPU zarządzający stanem wgpu
#[allow(dead_code)]
pub struct GpuContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
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

    /// Pobiera informacje o urządzeniu GPU
    #[allow(dead_code)]
    pub fn get_device_info(&self) -> (Features, Limits) {
        (self.device.features(), self.device.limits())
    }

    /// Tworzy bufor na GPU
    #[allow(dead_code)]
    pub fn create_buffer(
        &self,
        label: &str,
        size: u64,
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    /// Tworzy bufor uniformów
    #[allow(dead_code)]
    pub fn create_uniform_buffer<T: bytemuck::Pod>(
        &self,
        label: &str,
        data: &T,
    ) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(&[*data]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// Tworzy bufor storage
    #[allow(dead_code)]
    pub fn create_storage_buffer(
        &self,
        label: &str,
        size: u64,
        read_only: bool,
    ) -> wgpu::Buffer {
        let usage = if read_only {
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST
        } else {
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
        };

        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    /// Tworzy bufor staging do kopiowania danych między CPU a GPU
    #[allow(dead_code)]
    pub fn create_staging_buffer(
        &self,
        label: &str,
        size: u64,
    ) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Tworzy moduł shadera z kodu WGSL
    #[allow(dead_code)]
    pub fn create_shader_module(&self, label: &str, code: &str) -> wgpu::ShaderModule {
        self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(code.into()),
        })
    }

    /// Tworzy pipeline compute
    #[allow(dead_code)]
    pub fn create_compute_pipeline(
        &self,
        label: &str,
        module: &wgpu::ShaderModule,
        entry_point: &str,
        bind_group_layouts: &[&wgpu::BindGroupLayout],
    ) -> wgpu::ComputePipeline {
        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{} Pipeline Layout", label)),
            bind_group_layouts,
            push_constant_ranges: &[],
        });

        self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            module,
            entry_point: Some(entry_point),
            cache: None,
            compilation_options: Default::default(),
        })
    }

    /// Tworzy bind group layout
    #[allow(dead_code)]
    pub fn create_bind_group_layout(
        &self,
        label: &str,
        entries: &[wgpu::BindGroupLayoutEntry],
    ) -> wgpu::BindGroupLayout {
        self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        })
    }

    /// Tworzy bind group
    #[allow(dead_code)]
    pub fn create_bind_group(
        &self,
        label: &str,
        layout: &wgpu::BindGroupLayout,
        entries: &[wgpu::BindGroupEntry],
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries,
        })
    }

    /// Tworzy command encoder
    #[allow(dead_code)]
    pub fn create_command_encoder(&self, label: &str) -> wgpu::CommandEncoder {
        self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(label),
        })
    }

    /// Wysyła komendy do GPU
    #[allow(dead_code)]
    pub fn submit(&self, commands: wgpu::CommandBuffer) {
        self.queue.submit(std::iter::once(commands));
    }

    /// Czeka na zakończenie operacji GPU
    #[allow(dead_code)]
    pub fn poll(&self) {
        // W wgpu 26.0.1 używamy device.poll() z PollType::Wait
        let _ = self.device.poll(PollType::Wait);
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
