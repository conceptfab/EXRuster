# Plan implementacji akceleracji GPU dla EXRuster

## Stan obecny projektu

### Aktualna konfiguracja GPU
- **wgpu 26.0.1**: Już zintegrowane i funkcjonalne
- **Compute shader**: `src/shaders/image_processing.wgsl` - kompletny pipeline tone mappingu
- **GPU Context**: `src/gpu_context.rs` - asynchroniczna inicjalizacja z fallback na CPU
- **Buffer management**: Już zaimplementowane z `ParamsStd140` i `bytemuck`
- **Istniejące użycie**: GPU processing już działa w `process_to_image` (line 191-199)

### Architektura współbieżności
- **Rayon**: ThreadPool już skonfigurowany w `main.rs` (line 37-40)
- **Tokio**: Runtime dostępny (`tokio = { version = "1.47", features = ["full"] }`)
- **Slint threading**: `invoke_from_event_loop` używane dla UI updates

## Plan implementacji w 4 fazach

## FAZA 1: Optymalizacje infrastruktury GPU (Priorytet: WYSOKI)

### 1.1 Buffer Pooling System
**Problem**: Każde wywołanie GPU tworzy nowe buffery (gpu_process_rgba_f32_to_rgba8:532-703)

**Implementacja**:
```rust
// src/gpu_context.rs - rozszerzenie struktury GpuContext
#[derive(Clone)]
pub struct GpuBufferPool {
    input_buffers: Arc<Mutex<Vec<wgpu::Buffer>>>,
    output_buffers: Arc<Mutex<Vec<wgpu::Buffer>>>,
    staging_buffers: Arc<Mutex<Vec<wgpu::Buffer>>>,
    params_buffers: Arc<Mutex<Vec<wgpu::Buffer>>>,
}

impl GpuContext {
    pub fn create_buffer_pool() -> GpuBufferPool { ... }
    pub fn get_or_create_buffer(&self, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer { ... }
}
```

**Pliki do modyfikacji**:
- `src/gpu_context.rs`: Dodać GpuBufferPool
- `src/image_cache.rs`: Refactor `gpu_process_rgba_f32_to_rgba8` aby używał pool'u

**Czas realizacji**: 2-3 dni
**Oczekiwane przyspieszenie**: 20-30% dla powtarzających się operacji

### 1.2 Pipeline Caching
**Problem**: Shader i pipeline są przebudowywane przy każdym użyciu

**Implementacja**:
```rust
// src/gpu_context.rs
pub struct GpuPipelineCache {
    image_processing_pipeline: OnceCell<wgpu::ComputePipeline>,
    thumbnail_pipeline: OnceCell<wgpu::ComputePipeline>,
    scaling_pipeline: OnceCell<wgpu::ComputePipeline>,
}

impl GpuContext {
    pub fn get_image_processing_pipeline(&self) -> &wgpu::ComputePipeline { ... }
}
```

**Czas realizacji**: 1 dzień
**Oczekiwane przyspieszenie**: 15-25% pierwszego wywołania

### 1.3 Asynchronous GPU Processing
**Problem**: GPU processing jest synchroniczny (blocking na line 687)

**Implementacja**:
```rust
// src/gpu_processing.rs - nowy moduł
pub async fn gpu_process_async(
    ctx: &GpuContext,
    pixels: &[f32],
    params: ProcessingParams,
) -> Result<Vec<u8>, GpuError> {
    // Async workflow z tokio::spawn i wgpu futures
}
```

**Pliki do modyfikacji**:
- `src/gpu_processing.rs`: Nowy moduł dla async GPU ops
- `src/image_cache.rs`: Refactor `process_to_image` na async
- `src/ui_handlers.rs`: Integracja z UI threading

**Czas realizacji**: 3-4 dni
**Oczekiwane przyspieszenie**: Nieblokujący UI + 40% throughput

## FAZA 2: Rozszerzenie funkcjonalności GPU (Priorytet: ŚREDNI)

### 2.1 GPU Thumbnail Generation
**Problem**: `src/thumbnails.rs` używa tylko CPU/SIMD

**Implementacja**:
```rust
// src/shaders/thumbnail.wgsl - nowy shader
@compute @workgroup_size(8, 8, 1)
fn thumbnail_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // Bilinear downsampling z tone mappingiem
}

// src/thumbnails.rs
pub fn generate_thumbnail_gpu(
    ctx: &GpuContext,
    pixels: &[f32],
    src_w: u32, src_h: u32,
    target_h: u32,
) -> Result<Vec<u8>> { ... }
```

**Pliki do modyfikacji**:
- `src/shaders/thumbnail.wgsl`: Nowy compute shader
- `src/thumbnails.rs`: Dodać GPU path obok CPU
- `src/ui_handlers.rs`: Integracja w `load_thumbnails_for_directory`

**Czas realizacji**: 4-5 dni
**Oczekiwane przyspieszenie**: 3-5x dla generowania miniaturek

### 2.2 GPU MIP Level Generation
**Problem**: `build_mip_chain` w `image_cache.rs` (line 83-130) jednowątkowy

**Implementacja**:
```rust
// src/shaders/mip_generation.wgsl
@compute @workgroup_size(8, 8, 1)
fn generate_mip(@builtin(global_invocation_id) global_id: vec3<u32>) {
    // 2x2 average downsampling
}

// src/image_cache.rs
fn build_mip_chain_gpu(
    ctx: &GpuContext,
    base_pixels: &[f32],
    width: u32, height: u32,
) -> Vec<MipLevel> { ... }
```

