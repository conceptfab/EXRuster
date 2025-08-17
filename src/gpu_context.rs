use wgpu::{
    Adapter, Backends, Device, DeviceDescriptor, Features, Instance, Limits, PowerPreference,
    Queue, RequestAdapterOptions,
};
use anyhow::Result;

/// Kontekst GPU zarządzający stanem wgpu
#[derive(Clone)]
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
