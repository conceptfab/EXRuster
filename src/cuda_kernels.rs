// CUDA Kernels for EXRuster Image Processing
// This module contains CUDA kernel implementations for thumbnail generation

use cudarc::driver::{CudaDevice, CudaSlice, LaunchAsync, LaunchConfig};
use cudarc::nvrtc::Ptx;
use anyhow::Result;
use std::sync::Arc;

/// CUDA kernel source code for thumbnail processing
const THUMBNAIL_KERNEL_SRC: &str = r#"
extern "C" __global__ void resize_and_tonemap_rgba_f32_to_u8(
    float* input,           // Input RGBA f32 data
    unsigned char* output,  // Output RGBA u8 data
    int src_width,
    int src_height,
    int dst_width,
    int dst_height,
    float exposure,
    float gamma,
    int tonemap_mode,
    float* color_matrix,    // 3x3 color transform matrix (9 elements)
    int has_color_matrix
) {
    int dst_x = blockIdx.x * blockDim.x + threadIdx.x;
    int dst_y = blockIdx.y * blockDim.y + threadIdx.y;
    
    if (dst_x >= dst_width || dst_y >= dst_height) return;
    
    // Calculate source coordinates with bilinear interpolation
    float src_x_f = (float)dst_x * (float)src_width / (float)dst_width;
    float src_y_f = (float)dst_y * (float)src_height / (float)dst_height;
    
    int src_x = (int)floorf(src_x_f);
    int src_y = (int)floorf(src_y_f);
    
    float fx = src_x_f - src_x;
    float fy = src_y_f - src_y;
    
    // Ensure we don't go out of bounds
    int src_x1 = min(src_x + 1, src_width - 1);
    int src_y1 = min(src_y + 1, src_height - 1);
    src_x = min(src_x, src_width - 1);
    src_y = min(src_y, src_height - 1);
    
    // Bilinear interpolation
    float r, g, b, a;
    
    // Sample 4 pixels
    int idx00 = (src_y * src_width + src_x) * 4;
    int idx01 = (src_y * src_width + src_x1) * 4;
    int idx10 = (src_y1 * src_width + src_x) * 4;
    int idx11 = (src_y1 * src_width + src_x1) * 4;
    
    // Interpolate R channel
    float r00 = input[idx00];
    float r01 = input[idx01];
    float r10 = input[idx10]; 
    float r11 = input[idx11];
    r = r00 * (1-fx) * (1-fy) + r01 * fx * (1-fy) + r10 * (1-fx) * fy + r11 * fx * fy;
    
    // Interpolate G channel
    float g00 = input[idx00 + 1];
    float g01 = input[idx01 + 1];
    float g10 = input[idx10 + 1];
    float g11 = input[idx11 + 1];
    g = g00 * (1-fx) * (1-fy) + g01 * fx * (1-fy) + g10 * (1-fx) * fy + g11 * fx * fy;
    
    // Interpolate B channel
    float b00 = input[idx00 + 2];
    float b01 = input[idx01 + 2];
    float b10 = input[idx10 + 2];
    float b11 = input[idx11 + 2];
    b = b00 * (1-fx) * (1-fy) + b01 * fx * (1-fy) + b10 * (1-fx) * fy + b11 * fx * fy;
    
    // Interpolate A channel
    float a00 = input[idx00 + 3];
    float a01 = input[idx01 + 3];
    float a10 = input[idx10 + 3];
    float a11 = input[idx11 + 3];
    a = a00 * (1-fx) * (1-fy) + a01 * fx * (1-fy) + a10 * (1-fx) * fy + a11 * fx * fy;
    
    // Apply exposure
    r *= exposure;
    g *= exposure;
    b *= exposure;
    
    // Apply color matrix if provided
    if (has_color_matrix) {
        float r_new = color_matrix[0] * r + color_matrix[1] * g + color_matrix[2] * b;
        float g_new = color_matrix[3] * r + color_matrix[4] * g + color_matrix[5] * b;
        float b_new = color_matrix[6] * r + color_matrix[7] * g + color_matrix[8] * b;
        r = r_new;
        g = g_new;
        b = b_new;
    }
    
    // Apply tone mapping
    switch(tonemap_mode) {
        case 0: // ACES
            r = (r * (2.51f * r + 0.03f)) / (r * (2.43f * r + 0.59f) + 0.14f);
            g = (g * (2.51f * g + 0.03f)) / (g * (2.43f * g + 0.59f) + 0.14f);
            b = (b * (2.51f * b + 0.03f)) / (b * (2.43f * b + 0.59f) + 0.14f);
            break;
        case 1: // Reinhard
            r = r / (1.0f + r);
            g = g / (1.0f + g);
            b = b / (1.0f + b);
            break;
        case 2: // Linear (clamp)
            r = fmaxf(0.0f, fminf(1.0f, r));
            g = fmaxf(0.0f, fminf(1.0f, g));
            b = fmaxf(0.0f, fminf(1.0f, b));
            break;
        case 3: // Filmic
            {
                float A = 0.15f, B = 0.50f, C = 0.10f, D = 0.20f, E = 0.02f, F = 0.30f;
                r = ((r * (A * r + C * B) + D * E) / (r * (A * r + B) + D * F)) - E / F;
                g = ((g * (A * g + C * B) + D * E) / (g * (A * g + B) + D * F)) - E / F;
                b = ((b * (A * b + C * B) + D * E) / (b * (A * b + B) + D * F)) - E / F;
            }
            break;
        case 4: // Hable
            {
                float A = 0.22f, B = 0.30f, C = 0.10f, D = 0.20f, E = 0.01f, F = 0.30f;
                r = ((r * (A * r + C * B) + D * E) / (r * (A * r + B) + D * F)) - E / F;
                g = ((g * (A * g + C * B) + D * E) / (g * (A * g + B) + D * F)) - E / F;
                b = ((b * (A * b + C * B) + D * E) / (b * (A * b + B) + D * F)) - E / F;
            }
            break;
        default: // Linear
            break;
    }
    
    // Apply gamma correction
    r = powf(fmaxf(0.0f, r), 1.0f / gamma);
    g = powf(fmaxf(0.0f, g), 1.0f / gamma);
    b = powf(fmaxf(0.0f, b), 1.0f / gamma);
    
    // Clamp and convert to u8
    r = fmaxf(0.0f, fminf(1.0f, r));
    g = fmaxf(0.0f, fminf(1.0f, g));
    b = fmaxf(0.0f, fminf(1.0f, b));
    a = fmaxf(0.0f, fminf(1.0f, a));
    
    int dst_idx = (dst_y * dst_width + dst_x) * 4;
    output[dst_idx] = (unsigned char)(r * 255.0f);
    output[dst_idx + 1] = (unsigned char)(g * 255.0f);
    output[dst_idx + 2] = (unsigned char)(b * 255.0f);
    output[dst_idx + 3] = (unsigned char)(a * 255.0f);
}