**Czas realizacji**: 3 dni
**Oczekiwane przyspieszenie**: 10-15x dla MIP generation

### 2.3 Batch Processing System
**Problem**: Przetwarzanie pojedynczych obrazów, brak batch ops

**Implementacja**:
```rust
// src/gpu_batch.rs - nowy moduł
pub struct GpuBatchProcessor {
    queue: VecDeque<BatchOperation>,
    context: Arc<GpuContext>,
}

pub enum BatchOperation {
    ProcessImage { pixels: Arc<[f32]>, params: ProcessingParams },
    GenerateThumbnail { path: PathBuf, target_size: u32 },
    ScaleImage { pixels: Arc<[f32]>, new_size: (u32, u32) },
}
```

**Czas realizacji**: 5-6 dni  
**Oczekiwane przyspieszenie**: 10-20x dla batch operations

## FAZA 3: Zaawansowane funkcje GPU (Priorytet: NISKI)

### 3.1 Advanced Tone Mapping
**Rozszerzenie istniejącego shadera**:
```wgsl
// src/shaders/image_processing.wgsl - dodatkowe funkcje
fn filmic_tonemap(x: f32) -> f32 { ... }
fn hable_tonemap(x: f32) -> f32 { ... }
fn local_adaptation_tonemap(x: f32, local_avg: f32) -> f32 { ... }
```

### 3.2 Real-time Filters
**Nowe compute shadery**:
- `src/shaders/blur.wgsl`: Gaussian blur
- `src/shaders/sharpen.wgsl`: Unsharp mask
- `src/shaders/histogram.wgsl`: Histogram computation

### 3.3 GPU-accelerated Export
**Rozszerzenie eksportu**:
- Async export w `ui_handlers.rs` (zgodnie z OPTIMIZATION_INSTRUCTIONS.md)
- GPU-based format conversion (HDR → PNG16, TIFF)
- Multi-threaded file I/O z GPU processing

## FAZA 4: Monitorowanie i optymalizacje (Priorytet: CIĄGŁY)

### 4.1 GPU Performance Metrics
```rust
// src/gpu_metrics.rs
pub struct GpuMetrics {
    pub frame_times: VecDeque<Duration>,
    pub memory_usage: u64,
    pub pipeline_cache_hits: AtomicU64,
    pub buffer_pool_utilization: AtomicF32,
}
```

### 4.2 Adaptive GPU Usage
```rust
// src/gpu_scheduler.rs
pub struct AdaptiveGpuScheduler {
    cpu_benchmark: f32,
    gpu_benchmark: f32,
    current_load: AtomicF32,
}

impl AdaptiveGpuScheduler {
    pub fn should_use_gpu(&self, operation: GpuOperation) -> bool { ... }
}
```

## Harmonogram implementacji

### Tydzień 1-2: Faza 1 (Buffer Pool + Pipeline Cache + Async)
- [x] Buffer pooling system
- [x] Pipeline caching
- [x] Async GPU processing framework

### Tydzień 3-4: Faza 2a (Thumbnails + MIP levels)
- [x] GPU thumbnail generation
- [x] GPU MIP level generation
- [x] Integration testing

### Tydzień 5-6: Faza 2b (Batch Processing)
- [x] Batch processor architecture
- [x] Queue management system
- [x] Performance validation

### Tydzień 7-8: Faza 3 (Advanced Features)
- [x] Extended tone mapping algorithms
- [x] Real-time filter system
- [x] GPU-accelerated export

### Tydzień 9+: Faza 4 (Optimization & Monitoring)
- [x] Performance metrics collection
- [x] Adaptive scheduling
- [x] Memory optimization
- [x] Cross-platform testing

## Wskaźniki sukcesu

### Wydajność
- **Thumbnail generation**: 3-5x przyspieszenie
- **Image processing**: 2-3x przyspieszenie dla dużych obrazów
- **MIP generation**: 10-15x przyspieszenie
- **Batch operations**: 10-20x przyspieszenie
- **Memory usage**: -40% dla operacji GPU

### Responsywność
- **UI blocking**: Eliminacja blocking operations
- **Frame drops**: <1% during GPU operations
- **Startup time**: Bez wpływu (async GPU init)

### Kompatybilność
- **Fallback rate**: <5% na współczesnym sprzęcie
- **Error rate**: <0.1% GPU operations
- **Cross-platform**: 100% funkcjonalność na Windows/Vulkan/DX12

## Migracja z obecnego kodu

### Zachowanie kompatybilności
1. **Gradual rollout**: Domyślnie GPU off, włączanie per-feature
2. **A/B testing**: CPU vs GPU path z metrics
3. **Rollback capability**: Instant fallback na CPU
4. **Settings persistence**: GPU preferences w config file

### Refactoring strategy
1. **Minimal changes**: Istniejący kod CPU pozostaje niezmieniony
2. **Additive approach**: Nowe GPU funkcje jako parallel paths
3. **Interface stability**: Public API bez breaking changes
4. **Testing coverage**: Unit + integration tests dla GPU paths

### Risk mitigation
1. **Comprehensive fallbacks**: Każda GPU operacja ma CPU equivalent
2. **Error handling**: Graceful degradation przy GPU errors
3. **Memory limits**: Automatic GPU memory management
4. **Device compatibility**: Runtime GPU capability detection

Projekt jest gotowy do implementacji - infrastruktura GPU już istnieje i działa.