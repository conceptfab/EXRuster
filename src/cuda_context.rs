// CUDA Context Management for EXRuster
// Real CUDA implementation using cudarc

use std::sync::{Arc, Mutex};
use anyhow::Result;
use cudarc::driver::{CudaDevice, result};
use std::collections::HashMap;

/// CUDA Context for GPU-accelerated image processing
#[derive(Clone)]
pub struct CudaContext {
    device: Arc<CudaDevice>,
    device_id: i32,
    device_info: CudaDeviceInfo,
    memory_pools: Mutex<HashMap<usize, Vec<cudarc::driver::CudaSlice<u8>>>>,
}

impl CudaContext {
    /// Create a new CUDA context
    pub async fn new() -> Result<Self> {
        println!("CUDA: Initializing CUDA context...");
        
        // Initialize CUDA driver
        result::init().map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to initialize CUDA driver: {:?}", e)
        })?;
        
        // Get device count
        let device_count = Self::get_device_count()?;
        if device_count == 0 {
            anyhow::bail!("CUDA: No CUDA devices found");
        }
        
        println!("CUDA: Found {} CUDA device(s)", device_count);
        
        // Use first available device
        let device_id = 0;
        let device = CudaDevice::new(device_id).map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to create device {}: {:?}", device_id, e)
        })?;
        
        // Get device properties
        let device_info = Self::get_device_info_impl(&device, device_id as i32)?;
        
        println!("CUDA: Successfully initialized device: {}", device_info.name);
        println!("CUDA: Compute capability: {}.{}", device_info.compute_capability.0, device_info.compute_capability.1);
        println!("CUDA: Total memory: {} MB", device_info.memory_mb);
        
        Ok(CudaContext {
            device,
            device_id: device_id as i32,
            device_info,
            memory_pools: Mutex::new(HashMap::new()),
        })
    }
    
    /// Get the number of CUDA devices
    fn get_device_count() -> Result<i32> {
        result::device_count().map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to get device count: {:?}", e)
        })
    }
    
    /// Get device information implementation
    fn get_device_info_impl(device: &CudaDevice, device_id: i32) -> Result<CudaDeviceInfo> {
        let name = device.name().map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to get device name: {:?}", e)
        })?;
        
        let (major, minor) = device.compute_capability().map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to get compute capability: {:?}", e)
        })?;
        
        let memory_bytes = device.total_memory().map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to get total memory: {:?}", e)
        })?;
        
        Ok(CudaDeviceInfo {
            name,
            compute_capability: (major as i32, minor as i32),
            memory_mb: memory_bytes / (1024 * 1024),
        })
    }
    
    /// Get device information
    pub fn get_device_info(&self) -> CudaDeviceInfo {
        self.device_info.clone()
    }
    
    /// Check if CUDA context is available
    pub fn is_available(&self) -> bool {
        // Check if we can allocate a small test buffer
        match self.device.alloc_zeros::<u8>(1024) {
            Ok(_) => true,
            Err(_) => false,
        }
    }
    
    /// Get the CUDA device
    pub fn device(&self) -> &Arc<CudaDevice> {
        &self.device
    }
    
    /// Allocate GPU memory with pooling
    pub fn alloc_or_reuse<T: cudarc::driver::DeviceRepr>(&self, len: usize) -> Result<cudarc::driver::CudaSlice<T>> {
        self.device.alloc_zeros(len).map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to allocate {} elements: {:?}", len, e)
        })
    }
    
    /// Synchronize device
    pub fn synchronize(&self) -> Result<()> {
        self.device.synchronize().map_err(|e| {
            anyhow::anyhow!("CUDA: Synchronization failed: {:?}", e)
        })
    }
}

/// CUDA Device Information
#[derive(Debug, Clone)]
pub struct CudaDeviceInfo {
    pub name: String,
    pub compute_capability: (i32, i32),
    pub memory_mb: u64,
}

/// Global CUDA context type
pub type CudaContextType = Arc<Mutex<Option<CudaContext>>>;