extern "C" __global__ void compute_histogram_rgba_f32(
    float* input,
    int* red_bins,
    int* green_bins,
    int* blue_bins,
    int* lum_bins,
    int width,
    int height,
    float min_val,
    float max_val,
    int bin_count
) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;
    
    if (x >= width || y >= height) return;
    
    int idx = (y * width + x) * 4;
    float r = input[idx];
    float g = input[idx + 1];
    float b = input[idx + 2];
    
    // Clamp values to range
    r = fmaxf(min_val, fminf(max_val, r));
    g = fmaxf(min_val, fminf(max_val, g));
    b = fmaxf(min_val, fminf(max_val, b));
    
    float range = max_val - min_val;
    if (range <= 0.0f) return;
    
    // Calculate luminance
    float lum = 0.299f * r + 0.587f * g + 0.114f * b;
    lum = fmaxf(min_val, fminf(max_val, lum));
    
    // Calculate bin indices
    int r_bin = (int)((r - min_val) / range * (bin_count - 1));
    int g_bin = (int)((g - min_val) / range * (bin_count - 1));
    int b_bin = (int)((b - min_val) / range * (bin_count - 1));
    int lum_bin = (int)((lum - min_val) / range * (bin_count - 1));
    
    r_bin = max(0, min(bin_count - 1, r_bin));
    g_bin = max(0, min(bin_count - 1, g_bin));
    b_bin = max(0, min(bin_count - 1, b_bin));
    lum_bin = max(0, min(bin_count - 1, lum_bin));
    
    // Atomic increment
    atomicAdd(&red_bins[r_bin], 1);
    atomicAdd(&green_bins[g_bin], 1);  
    atomicAdd(&blue_bins[b_bin], 1);
    atomicAdd(&lum_bins[lum_bin], 1);
}
"#;

/// CUDA Kernels manager
pub struct CudaKernels {
    device: Arc<CudaDevice>,
    thumbnail_kernel: cudarc::driver::CudaFunction,
    histogram_kernel: cudarc::driver::CudaFunction,
}

