# Analiza wspomagania GPU w aplikacji EXRuster

## Stan obecny

### Komponenty związane z GPU
1. **gpu_context.rs** - Kontekst GPU używający biblioteki wgpu 26.0.1
2. **shaders/image_processing.wgsl** - Compute shader do przetwarzania obrazów
3. **image_cache.rs** - Implementacja GPU processing w funkcji `gpu_process_rgba_f32_to_rgba8`
4. **main.rs** - Inicjalizacja GPU w osobnym wątku
5. **ui_handlers.rs** - Globalne zarządzanie stanem GPU

### Obecne wykorzystanie GPU

#### Zaimplementowane funkcje
- **Inicjalizacja GPU**: Asynchroniczne tworzenie kontekstu wgpu z obsługą błędów
- **Tone mapping na GPU**: Compute shader obsługuje ACES, Reinhard i Linear tone mapping
- **Korekcja ekspozycji i gamma**: Pełny pipeline przetwarzania HDR → LDR na GPU
- **Transformacje kolorów**: Macierz transformacji kolorów w shaderze
- **Fallback na CPU**: Automatyczne przejście na tryb CPU przy braku GPU

#### Architektura GPU
- **Workgroup size**: 8×8×1 (64 wątki na grupę roboczą)
- **Format wejściowy**: RGBA f32 (HDR)
- **Format wyjściowy**: packed RGBA u32 (8-bit na kanał)
- **Buffery**: Input/Output storage + uniform parameters + staging dla odczytu

## Obszary do optymalizacji

### 1. Rozszerzenie użycia GPU

#### Obecnie tylko na CPU
- **Generowanie miniaturek**: `src/thumbnails.rs` - tylko CPU/SIMD
- **Skalowanie obrazów**: MIP levels w `image_cache.rs` - jednowątkowe CPU
- **Konwersja formatów**: Eksport do TIFF/PNG - tylko CPU
- **Wczytywanie EXR**: Dekompresja i konwersja danych - tylko CPU

#### Potencjalne obszary GPU
```rust
// Miniaturki - można zrównoleglić na GPU
fn generate_thumbnail_gpu(width: u32, height: u32, target_size: u32) -> Result<Vec<u8>>

// Skalowanie z filtrowaniem
fn downscale_gpu(pixels: &[f32], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<f32>

// Konwersja kolorów dla całych warstw
fn apply_color_space_transform_gpu(pixels: &[f32], matrix: Mat3) -> Vec<f32>
```

### 2. Optymalizacje wydajności

#### Memory Management
- **Problem**: Każde wywołanie GPU tworzy nowe buffery
- **Rozwiązanie**: Pool buforów wielokrotnego użytku
```rust
struct GpuBufferPool {
    input_buffers: Vec<wgpu::Buffer>,
    output_buffers: Vec<wgpu::Buffer>,
    staging_buffers: Vec<wgpu::Buffer>,
}
```

#### Pipeline Caching
- **Problem**: Shader i pipeline są przebudowywane przy każdym użyciu
- **Rozwiązanie**: Cache skompilowanych pipeline'ów w GpuContext

#### Asynchronous Processing
- **Problem**: GPU processing jest synchroniczny (blocking)
- **Rozwiązanie**: Asynchroniczny workflow z kolejkami zadań

### 3. Nowe funkcje GPU

#### Filtry i efekty
- **Blur/Sharpen**: Filtry konwolucyjne
- **Histogram**: Obliczanie histogramu na GPU
- **Color grading**: Zaawansowane korekcje kolorów
- **Denoising**: Algorytmy redukcji szumu

#### Zaawansowane tone mapping
- **Filmic tone mapping**: Bardziej zaawansowane krzywe
- **Local adaptation**: Tone mapping uwzględniający lokalny kontrast
- **Exposure fusion**: HDR z wielu ekspozycji

## Rekomendacje implementacji

### Faza 1: Optymalizacje podstawowe
1. **Buffer pooling** - zmniejszenie alokacji pamięci GPU
2. **Pipeline caching** - przyspieszenie tworzenia compute pass
3. **Async GPU processing** - nieblokujące przetwarzanie

### Faza 2: Rozszerzenie funkcjonalności  
1. **GPU thumbnail generation** - przyspieszenie generowania miniaturek
2. **GPU image scaling** - szybkie skalowanie MIP levels
3. **Batch processing** - przetwarzanie wielu obrazów jednocześnie

### Faza 3: Zaawansowane funkcje
1. **Advanced tone mapping** - nowe algorytmy tone mappingu
2. **Real-time filters** - filtry stosowane w czasie rzeczywistym
3. **GPU-accelerated export** - przyspieszenie eksportu do różnych formatów

### Wskaźniki wydajności
- **Obecne GPU usage**: ~30% możliwości (tylko tone mapping)
- **Potencjalne przyspieszenie**: 3-5x dla miniaturek, 10-20x dla batch processing
- **Memory efficiency**: Możliwość zmniejszenia użycia RAM o ~40% przy większych obrazach

## Uwagi techniczne

### Ograniczenia wgpu 26.0.1
- Brak compute shader'ów w WebGL (tylko native)
- Limity rozmiaru workgroup zależne od GPU
- Konieczność obsługi różnych backend'ów (Vulkan, DirectX, Metal)

### Kompatybilność
- **Windows**: DirectX 12, Vulkan
- **Fallback**: Zawsze dostępny tryb CPU z SIMD
- **GPU detection**: Automatyczne wykrywanie możliwości sprzętu

### Bezpieczeństwo
- Wszystkie buffery GPU są walidowane pod kątem rozmiaru
- Obsługa błędów GPU z graceful fallback na CPU
- Zabezpieczenia przed NaN/Inf w compute shaderach