use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use exr::prelude as exr;
use std::path::PathBuf;
use crate::image_processing::process_pixel;
use rayon::prelude::*;
use std::collections::HashMap; // potrzebne dla extract_layers_info
use crate::utils::split_layer_and_short;
use crate::progress::ProgressSink;
use crate::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer;
use glam::{Mat3, Vec3};
use std::sync::Arc;
use crate::full_exr_cache::FullExrCacheData;
use core::simd::{f32x4, Simd};
use std::simd::prelude::SimdFloat;
use wgpu;
use std::sync::Mutex;

/// Zwraca kanoniczny skrót kanału na podstawie aliasów/nazw przyjaznych.
/// Np. "red"/"Red"/"RED"/"R"/"R8" → "R"; analogicznie dla G/B/A.
#[inline]
pub(crate) fn channel_alias_to_short(input: &str) -> String {
    let trimmed = input.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper == "R" || upper.starts_with("RED") { return "R".to_string(); }
    if upper == "G" || upper.starts_with("GREEN") { return "G".to_string(); }
    if upper == "B" || upper.starts_with("BLUE") { return "B".to_string(); }
    if upper == "A" || upper.starts_with("ALPHA") { return "A".to_string(); }
    trimmed.to_string()
}

#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub name: String,
    pub channels: Vec<ChannelInfo>,
}

// split_layer_and_short przeniesione do utils

#[derive(Clone, Debug)]
pub struct ChannelInfo {
    pub name: String,           // krótka nazwa (po ostatniej kropce)
}

#[derive(Clone, Debug)]
pub struct LayerChannels {
    pub layer_name: String,
    pub width: u32,
    pub height: u32,
    // Stabilna lista krótkich nazw kanałów (np. "R", "G", "B", "A", "Z", itp.)
    pub channel_names: Vec<String>,
    // Dane w układzie planarnym: [ch0(0..N), ch1(0..N), ...]
    pub channel_data: Arc<[f32]>, // Zmieniono z Vec<f32> na Arc<[f32]>
}

pub struct ImageCache {
    pub raw_pixels: Vec<(f32, f32, f32, f32)>,
    pub width: u32,
    pub height: u32,
    pub layers_info: Vec<LayerInfo>,
    pub current_layer_name: String,
    // Opcjonalna macierz konwersji z przestrzeni primaries pliku do sRGB (linear RGB)
    color_matrix_rgb_to_srgb: Option<Mat3>,
    // Cache wszystkich kanałów dla bieżącej warstwy aby uniknąć I/O przy przełączaniu
    pub current_layer_channels: Option<LayerChannels>,
    // Pełne dane EXR (wszystkie warstwy i kanały) w pamięci
    full_cache: Arc<FullExrCacheData>,
    // MIP cache: przeskalowane podglądy (float RGBA) do szybkiego preview
    mip_levels: Vec<MipLevel>,
    // Kontekst GPU do akceleracji przetwarzania obrazów
    gpu_context: Option<Arc<Mutex<Option<crate::gpu_context::GpuContext>>>>,
}

#[derive(Clone, Debug)]
struct MipLevel {
    width: u32,
    height: u32,
    pixels: Vec<(f32, f32, f32, f32)>,
}

fn build_mip_chain(
    base_pixels: &[(f32, f32, f32, f32)],
    mut width: u32,
    mut height: u32,
    max_levels: usize,
) -> Vec<MipLevel> {
    let mut levels: Vec<MipLevel> = Vec::new();
    let mut prev: Vec<(f32, f32, f32, f32)> = base_pixels.to_vec();
    for _ in 0..max_levels {
        if width <= 1 && height <= 1 { break; }
        let new_w = (width / 2).max(1);
        let new_h = (height / 2).max(1);
        let mut next: Vec<(f32, f32, f32, f32)> = vec![(0.0, 0.0, 0.0, 0.0); (new_w as usize) * (new_h as usize)];
        // Jednowątkowe uśrednianie 2x2
        for y_out in 0..(new_h as usize) {
            let y0 = (y_out * 2).min(height as usize - 1);
            let y1 = (y0 + 1).min(height as usize - 1);
            for x_out in 0..(new_w as usize) {
                let x0 = (x_out * 2).min(width as usize - 1);
                let x1 = (x0 + 1).min(width as usize - 1);
                let p00 = prev[y0 * (width as usize) + x0];
                let p01 = prev[y0 * (width as usize) + x1];
                let p10 = prev[y1 * (width as usize) + x0];
                let p11 = prev[y1 * (width as usize) + x1];
                let acc = (
                    (p00.0 + p01.0 + p10.0 + p11.0) * 0.25,
                    (p00.1 + p01.1 + p10.1 + p11.1) * 0.25,
                    (p00.2 + p01.2 + p10.2 + p11.2) * 0.25,
                    (p00.3 + p01.3 + p10.3 + p11.3) * 0.25,
                );
                next[y_out * (new_w as usize) + x_out] = acc;
            }
        }
        levels.push(MipLevel { width: new_w, height: new_h, pixels: next.clone() });
        prev = next;
        width = new_w; height = new_h;
        if new_w <= 32 && new_h <= 32 { break; }
    }
    levels
}

impl ImageCache {
    pub fn new_with_full_cache(path: &PathBuf, full_cache: Arc<FullExrCacheData>) -> anyhow::Result<Self> {
        // Najpierw wyciągnij informacje o warstwach (meta), wybierz najlepszą i wczytaj ją jako startowy podgląd
        let layers_info = extract_layers_info(path)?;
        let best_layer = find_best_layer(&layers_info);
        let layer_channels = load_all_channels_for_layer_from_full(&full_cache, &best_layer, None)?;

        let raw_pixels = compose_composite_from_channels(&layer_channels);
        let width = layer_channels.width;
        let height = layer_channels.height;
        let current_layer_name = layer_channels.layer_name.clone();

        // Spróbuj wyliczyć macierz konwersji primaries → sRGB na podstawie atrybutu chromaticities (dla wybranej warstwy/partu)
        let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, &best_layer).ok();