impl CudaKernels {
    /// Create and compile CUDA kernels
    pub fn new(device: Arc<CudaDevice>) -> Result<Self> {
        println!("CUDA: Compiling kernels...");
        
        // Compile the kernel source
        let ptx = cudarc::nvrtc::Ptx::from_src(THUMBNAIL_KERNEL_SRC);
        
        device.load_ptx(ptx, "thumbnail_kernels", &[
            "resize_and_tonemap_rgba_f32_to_u8", 
            "compute_histogram_rgba_f32"
        ]).map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to load kernels: {:?}", e)
        })?;
        
        let thumbnail_kernel = device.get_func("thumbnail_kernels", "resize_and_tonemap_rgba_f32_to_u8")
            .map_err(|e| anyhow::anyhow!("CUDA: Failed to get thumbnail kernel: {:?}", e))?;
            
        let histogram_kernel = device.get_func("thumbnail_kernels", "compute_histogram_rgba_f32")
            .map_err(|e| anyhow::anyhow!("CUDA: Failed to get histogram kernel: {:?}", e))?;
        
        println!("CUDA: Kernels compiled successfully");
        
        Ok(CudaKernels {
            device,
            thumbnail_kernel,
            histogram_kernel,
        })
    }
    
    /// Launch thumbnail resize and tone mapping kernel
    pub fn launch_thumbnail_kernel(
        &self,
        input: &CudaSlice<f32>,
        output: &mut CudaSlice<u8>,
        src_width: u32,
        src_height: u32,
        dst_width: u32,
        dst_height: u32,
        exposure: f32,
        gamma: f32,
        tonemap_mode: i32,
        color_matrix: Option<&CudaSlice<f32>>,
    ) -> Result<()> {
        let block_size = (16, 16, 1);
        let grid_size = (
            (dst_width + block_size.0 - 1) / block_size.0,
            (dst_height + block_size.1 - 1) / block_size.1,
            1
        );
        
        let config = LaunchConfig {
            grid_dim: grid_size,
            block_dim: block_size,
            shared_mem_bytes: 0,
        };
        
        // Prepare color matrix parameters
        let (color_matrix_ptr, has_color_matrix) = if let Some(matrix) = color_matrix {
            (matrix as *const CudaSlice<f32> as *const std::ffi::c_void, 1i32)
        } else {
            (std::ptr::null(), 0i32)
        };
        
        // Launch kernel
        unsafe {
            self.thumbnail_kernel.launch(
                config,
                (
                    input as *const CudaSlice<f32> as *const std::ffi::c_void,
                    output as *mut CudaSlice<u8> as *mut std::ffi::c_void,
                    src_width as i32,
                    src_height as i32,
                    dst_width as i32,
                    dst_height as i32,
                    exposure,
                    gamma,
                    tonemap_mode,
                    color_matrix_ptr,
                    has_color_matrix,
                )
            )
        }.map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to launch thumbnail kernel: {:?}", e)
        })?;
        
        Ok(())
    }
    
    /// Launch histogram computation kernel
    pub fn launch_histogram_kernel(
        &self,
        input: &CudaSlice<f32>,
        red_bins: &mut CudaSlice<i32>,
        green_bins: &mut CudaSlice<i32>,
        blue_bins: &mut CudaSlice<i32>,
        lum_bins: &mut CudaSlice<i32>,
        width: u32,
        height: u32,
        min_val: f32,
        max_val: f32,
        bin_count: i32,
    ) -> Result<()> {
        let block_size = (16, 16, 1);
        let grid_size = (
            (width + block_size.0 - 1) / block_size.0,
            (height + block_size.1 - 1) / block_size.1,
            1
        );
        
        let config = LaunchConfig {
            grid_dim: grid_size,
            block_dim: block_size,
            shared_mem_bytes: 0,
        };
        
        // Launch kernel
        unsafe {
            self.histogram_kernel.launch(
                config,
                (
                    input as *const CudaSlice<f32> as *const std::ffi::c_void,
                    red_bins as *mut CudaSlice<i32> as *mut std::ffi::c_void,
                    green_bins as *mut CudaSlice<i32> as *mut std::ffi::c_void,
                    blue_bins as *mut CudaSlice<i32> as *mut std::ffi::c_void,
                    lum_bins as *mut CudaSlice<i32> as *mut std::ffi::c_void,
                    width as i32,
                    height as i32,
                    min_val,
                    max_val,
                    bin_count,
                )
            )
        }.map_err(|e| {
            anyhow::anyhow!("CUDA: Failed to launch histogram kernel: {:?}", e)
        })?;
        
        Ok(())
    }
}