        let mip_levels = build_mip_chain(&raw_pixels, width, height, 4);
        Ok(ImageCache {
            raw_pixels,
            width,
            height,
            layers_info,
            current_layer_name,
            color_matrix_rgb_to_srgb,
            current_layer_channels: Some(layer_channels),
            full_cache: full_cache,
            mip_levels,
            gpu_context: None,
        })
    }
    
    pub fn load_layer(&mut self, path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
        // Jednorazowo wczytaj wszystkie kanały wybranej warstwy z pełnego cache i zbuduj kompozyt
        let layer_channels = load_all_channels_for_layer_from_full(&self.full_cache, layer_name, progress)?;

        self.width = layer_channels.width;
        self.height = layer_channels.height;
        self.current_layer_name = layer_channels.layer_name.clone();
        self.raw_pixels = compose_composite_from_channels(&layer_channels);
        self.current_layer_channels = Some(layer_channels);
        // Reoblicz macierz primaries→sRGB na wypadek, gdyby warstwa/part zmieniały chromaticities
        self.color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name).ok();
        // Odbuduj MIP-y dla nowego obrazu
        self.mip_levels = build_mip_chain(&self.raw_pixels, self.width, self.height, 4);

        Ok(())
    }

    #[inline]
    pub fn color_matrix(&self) -> Option<Mat3> { self.color_matrix_rgb_to_srgb }
    
    /// Ustawia kontekst GPU dla akceleracji
    pub fn set_gpu_context(&mut self, gpu_context: Arc<Mutex<Option<crate::gpu_context::GpuContext>>>) {
        self.gpu_context = Some(gpu_context);
    }
    
    pub fn process_to_image(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // SIMD: przetwarzaj paczki po 4 piksele
        let in_chunks = self.raw_pixels.par_chunks_exact(4);
        let out_chunks = out_slice.par_chunks_mut(4);
        in_chunks.zip(out_chunks).for_each(|(in4, out4)| {
            // Zbierz do rejestrów SIMD
            let (mut r, mut g, mut b, a) = {
                let r = f32x4::from_array([in4[0].0, in4[1].0, in4[2].0, in4[3].0]);
                let g = f32x4::from_array([in4[0].1, in4[1].1, in4[2].1, in4[3].1]);
                let b = f32x4::from_array([in4[0].2, in4[1].2, in4[2].2, in4[3].2]);
                let a = f32x4::from_array([in4[0].3, in4[1].3, in4[2].3, in4[3].3]);
                (r, g, b, a)
            };

            // Macierz kolorów (primaries → sRGB) jeśli dostępna
            if let Some(mat) = color_m {
                let m00 = Simd::splat(mat.x_axis.x);
                let m01 = Simd::splat(mat.y_axis.x);
                let m02 = Simd::splat(mat.z_axis.x);
                let m10 = Simd::splat(mat.x_axis.y);
                let m11 = Simd::splat(mat.y_axis.y);
                let m12 = Simd::splat(mat.z_axis.y);
                let m20 = Simd::splat(mat.x_axis.z);
                let m21 = Simd::splat(mat.y_axis.z);
                let m22 = Simd::splat(mat.z_axis.z);
                let rr = m00 * r + m01 * g + m02 * b;
                let gg = m10 * r + m11 * g + m12 * b;
                let bb = m20 * r + m21 * g + m22 * b;
                r = rr; g = gg; b = bb;
            }

            let (r8, g8, b8) = crate::image_processing::tone_map_and_gamma_simd(r, g, b, exposure, gamma, tonemap_mode);
            let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

            // Zapisz 4 piksele
            let ra: [f32; 4] = r8.into();
            let ga: [f32; 4] = g8.into();
            let ba: [f32; 4] = b8.into();
            let aa: [f32; 4] = a8.into();
            for i in 0..4 {
                out4[i] = Rgba8Pixel {
                    r: (ra[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    g: (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    b: (ba[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    a: (aa[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                };
            }
        });

        // Remainder (0..3 piksele)
        let rem = self.raw_pixels.len() % 4;
        if rem > 0 {
            let start = self.raw_pixels.len() - rem;
            for i in 0..rem {
                let (r0, g0, b0, a0) = self.raw_pixels[start + i];
                let mut r = r0; let mut g = g0; let mut b = b0;
                if let Some(mat) = color_m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                out_slice[start + i] = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
            }
        }

        Image::from_rgba8(buffer)
    }
    
    /// Przetwarza obraz na GPU z użyciem compute shadera
    pub fn process_to_image_gpu(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Result<Image, Box<dyn std::error::Error>> {
        println!("GPU: Rozpoczynam przetwarzanie obrazu {}x{} pikseli", self.width, self.height);
        
        // Wrap całą funkcję w catch_unwind dla bezpieczeństwa
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.process_to_image_gpu_internal(exposure, gamma, tonemap_mode)
        }));
        
        match result {
            Ok(res) => res,
            Err(_) => {
                println!("GPU: PANIC wykryty w przetwarzaniu GPU! Fallback do CPU.");
                Err("Panic w przetwarzaniu GPU - fallback do CPU".into())
            }
        }
    }
    
    /// Wewnętrzna funkcja przetwarzania GPU (bez catch_unwind)
    fn process_to_image_gpu_internal(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Result<Image, Box<dyn std::error::Error>> {
        
        // Sprawdź czy kontekst GPU jest dostępny
        let gpu_context = match &self.gpu_context {
            Some(ctx) => ctx,
            None => {
                println!("GPU: Błąd - kontekst GPU nie jest dostępny");
                return Err("Kontekst GPU nie jest dostępny".into());
            }
        };
        
        let gpu_guard = gpu_context.lock().map_err(|e| {
            println!("GPU: Błąd - nie można uzyskać dostępu do kontekstu GPU: {:?}", e);
            "Nie można uzyskać dostępu do kontekstu GPU"
        })?;
        let gpu_context = match gpu_guard.as_ref() {
            Some(ctx) => ctx,
            None => {
                println!("GPU: Błąd - kontekst GPU nie został zainicjalizowany");
                return Err("Kontekst GPU nie został zainicjalizowany".into());
            }
        };
        
        // Dodatkowe sprawdzenie dostępności GPU
        if !gpu_context.is_available() {
            println!("GPU: Błąd - urządzenie GPU nie jest dostępne");
            return Err("Urządzenie GPU nie jest dostępne".into());
        }
        
        // Sprawdź rozmiar obrazu
        if self.width == 0 || self.height == 0 {
            println!("GPU: Błąd - nieprawidłowe wymiary obrazu: {}x{}", self.width, self.height);
            return Err("Nieprawidłowe wymiary obrazu".into());
        }
        
        if self.raw_pixels.is_empty() {
            println!("GPU: Błąd - brak danych obrazu do przetworzenia");
            return Err("Brak danych obrazu do przetworzenia".into());
        }
        
        // Sprawdź czy rozmiar pikseli odpowiada wymiarom obrazu
        let expected_pixels = (self.width * self.height) as usize;
        if self.raw_pixels.len() != expected_pixels {
            println!("GPU: Błąd - niezgodność rozmiaru: oczekiwano {} pikseli, mam {}", 
                     expected_pixels, self.raw_pixels.len());
            return Err("Niezgodność rozmiaru pikseli z wymiarami obrazu".into());
        }
        
        // Sprawdź czy wymiary nie są zbyt duże dla GPU
        const MAX_DIMENSION: u32 = 16384; // 16K max
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            println!("GPU: Błąd - obraz zbyt duży: {}x{} (max {}x{})", 
                     self.width, self.height, MAX_DIMENSION, MAX_DIMENSION);
            return Err("Obraz zbyt duży dla przetwarzania GPU".into());
        }
        
        // Sprawdź limity GPU
        let (_, limits) = gpu_context.get_device_info();
        let max_buffer_size = limits.max_buffer_size;
        let input_size = (self.raw_pixels.len() * std::mem::size_of::<[f32; 4]>()) as u64;
        let output_size = (self.width * self.height * 4) as u64;
        
        if input_size > max_buffer_size || output_size > max_buffer_size {
            println!("GPU: Błąd - bufory zbyt duże dla GPU: input {} MB, output {} MB (max {} MB)", 
                     input_size / 1024 / 1024, output_size / 1024 / 1024, max_buffer_size / 1024 / 1024);
            return Err("Bufory zbyt duże dla GPU".into());
        }
        
        println!("GPU: Parametry - exposure: {}, gamma: {}, tonemap: {}", exposure, gamma, tonemap_mode);
        
        // Parametry uniformów
        #[repr(C)]
        #[derive(bytemuck::Pod, bytemuck::Zeroable, Clone, Copy)]
        struct Params {
            exposure: f32,
            gamma: f32,
            tonemap_mode: u32,
            width: u32,
            height: u32,
        }
        
        let params = Params {
            exposure,
            gamma,
            tonemap_mode: tonemap_mode as u32,
            width: self.width,
            height: self.height,
        };
        
        // Utworzenie buforów z logowaniem
        println!("GPU: Tworzę bufory - wejściowy: {} bajtów, wyjściowy: {} bajtów", 
                 self.raw_pixels.len() * std::mem::size_of::<[f32; 4]>(),
                 self.width * self.height * 4);
        
        let input_buffer = gpu_context.create_storage_buffer(
            "input_pixels",
            (self.raw_pixels.len() * std::mem::size_of::<[f32; 4]>()) as u64,
            true, // read_only
        );
        
        // NAPRAWIONE: Bufory dla u32 (4 bajty na piksel) - poprawione typy
        let output_buffer = gpu_context.create_storage_buffer(
            "output_pixels",
            self.width as u64 * self.height as u64 * std::mem::size_of::<u32>() as u64,
            false, // write_only
        );
        
        let staging_buffer = gpu_context.create_staging_buffer(
            "staging_buffer",
            self.width as u64 * self.height as u64 * std::mem::size_of::<u32>() as u64,
        );
        
        let uniform_buffer = gpu_context.create_uniform_buffer("params", &params);
        
        // Wypełnienie bufora wejściowego danymi
        println!("GPU: Przygotowuję dane wejściowe - {} pikseli", self.raw_pixels.len());
        let input_data: Vec<[f32; 4]> = self.raw_pixels.iter()
            .map(|(r, g, b, a)| [*r, *g, *b, *a])
            .collect();
        
        // Sprawdź dane wejściowe na problematyczne wartości
        let problematic_pixels = input_data.iter().enumerate().take(10).filter(|(_, pixel)| {
            !pixel[0].is_finite() || !pixel[1].is_finite() || !pixel[2].is_finite() || !pixel[3].is_finite()
        }).count();
        
        if problematic_pixels > 0 {
            println!("GPU: Ostrzeżenie - znaleziono {} problematycznych pikseli (NaN/Inf) w pierwszych 10", problematic_pixels);
        }
        
        gpu_context.queue.write_buffer(&input_buffer, 0, bytemuck::cast_slice(&input_data));
        
        // Utworzenie layoutu bind group - POPRAWIONE: wszystkie bindingi w jednej grupie
        let bind_group_layout = gpu_context.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image_processing_bind_group_layout"),
            entries: &[
                // Binding 0: Uniformy
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
                // Binding 1: Bufor wejściowy (read-only)
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
                // Binding 2: Bufor wyjściowy (write-only)
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
        });
        
        // Utworzenie pipeline compute
        let shader_module = gpu_context.create_shader_module(
            "image_processing_shader",
            include_str!("shaders/image_processing.wgsl")
        );
        
        let pipeline_layout = gpu_context.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image_processing_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        
        let compute_pipeline = gpu_context.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("image_processing_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: Some("main"),
            cache: None,
            compilation_options: Default::default(),
        });
        
        // Utworzenie bind group - POPRAWIONE: poprawne bindingi
        let bind_group = gpu_context.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image_processing_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: input_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output_buffer.as_entire_binding(),
                },
            ],
        });
        
        // Wysłanie komend do GPU
        println!("GPU: Przygotowuję komendy przetwarzania");
        let mut encoder = gpu_context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("image_processing_encoder"),
        });
        
        {
            let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("image_processing_compute_pass"),
                timestamp_writes: None,
            });
            
            compute_pass.set_pipeline(&compute_pipeline);
            compute_pass.set_bind_group(0, &bind_group, &[]);
            
            // Obliczenie liczby grup roboczych
            let workgroup_size = 8;
            let workgroups_x = (self.width + workgroup_size - 1) / workgroup_size;
            let workgroups_y = (self.height + workgroup_size - 1) / workgroup_size;
            
            println!("GPU: Uruchamiam compute shader - grupy robocze: {}x{} (workgroup_size: {})", 
                     workgroups_x, workgroups_y, workgroup_size);
            
            compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
        }
        
        // Kopiowanie wyników do staging buffer
        println!("GPU: Kopiuję wyniki do staging buffer");
        encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging_buffer, 0, self.width as u64 * self.height as u64 * std::mem::size_of::<u32>() as u64);
        
        // Wysłanie komend
        println!("GPU: Wysyłam komendy do wykonania");
        gpu_context.queue.submit(std::iter::once(encoder.finish()));
        
        // RELEASE MODE: Force sync przed mapowaniem
        println!("GPU: Synchronizuję operacje GPU...");
        let _ = gpu_context.device.poll(wgpu::PollType::Wait);
        
        // Sprawdź stan urządzenia po wysłaniu komend
        if !gpu_context.is_available() {
            return Err("Urządzenie GPU zostało utracone podczas przetwarzania".into());
        }
        
        // Odczyt wyników - NAPRAWIONE: prawidłowa kolejność operacji
        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        
        // Mapowanie bufora z obsługą błędów
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        
        // Najpierw poczekaj na zakończenie wszystkich operacji GPU
        let start_time = std::time::Instant::now();
        const MAX_WAIT_TIME: std::time::Duration = std::time::Duration::from_secs(30); // ZWIĘKSZONY TIMEOUT dla release
        
        // NAPRAWIONE: bezpieczniejsze oczekiwanie na mapowanie
        loop {
            // Krótki poll zamiast Wait - unikamy zablokowania
            let _ = gpu_context.device.poll(wgpu::PollType::Wait);
            
            // Sprawdź czy otrzymaliśmy odpowiedź z mapowania
            match rx.try_recv() {
                Ok(Ok(())) => {
                    println!("GPU: Mapowanie bufora zakończone pomyślnie");
                    break;
                },
                Ok(Err(e)) => {
                    println!("GPU: Błąd mapowania bufora: {:?}", e);
                    return Err(format!("Błąd mapowania bufora GPU: {:?}", e).into());
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Jeszcze czekamy
                    if start_time.elapsed() > MAX_WAIT_TIME {
                        println!("GPU: Timeout podczas mapowania bufora");
                        return Err("Timeout podczas oczekiwania na mapowanie bufora GPU".into());
                    }
                    // RELEASE MODE: jeszcze dłuższy sleep dla stabilności
                    std::thread::sleep(std::time::Duration::from_millis(50));
                },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    println!("GPU: Kanał komunikacji został zamknięty");
                    return Err("Błąd komunikacji podczas mapowania bufora GPU".into());
                }
            }
        }
        
        // Pobranie zmapowanych danych - POPRAWIONE: bezpieczny odczyt
        let data = buffer_slice.get_mapped_range();
        if data.is_empty() {
            return Err("Bufor GPU jest pusty".into());
        }
        
        // NAPRAWIONE: Odczyt danych jako u32 (packed RGBA)
        let data_u32: &[u32] = bytemuck::cast_slice(&data);
        
        // Sprawdzenie rozmiaru danych
        if data_u32.len() != (self.width * self.height) as usize {
            return Err(format!(
                "Nieprawidłowy rozmiar danych GPU: oczekiwano {} pikseli u32, otrzymano {}", 
                self.width * self.height, 
                data_u32.len()
            ).into());
        }
        
        println!("GPU: Odczytano {} pikseli z bufora", data_u32.len());
        
        // Stworzenie SharedPixelBuffer
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();
        
        // Rozpakuj u32 do RGBA
        for (i, &packed_pixel) in data_u32.iter().enumerate() {
            if i < out_slice.len() {
                out_slice[i] = Rgba8Pixel {
                    r: (packed_pixel & 0xFF) as u8,
                    g: ((packed_pixel >> 8) & 0xFF) as u8,
                    b: ((packed_pixel >> 16) & 0xFF) as u8,
                    a: ((packed_pixel >> 24) & 0xFF) as u8,
                };
            }
        }
        
        drop(data);
        
        // Unmapowanie bufora
        staging_buffer.unmap();
        
        Ok(Image::from_rgba8(buffer))
    }

    pub fn process_to_composite(&self, exposure: f32, gamma: f32, tonemap_mode: i32, lighting_rgb: bool) -> Image {
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let out_slice = buffer.make_mut_slice();

        let color_m = self.color_matrix_rgb_to_srgb;

        // SIMD: przetwarzaj paczki po 4 piksele
        let in_chunks = self.raw_pixels.par_chunks_exact(4);
        let out_chunks = out_slice.par_chunks_mut(4);
        in_chunks.zip(out_chunks).for_each(|(in4, out4)| {
            // Zbierz do rejestrów SIMD
            let (mut r, mut g, mut b, a) = {
                let r = f32x4::from_array([in4[0].0, in4[1].0, in4[2].0, in4[3].0]);
                let g = f32x4::from_array([in4[0].1, in4[1].1, in4[2].1, in4[3].1]);
                let b = f32x4::from_array([in4[0].2, in4[1].2, in4[2].2, in4[3].2]);
                let a = f32x4::from_array([in4[0].3, in4[1].3, in4[2].3, in4[3].3]);
                (r, g, b, a)
            };

            // Macierz kolorów (primaries → sRGB) jeśli dostępna
            if let Some(mat) = color_m {
                let m00 = Simd::splat(mat.x_axis.x);
                let m01 = Simd::splat(mat.y_axis.x);
                let m02 = Simd::splat(mat.z_axis.x);
                let m10 = Simd::splat(mat.x_axis.y);
                let m11 = Simd::splat(mat.y_axis.y);
                let m12 = Simd::splat(mat.z_axis.y);
                let m20 = Simd::splat(mat.x_axis.z);
                let m21 = Simd::splat(mat.y_axis.z);
                let m22 = Simd::splat(mat.z_axis.z);
                let rr = m00 * r + m01 * g + m02 * b;
                let gg = m10 * r + m11 * g + m12 * b;
                let bb = m20 * r + m21 * g + m22 * b;
                r = rr; g = gg; b = bb;
            }

            if lighting_rgb {
                let (r8, g8, b8) = crate::image_processing::tone_map_and_gamma_simd(r, g, b, exposure, gamma, tonemap_mode);
                let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

                let ra: [f32; 4] = r8.into();
                let ga: [f32; 4] = g8.into();
                let ba: [f32; 4] = b8.into();
                let aa: [f32; 4] = a8.into();
                for i in 0..4 {
                    out4[i] = Rgba8Pixel {
                        r: (ra[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                        g: (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                        b: (ba[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                        a: (aa[i] * 255.0).round().clamp(0.0, 255.0) as u8,
                    };
                }
            } else {
                // Grayscale processing
                let (r_linear, g_linear, b_linear) = (r, g, b); // Keep linear for grayscale conversion
                let (r_tm, g_tm, b_tm) = crate::image_processing::tone_map_and_gamma_simd(r_linear, g_linear, b_linear, exposure, gamma, tonemap_mode);

                let gray = r_tm.simd_max(g_tm).simd_max(b_tm).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
                let a8 = a.simd_clamp(Simd::splat(0.0), Simd::splat(1.0));

                let ga: [f32; 4] = gray.into();
                let aa: [f32; 4] = a8.into();
                for i in 0..4 {
                    let g8 = (ga[i] * 255.0).round().clamp(0.0, 255.0) as u8;
                    out4[i] = Rgba8Pixel { r: g8, g: g8, b: g8, a: aa[i] as u8 }; // Alpha should be 255 if not explicitly set
                }
            }
        });

        // Remainder (0..3 piksele)
        let rem = self.raw_pixels.len() % 4;
        if rem > 0 {
            let start = self.raw_pixels.len() - rem;
            for i in 0..rem {
                let (r0, g0, b0, a0) = self.raw_pixels[start + i];
                let (mut r, mut g, mut b) = (r0, g0, b0);
                if let Some(mat) = color_m {
                    let v = mat * Vec3::new(r, g, b);
                    r = v.x; g = v.y; b = v.z;
                }
                if lighting_rgb {
                    out_slice[start + i] = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
                } else {
                    let px = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);
                    let rr = (px.r as f32) / 255.0;
                    let gg = (px.g as f32) / 255.0;
                    let bb = (px.b as f32) / 255.0;
                    let gray = (rr.max(gg).max(bb)).clamp(0.0, 1.0);
                    let g8 = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
                    out_slice[start + i] = Rgba8Pixel { r: g8, g: g8, b: g8, a: px.a };
                }
            }
        }

        Image::from_rgba8(buffer)
    }
    // Nowa metoda dla preview (szybsze przetwarzanie małego obrazka)
    pub fn process_to_thumbnail(&self, exposure: f32, gamma: f32, tonemap_mode: i32, max_size: u32) -> Image {
        let scale = (max_size as f32 / self.width.max(self.height) as f32).min(1.0);
        let thumb_width = (self.width as f32 * scale) as u32;
        let thumb_height = (self.height as f32 * scale) as u32;
        
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(thumb_width, thumb_height);
        let slice = buffer.make_mut_slice();
        
        // Wybierz najlepsze źródło: oryginał lub najbliższy MIP >= docelowej wielkości
        let (src_pixels, src_w, src_h): (&[(f32, f32, f32, f32)], u32, u32) = {
            if self.mip_levels.is_empty() { (&self.raw_pixels[..], self.width, self.height) } else {
                // wybierz poziom, którego dłuższy bok jest najbliższy docelowemu, ale nie mniejszy niż docelowy
                let target = thumb_width.max(thumb_height);
                if let Some(lvl) = self.mip_levels.iter().find(|lvl| lvl.width.max(lvl.height) >= target) {
                    (&lvl.pixels[..], lvl.width, lvl.height)
                } else {
                    // Fallback: użyj pełnej rozdzielczości (nigdy poniżej 1:1)
                    (&self.raw_pixels[..], self.width, self.height)
                }
            }
        };

        // Proste nearest neighbor sampling dla szybkości, ale przetwarzanie blokami 4 pikseli
        let m = self.color_matrix_rgb_to_srgb;
        let scale_x = (src_w as f32) / (thumb_width.max(1) as f32);
        let scale_y = (src_h as f32) / (thumb_height.max(1) as f32);
        // SIMD: paczki po 4 piksele miniatury (równolegle na blokach 4 pikseli)
        slice
            .par_chunks_mut(4) // 4 piksele na blok
            .enumerate()
            .for_each(|(block_idx, out_block)| {
                let base = block_idx * 4;
                let mut rr = [0.0f32; 4];
                let mut gg = [0.0f32; 4];
                let mut bb = [0.0f32; 4];
                let mut aa = [1.0f32; 4];
                let mut valid = 0usize;
                for lane in 0..4 {
                    let i = base + lane;
                    if i >= (thumb_width as usize) * (thumb_height as usize) { break; }
                    let x = (i as u32) % thumb_width;
                    let y = (i as u32) / thumb_width;
                    let src_x = ((x as f32) * scale_x) as u32;
                    let src_y = ((y as f32) * scale_y) as u32;
                    let sx = src_x.min(src_w.saturating_sub(1)) as usize;
                    let sy = src_y.min(src_h.saturating_sub(1)) as usize;
                    let src_idx = sy * (src_w as usize) + sx;
                    let (mut r, mut g, mut b, a) = src_pixels[src_idx];
                    if let Some(mat) = m {
                        let v = mat * Vec3::new(r, g, b);
                        r = v.x; g = v.y; b = v.z;
                    }
                    rr[lane] = r; gg[lane] = g; bb[lane] = b; aa[lane] = a; valid += 1;
                }
                let (r8, g8, b8) = crate::image_processing::tone_map_and_gamma_simd(
                    f32x4::from_array(rr), f32x4::from_array(gg), f32x4::from_array(bb), exposure, gamma, tonemap_mode);
                let a8 = f32x4::from_array(aa).simd_clamp(Simd::splat(0.0), Simd::splat(1.0));
                let ra: [f32; 4] = r8.into();
                let ga: [f32; 4] = g8.into();
                let ba: [f32; 4] = b8.into();
                let aa: [f32; 4] = a8.into();
                for lane in 0..valid.min(4) {
                    out_block[lane] = Rgba8Pixel {
                        r: (ra[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                        g: (ga[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                        b: (ba[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                        a: (aa[lane] * 255.0).round().clamp(0.0, 255.0) as u8,
                    };
                }
            });
        
        Image::from_rgba8(buffer)
    }

    /// Zwraca współdzielony wskaźnik do pełnego obrazu EXR trzymanego w pamięci
    #[allow(dead_code)]
    pub fn full_cache(&self) -> Arc<FullExrCacheData> { self.full_cache.clone() }
}



pub(crate) fn extract_layers_info(path: &PathBuf) -> anyhow::Result<Vec<LayerInfo>> {
        // Odczytaj jedynie meta-dane (nagłówki) bez pikseli
        let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/false)?;

        // Mapowanie: nazwa_warstwy -> kanały
        let mut layer_map: HashMap<String, Vec<ChannelInfo>> = HashMap::new();
        // Kolejność pierwszego wystąpienia nazw warstw do stabilnego porządku w UI
        let mut layer_order: Vec<String> = Vec::new();

        for header in meta.headers.iter() {
            // Preferuj nazwę z atrybutu warstwy; jeśli brak, kanały mogą być w formacie "warstwa.kanał"
            let base_layer_name: Option<String> = header
                .own_attributes
                .layer_name
                .as_ref()
                .map(|t| t.to_string());

            for ch in header.channels.list.iter() {
                let full_channel_name = ch.name.to_string();
                let (layer_name_effective, short_channel_name) =
                    split_layer_and_short(&full_channel_name, base_layer_name.as_deref());

                let entry = layer_map.entry(layer_name_effective.clone()).or_insert_with(|| {
                    layer_order.push(layer_name_effective.clone());
                    Vec::new()
                });

                entry.push(ChannelInfo { name: short_channel_name });
            }
        }

        // Zbuduj listę warstw w kolejności pierwszego wystąpienia
        let mut layers: Vec<LayerInfo> = Vec::with_capacity(layer_map.len());
        for name in layer_order {
            if let Some(channels) = layer_map.remove(&name) {
                layers.push(LayerInfo { name, channels });
            }
        }

        Ok(layers)
}

pub(crate) fn find_best_layer(layers_info: &[LayerInfo]) -> String {
    // DODAJĘ DEBUGOWANIE - może to jest źródło problemu z przesuniętymi liniami!
    println!("DEBUG find_best_layer: {} warstw dostępnych", layers_info.len());
    for (i, layer) in layers_info.iter().enumerate() {
        println!("DEBUG layer {}: '{}' z kanałami: {:?}", 
                 i, layer.name, layer.channels.iter().map(|c| &c.name).collect::<Vec<_>>());
    }
    
    // Plan A: Sprawdź czy istnieje warstwa pusta ("") z kanałami R, G, B
    // Ta warstwa zawiera główne kanały obrazu bez prefiksu
    if let Some(layer) = layers_info.iter().find(|l| l.name.is_empty()) {
        let mut has_r = false;
        let mut has_g = false;
        let mut has_b = false;
        for ch in &layer.channels {
            let n = ch.name.trim().to_ascii_uppercase();
            if n == "R" { has_r = true; }
            else if n == "G" { has_g = true; }
            else if n == "B" { has_b = true; }
        }
        if has_r && has_g && has_b {
            println!("DEBUG find_best_layer: WYBRANO warstwę pustą '{}' (Plan A)", layer.name);
            return layer.name.clone();
        }
    }
    
    // Plan B: Priorytetowa lista nazw warstw (zgodnie z mini.md)
    let priority_names = ["beauty", "Beauty", "RGBA", "rgba", "default", "Default", "combined", "Combined"];
    
    // Sprawdź czy istnieje warstwa o priorytetowej nazwie
    for priority_name in &priority_names {
        if let Some(layer) = layers_info.iter().find(|l| l.name.to_lowercase().contains(&priority_name.to_lowercase())) {
            println!("DEBUG find_best_layer: WYBRANO warstwę '{}' (Plan B - priority: {})", layer.name, priority_name);
            return layer.name.clone();
        }
    }
    
    // Plan C: Znajdź pierwszą warstwę z kanałami R, G, B (porównanie dokładne krótkie nazwy)
    for layer in layers_info {
        let mut has_r = false;
        let mut has_g = false;
        let mut has_b = false;
        for ch in &layer.channels {
            let n = ch.name.trim().to_ascii_uppercase();
            if n == "R" { has_r = true; }
            else if n == "G" { has_g = true; }
            else if n == "B" { has_b = true; }
        }
        if has_r && has_g && has_b {
            println!("DEBUG find_best_layer: WYBRANO warstwę '{}' (Plan C - ma R,G,B)", layer.name);
            return layer.name.clone();
        }
    }
    
    // Plan D (ostateczność): Pierwsza warstwa
    let result = layers_info.first()
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "Layer 1".to_string());
    
    println!("DEBUG find_best_layer: WYBRANO warstwę '{}' (Plan D - pierwsza)", result);
    result
}

pub(crate) fn load_specific_layer(path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<(Vec<(f32, f32, f32, f32)>, u32, u32, String)> {
    // Szybka ścieżka: jeżeli prosimy o bazową/typową warstwę RGBA, użyj gotowej funkcji czytającej pierwszą RGBA
    // Dotyczy częstych nazw: "", "beauty", "rgba", "default", "combined"
    let lname = layer_name.trim();
    let lname_lower = lname.to_ascii_lowercase();
    let is_typical_rgba = lname.is_empty()
        || lname_lower == "beauty"
        || lname_lower == "rgba"
        || lname_lower == "default"
        || lname_lower == "combined";

    if is_typical_rgba {
        if let Some(p) = progress { p.set(0.08, Some("Fast path: RGBA layer")); }
        if let Ok((pixels, w, h, _)) = load_first_rgba_layer(path) {
            if let Some(p) = progress { p.set(0.92, Some("Fast path done")); }
            return Ok((pixels, w, h, layer_name.to_string()));
        }
    }

    // Standardowa ścieżka: wczytaj płaskie warstwy (bez mip-map), aby uzyskać FlatSamples
    if let Some(p) = progress { p.set(0.1, Some("Reading layer data...")); }
    let any_image = exr::read_all_flat_layers_from_file(path)?;

    // Szukaj grupy kanałów odpowiadającej nazwie warstwy (spójne z extract_layers_info)
    let wanted_lower = layer_name.to_lowercase();
    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);

        let base_attr: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());

        // Indeksy R/G/B/A w grupie, jeśli dopasowano, oraz lista wszystkich indeksów w grupie
        let mut r_idx: Option<usize> = None;
        let mut g_idx: Option<usize> = None;
        let mut b_idx: Option<usize> = None;
        let mut a_idx: Option<usize> = None;
        let mut group_found = false;
        let mut group_indices: Vec<usize> = Vec::with_capacity(layer.channel_data.list.len());

        let name_matches = |lname: &str| -> bool {
            let lname_lower = lname.to_lowercase();
            if wanted_lower.is_empty() && lname_lower.is_empty() {
                true
            } else if wanted_lower.is_empty() || lname_lower.is_empty() {
                false
            } else {
                lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
            }
        };

        for (idx, ch) in layer.channel_data.list.iter().enumerate() {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());

            if name_matches(&lname) {
                group_found = true;
                group_indices.push(idx);
                let su = short.to_ascii_uppercase();
                
                // DODAJĘ DEBUGOWANIE - sprawdzam wykrywanie kanałów R/G/B/A
                println!("DEBUG channel detection: '{}' -> short='{}' -> su='{}'", full, short, su);
                
                match su.as_str() {
                    "R" | "RED" => {
                        r_idx = Some(idx);
                        println!("DEBUG: Znaleziono R kanał na indeksie {}", idx);
                    }
                    "G" | "GREEN" => {
                        g_idx = Some(idx);
                        println!("DEBUG: Znaleziono G kanał na indeksie {}", idx);
                    }
                    "B" | "BLUE" => {
                        b_idx = Some(idx);
                        println!("DEBUG: Znaleziono B kanał na indeksie {}", idx);
                    }
                    "A" | "ALPHA" => {
                        a_idx = Some(idx);
                        println!("DEBUG: Znaleziono A kanał na indeksie {}", idx);
                    }
                    _ => {
                        // Dodatkowe heurystyki: nazwy zaczynające się od R/G/B
                        if r_idx.is_none() && su.starts_with('R') { 
                            r_idx = Some(idx);
                            println!("DEBUG: Znaleziono R kanał (heurystyka) na indeksie {}", idx);
                        }
                        else if g_idx.is_none() && su.starts_with('G') { 
                            g_idx = Some(idx);
                            println!("DEBUG: Znaleziono G kanał (heurystyka) na indeksie {}", idx);
                        }
                        else if b_idx.is_none() && su.starts_with('B') { 
                            b_idx = Some(idx);
                            println!("DEBUG: Znaleziono B kanał (heurystyka) na indeksie {}", idx);
                        }
                    }
                }
            }
        }

        if group_found {
            if let Some(p) = progress { p.set(0.4, Some("Processing pixels...")); }
            
            // DODAJĘ DEBUGOWANIE - sprawdzam mapowanie kanałów R/G/B/A
            println!("DEBUG load_specific_layer: Znaleziono warstwę '{}'", layer_name);
            println!("DEBUG load_specific_layer: r_idx={:?}, g_idx={:?}, b_idx={:?}, a_idx={:?}", 
                     r_idx, g_idx, b_idx, a_idx);
            println!("DEBUG load_specific_layer: group_indices={:?}", group_indices);
            
            // DODAJĘ DEBUGOWANIE - sprawdzam kolejność kanałów w pliku
            println!("DEBUG load_specific_layer: Kolejność kanałów w pliku:");
            for (idx, ch) in layer.channel_data.list.iter().enumerate() {
                let full = ch.name.to_string();
                let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());
                println!("DEBUG   [{}]: '{}' -> layer='{}', short='{}'", idx, full, lname, short);
            }
            
            // Zapewnij 3 kanały: jeśli brakuje, uzupełnij z listy kanałów grupy lub duplikuj poprzedni
            if r_idx.is_none() {
                r_idx = group_indices.get(0).cloned();
                println!("DEBUG load_specific_layer: Uzupełniono r_idx={:?}", r_idx);
            }
            if g_idx.is_none() {
                g_idx = group_indices.get(1).cloned().or(r_idx);
                println!("DEBUG load_specific_layer: Uzupełniono g_idx={:?}", g_idx);
            }
            if b_idx.is_none() {
                b_idx = group_indices.get(2).cloned().or(g_idx).or(r_idx);
                println!("DEBUG load_specific_layer: Uzupełniono b_idx={:?}", b_idx);
            }

            // Jeżeli nadal coś jest None (pusta grupa), zgłoś błąd
            let (ri, gi, bi) = match (r_idx, g_idx, b_idx) {
                (Some(ri), Some(gi), Some(bi)) => (ri, gi, bi),
                _ => anyhow::bail!("Warstwa '{}' nie zawiera kanałów do kompozytu", layer_name),
            };
            
            println!("DEBUG load_specific_layer: Finalne indeksy: r={}, g={}, b={}", ri, gi, bi);

            let mut out: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(pixel_count);
            
            // DODAJĘ DEBUGOWANIE - sprawdzam kolejność pikseli w buforze
            println!("DEBUG load_specific_layer: Rozpoczynam wczytywanie {} pikseli ({}x{})", 
                     pixel_count, width, height);
            
            for i in 0..pixel_count {
                let x = i % width as usize;
                let y = i / width as usize;
                
                // Debugowanie dla pierwszych kilku pikseli każdego wiersza
                if x < 5 && y < 5 {
                    println!("DEBUG pixel[{}]: pos=({},{})", i, x, y);
                }
                
                let r = layer.channel_data.list[ri].sample_data.value_by_flat_index(i).to_f32();
                let g = layer.channel_data.list[gi].sample_data.value_by_flat_index(i).to_f32();
                let b = layer.channel_data.list[bi].sample_data.value_by_flat_index(i).to_f32();
                let a = a_idx.map(|ci| layer.channel_data.list[ci].sample_data.value_by_flat_index(i).to_f32()).unwrap_or(1.0);
                out.push((r, g, b, a));
            }
            
            // DODAJĘ DEBUGOWANIE - sprawdzam pierwsze kilka pikseli
            if pixel_count > 0 {
                println!("DEBUG load_specific_layer: Pierwszy piksel: R={:.3}, G={:.3}, B={:.3}, A={:.3}", 
                         out[0].0, out[0].1, out[0].2, out[0].3);
                
                // Sprawdź czy ostatni piksel jest poprawny
                let last_idx = pixel_count - 1;
                let last_x = last_idx % width as usize;
                let last_y = last_idx / width as usize;
                println!("DEBUG load_specific_layer: Ostatni piksel[{}]: pos=({},{}) R={:.3}, G={:.3}, B={:.3}, A={:.3}", 
                         last_idx, last_x, last_y, out[last_idx].0, out[last_idx].1, out[last_idx].2, out[last_idx].3);
            }
            
            if let Some(p) = progress { p.set(0.9, Some("Finalizing...")); }
            // Zwracamy żądaną nazwę jako aktualną, aby była spójna z UI
            return Ok((out, width, height, layer_name.to_string()));
        }
    }

    // Jeśli nie znaleziono warstwy, fallback do pierwszej RGBA
    let (pixels, width, height, _) = load_first_rgba_layer(path)?;
    Ok((pixels, width, height, layer_name.to_string()))
}

fn load_first_rgba_layer(path: &PathBuf) -> anyhow::Result<(Vec<(f32, f32, f32, f32)>, u32, u32, String)> {
    use std::convert::Infallible;
    use std::cell::RefCell;
    use std::rc::Rc;
    
    let pixels = Rc::new(RefCell::new(Vec::new()));
    let dimensions = Rc::new(RefCell::new((0u32, 0u32)));
    
    let pixels_clone1 = pixels.clone();
    let pixels_clone2 = pixels.clone();
    let dimensions_clone = dimensions.clone();
    
    exr::read_first_rgba_layer_from_file(
        path,
        move |resolution, _| -> Result<(), Infallible> {
            let width = resolution.width() as u32;
            let height = resolution.height() as u32;
            *dimensions_clone.borrow_mut() = (width, height);
            pixels_clone1.borrow_mut().reserve_exact((width * height) as usize);
            Ok(())
        },
        move |_, _, (r, g, b, a): (f32, f32, f32, f32)| {
            pixels_clone2.borrow_mut().push((r, g, b, a));
        },
    )?;

    let (width, height) = *dimensions.borrow();
    let raw_pixels = match Rc::try_unwrap(pixels) {
        Ok(cell) => cell.into_inner(),
        Err(rc) => rc.borrow().clone(),
    };
    
    Ok((raw_pixels, width, height, "First RGBA Layer".to_string()))
}

// Funkcja usunięta - nie jest używana w uproszczonej implementacji

// usunięto rozbudowane wykrywanie rodzaju kanału — UI pokazuje teraz realne kanały bez grupowania

impl ImageCache {
    /// Wczytuje jeden wskazany kanał z danej warstwy i zapisuje go jako grayscale (R=G=B=val, A=1)
    pub fn load_channel(&mut self, path: &PathBuf, layer_name: &str, channel_short: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
        // Zapewnij, że cache kanałów dla żądanej warstwy jest dostępny
        let need_reload = self.current_layer_channels.as_ref().map(|lc| lc.layer_name.to_lowercase() != layer_name.to_lowercase()).unwrap_or(true);
        if need_reload {
            // Załaduj wskazaną warstwę (zapełni current_layer_channels oraz ustawi kompozyt)
            self.load_layer(path, layer_name, progress)?;
        }

        // Teraz mamy current_layer_channels dla właściwej warstwy
        let layer_cache = self.current_layer_channels.as_ref().ok_or_else(|| anyhow::anyhow!("Brak cache kanałów dla warstwy"))?;

        let pixel_count = (layer_cache.width as usize) * (layer_cache.height as usize);

        let find_channel_index = |wanted: &str| -> Option<usize> {
            // 1) dokładne dopasowanie (case-sensitive)
            if let Some(idx) = layer_cache.channel_names.iter().position(|k| k == wanted) { return Some(idx); }
            // 2) case-insensitive
            let wanted_lower = wanted.to_lowercase();
            if let Some((idx, _)) = layer_cache.channel_names.iter().enumerate().find(|(_, k)| k.to_lowercase() == wanted_lower) { return Some(idx); }
            // 3) według kanonicznego skrótu R/G/B/A
            let wanted_canon = channel_alias_to_short(wanted).to_ascii_uppercase();
            if let Some((idx, _)) = layer_cache.channel_names.iter().enumerate().find(|(_, k)| channel_alias_to_short(k).to_ascii_uppercase() == wanted_canon) { return Some(idx); }
            None
        };

        // Specjalne traktowanie Depth
        let wanted_upper = channel_short.to_ascii_uppercase();
        let is_depth = wanted_upper == "Z" || wanted_upper.contains("DEPTH");

        let channel_index_opt = if is_depth {
            // Preferuj dokładnie "Z"; w razie braku wybierz kanał zawierający "DEPTH" albo "DISTANCE"
            find_channel_index("Z").or_else(|| {
                layer_cache
                    .channel_names
                    .iter()
                    .position(|k| k.to_ascii_uppercase().contains("DEPTH") || k.to_ascii_uppercase() == "DISTANCE")
            })
        } else {
            find_channel_index(channel_short)
        };

        let channel_index = channel_index_opt
            .ok_or_else(|| anyhow::anyhow!(format!("Nie znaleziono kanału '{}' w warstwie '{}'", channel_short, layer_cache.layer_name)))?;

        let base = channel_index * pixel_count;
        let channel_slice = &layer_cache.channel_data[base..base + pixel_count];

        let mut out: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(pixel_count);
        for &v in channel_slice.iter() {
            out.push((v, v, v, 1.0));
        }

        self.raw_pixels = out;
        self.width = layer_cache.width;
        self.height = layer_cache.height;
        self.current_layer_name = layer_cache.layer_name.clone();
        Ok(())
    }

    /// Specjalne renderowanie głębi: auto-normalizacja percentylowa + opcjonalne odwrócenie
    pub fn process_depth_image_with_progress(&self, invert: bool, progress: Option<&dyn ProgressSink>) -> Image {
        if let Some(p) = progress { p.start_indeterminate(Some("Processing depth data...")); }
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(self.width, self.height);
        let slice = buffer.make_mut_slice();

        // Wyciągnij z surowych pikseli jeden kanał (zakładamy, że R=G=B=val)
        let mut values: Vec<f32> = self.raw_pixels.iter().map(|(r, _g, _b, _a)| *r).collect();
        if values.is_empty() {
            return Image::from_rgba8(buffer);
        }

        // Policz percentyle 1% i 99% (odporne na outliery) w ~O(n)
        use std::cmp::Ordering;
        let len = values.len();
        let p_lo_idx = ((len as f32) * 0.01).floor() as usize;
        let mut p_hi_idx = ((len as f32) * 0.99).ceil() as isize - 1;
        if p_hi_idx < 0 { p_hi_idx = 0; }
        let p_hi_idx = (p_hi_idx as usize).min(len - 1);
        let (_, lo_ref, _) = values.select_nth_unstable_by(p_lo_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let mut lo = *lo_ref;
        let (_, hi_ref, _) = values.select_nth_unstable_by(p_hi_idx, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        let mut hi = *hi_ref;
        if let Some(p) = progress { p.set(0.4, Some("Computing percentiles...")); }
        if !lo.is_finite() || !hi.is_finite() || (hi - lo).abs() < 1e-20 {
            // Fallback do min/max jeśli degeneracja lub NaN/Inf
            let mut min_v = f32::INFINITY;
            let mut max_v = f32::NEG_INFINITY;
            for &v in &values {
                let nv = if v.is_finite() { v } else { 0.0 };
                if nv < min_v { min_v = nv; }
                if nv > max_v { max_v = nv; }
            }
            lo = min_v;
            hi = max_v;
        }
        if (hi - lo).abs() < 1e-12 {
            hi = lo + 1.0;
        }

        let map_val = |v: f32| -> u8 {
            let mut t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            if invert { t = 1.0 - t; }
            (t * 255.0).round().clamp(0.0, 255.0) as u8
        };

        if let Some(p) = progress { p.set(0.8, Some("Rendering depth image...")); }
        self.raw_pixels.par_iter().zip(slice.par_iter_mut()).for_each(|(&(r, _g, _b, _a), out)| {
            let g8 = map_val(r);
            *out = Rgba8Pixel { r: g8, g: g8, b: g8, a: 255 };
        });

        if let Some(p) = progress { p.finish(Some("Depth processed")); }
        Image::from_rgba8(buffer)
    }
    // uproszczono API: używaj `process_depth_image_with_progress` bezpośrednio

    // usunięto: specjalny preview Cryptomatte
}

/// Hashuje identyfikator z cryptomatte (f32 bit pattern) do stabilnego koloru w 0..1
// usunięto: hash_id_to_color

/// Buduje kolorowy preview dla warstwy Cryptomatte, łącząc pary (id, coverage)
// usunięto: funkcja preview warstwy Cryptomatte

// (usunięto) stary loader pojedynczego kanału – zastąpiony cachingiem wszystkich kanałów warstwy

// Pomocnicze: wczytuje wszystkie kanały dla wybranej warstwy do pamięci (bez dalszego I/O przy przełączaniu)
pub(crate) fn load_all_channels_for_layer_from_full(
    full: &Arc<FullExrCacheData>,
    layer_name: &str,
    _progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<LayerChannels> {
    if let Some(p) = _progress { p.start_indeterminate(Some("Reading layer channels...")); }

    let wanted_lower = layer_name.to_lowercase();

    for layer in full.layers.iter() {
        let lname_lower = layer.name.to_lowercase();
        let matches = if wanted_lower.is_empty() && lname_lower.is_empty() {
            true
        } else if wanted_lower.is_empty() || lname_lower.is_empty() {
            false
        } else {
            lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
        };
        if matches {
            if let Some(p) = _progress { p.set(0.35, Some("Copying channel data...")); }
            let channel_names = layer.channel_names.clone();
            // Zmieniono: Utwórz Arc<[f32]> z istniejących danych
            let channel_data = Arc::from(layer.channel_data.as_slice());
            if let Some(p) = _progress { p.finish(Some("Layer channels loaded")); }
            return Ok(LayerChannels { layer_name: layer_name.to_string(), width: layer.width, height: layer.height, channel_names, channel_data });
        }
    }

    if let Some(p) = _progress { p.reset(); }
    anyhow::bail!(format!("Nie znaleziono warstwy '{}'", layer_name))
}

/// Zachowany wariant czytający z dysku (używany w ścieżkach niezależnych od globalnego cache)
#[allow(dead_code)]
pub(crate) fn load_all_channels_for_layer(
    path: &PathBuf,
    layer_name: &str,
    _progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<LayerChannels> {
    let any_image = exr::read_all_flat_layers_from_file(path)?;
    let wanted_lower = layer_name.to_lowercase();
    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);
        let base_attr: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());
        let lname_lower = base_attr.as_deref().unwrap_or("").to_lowercase();
        let matches = if wanted_lower.is_empty() && lname_lower.is_empty() {
            true
        } else if wanted_lower.is_empty() || lname_lower.is_empty() {
            false
        } else {
            lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
        };
        if matches {
            let num_channels = layer.channel_data.list.len();
            let mut channel_names: Vec<String> = Vec::with_capacity(num_channels);
            let mut channel_data_vec: Vec<f32> = Vec::with_capacity(pixel_count * num_channels); // Temporary Vec
            for (ci, ch) in layer.channel_data.list.iter().enumerate() {
                let full = ch.name.to_string();
                let (_lname, short) = split_layer_and_short(&full, base_attr.as_deref());
                channel_names.push(short);
                for i in 0..pixel_count { channel_data_vec.push(layer.channel_data.list[ci].sample_data.value_by_flat_index(i).to_f32()); }
            }
            let channel_data = Arc::from(channel_data_vec.into_boxed_slice()); // Convert Vec to Arc<[f32]>
            return Ok(LayerChannels { layer_name: layer_name.to_string(), width, height, channel_names, channel_data });
        }
    }
    anyhow::bail!(format!("Nie znaleziono warstwy '{}'", layer_name))
}

// Pomocnicze: buduje kompozyt RGB z mapy kanałów
fn compose_composite_from_channels(layer_channels: &LayerChannels) -> Vec<(f32, f32, f32, f32)> {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    let mut out: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(pixel_count);

    // Heurystyki: najpierw dokładne R/G/B/A, potem nazwy zaczynające się od R/G/B, a na końcu pierwszy dostępny kanał
    let pick_exact_index = |name: &str| -> Option<usize> { layer_channels.channel_names.iter().position(|n| n == name) };
    let pick_prefix_index = |prefix: char| -> Option<usize> {
        let prefix = prefix.to_ascii_uppercase();
        layer_channels.channel_names.iter().position(|n| n.to_ascii_uppercase().starts_with(prefix))
    };

    let r_idx = pick_exact_index("R").or_else(|| pick_prefix_index('R')).unwrap_or(0);
    let g_idx = pick_exact_index("G").or_else(|| pick_prefix_index('G')).unwrap_or(r_idx);
    let b_idx = pick_exact_index("B").or_else(|| pick_prefix_index('B')).unwrap_or(g_idx);
    let a_idx = pick_exact_index("A").or_else(|| pick_prefix_index('A'));

    let base_r = r_idx * pixel_count;
    let base_g = g_idx * pixel_count;
    let base_b = b_idx * pixel_count;
    let a_base_opt = a_idx.map(|ai| ai * pixel_count);

    for i in 0..pixel_count {
        let rr = layer_channels.channel_data[base_r + i];
        let gg = layer_channels.channel_data[base_g + i];
        let bb = layer_channels.channel_data[base_b + i];
        let aa = a_base_opt.map(|ab| layer_channels.channel_data[ab + i]).unwrap_or(1.0);
        out.push((rr, gg, bb, aa));
    }

    out
}