# Plan implementacji optymalizacji GPU

Poniższy dokument zawiera szczegółowe kroki implementacyjne dotyczące optymalizacji potoku renderującego GPU, zgodnie z "Etapem 1" z `RAPORT_OPTYMALIZACJI.md`.

---

### 1. Modyfikacja `shaders/image_processing.wgsl`

**Cel:** Uproszczenie i optymalizacja kodu shadera.

**Zadania:**

W pliku `src/shaders/image_processing.wgsl` należy zmodyfikować następujące funkcje, aby były bardziej idiomatyczne i potencjalnie wydajniejsze.

#### a. Funkcja `srgb_oetf`

Zastąp obecną implementację, która używa `if`, wersją opartą w pełni na `select` i `clamp`.

**Kod przed zmianą:**
```wgsl
fn srgb_oetf(x: f32) -> f32 {
    let x = select(0.0, select(1.0, x, x < 1.0), x > 0.0);
    if (x <= 0.0031308) {
        return 12.92 * x;
    } else {
        return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
    }
}
```

**Kod po zmianie:**
```wgsl
fn srgb_oetf(x: f32) -> f32 {
    let x_clamped = clamp(x, 0.0, 1.0);
    let linear_part = 12.92 * x_clamped;
    let nonlinear_part = 1.055 * pow(x_clamped, 1.0 / 2.4) - 0.055;
    return select(nonlinear_part, linear_part, x_clamped <= 0.0031308);
}
```

#### b. Funkcja `apply_gamma`

Uprość funkcję, usuwając zbędne wywołanie `select` na rzecz `clamp`.

**Kod przed zmianą:**
```wgsl
fn apply_gamma(x: f32, gamma_inv: f32) -> f32 {
    let x = select(0.0, select(1.0, x, x < 1.0), x > 0.0);
    return pow(x, gamma_inv);
}
```

**Kod po zmianie:**
```wgsl
fn apply_gamma(x: f32, gamma_inv: f32) -> f32 {
    return pow(clamp(x, 0.0, 1.0), gamma_inv);
}
```

#### c. Funkcja `aces_tonemap`

Uprość obsługę przypadków brzegowych (NaN, ujemne wartości, dzielenie przez zero).

**Kod przed zmianą:**
```wgsl
fn aces_tonemap(x: f32) -> f32 {
    if (x != x || x < 0.0) {
        return 0.0;
    }
    // ...
    let denominator = x * (c * x + d) + e;
    if (abs(denominator) < 1e-10) {
        return 0.0;
    }
    let result = (x * (a * x + b)) / denominator;
    return clamp(result, 0.0, 1.0);
}
```

**Kod po zmianie:**
```wgsl
fn aces_tonemap(x: f32) -> f32 {
    let x_safe = max(x, 0.0); // Zabezpiecza przed wartościami ujemnymi
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let numerator = x_safe * (a * x_safe + b);
    let denominator = x_safe * (c * x_safe + d) + e;
    // Dodaj epsilon, aby uniknąć dzielenia przez zero
    return clamp(numerator / (denominator + 1e-9), 0.0, 1.0);
}
```

---

### 2. Modyfikacja `image_cache.rs`

**Cel:** Zaimplementowanie mechanizmu ponownego wykorzystania zasobów GPU w `ImageCache`, aby uniknąć ich tworzenia przy każdej aktualizacji podglądu.

**Zadania:**

#### a. Rozszerzenie struktury `ImageCache`

Dodaj pola do przechowywania stanu GPU. Zmień też `raw_pixels` na `Vec<f32>`.

**Kod przed zmianą:**
```rust
pub struct ImageCache {
    pub raw_pixels: Vec<(f32, f32, f32, f32)>,
    // ... reszta pól
    gpu_context: Option<Arc<Mutex<Option<crate::gpu_context::GpuContext>>>>,
}
```

**Kod po zmianie:**
```rust
// Dodaj na górze pliku
use wgpu::{ComputePipeline, BindGroupLayout, Buffer};

// Zmodyfikuj strukturę
pub struct ImageCache {
    pub raw_pixels: Vec<f32>, // Zmiana z Vec<(f32,f32,f32,f32)> na Vec<f32>
    pub width: u32,
    pub height: u32,
    pub layers_info: Vec<LayerInfo>,
    pub current_layer_name: String,
    color_matrix_rgb_to_srgb: Option<Mat3>,
    pub current_layer_channels: Option<LayerChannels>,
    full_cache: Arc<FullExrCacheData>,
    mip_levels: Vec<MipLevel>,
    gpu_context: Option<Arc<Mutex<Option<crate::gpu_context::GpuContext>>>>,

    // Nowe pola do cachowania zasobów GPU
    gpu_pipeline: Option<ComputePipeline>,
    gpu_bind_group_layout: Option<BindGroupLayout>,
    gpu_input_buffer: Option<Buffer>,
    gpu_output_buffer: Option<Buffer>,
    gpu_uniform_buffer: Option<Buffer>,
}
```
*Uwaga: Po zmianie `raw_pixels` trzeba będzie dostosować wszystkie miejsca w kodzie, które z niego korzystają, m.in. `process_to_image`, `process_to_thumbnail`, `compose_composite_from_channels`.*

#### b. Modyfikacja `ImageCache::new_with_full_cache`

Zainicjalizuj nowe pola GPU jako `None`.

**Kod po zmianie (fragment konstruktora):**
```rust
// ... pod koniec konstruktora
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
    // Zainicjalizuj nowe pola
    gpu_pipeline: None,
    gpu_bind_group_layout: None,
    gpu_input_buffer: None,
    gpu_output_buffer: None,
    gpu_uniform_buffer: None,
})
```

#### c. Refaktoryzacja `process_to_image_gpu_internal`

Przebuduj funkcję, aby tworzyła zasoby GPU tylko raz.

**Logika po zmianie:**
1.  Funkcja powinna stać się `&mut self`, aby mogła modyfikować stan cache'u GPU.
2.  Przy pierwszym wywołaniu (np. `self.gpu_pipeline.is_none()`):
    *   Stwórz `ComputePipeline`, `BindGroupLayout`, `Buffer` dla uniformów, wejścia i wyjścia.
    *   Upewnij się, że bufory mają odpowiedni rozmiar dla bieżącego obrazu.
    *   Zapisz wszystkie te zasoby w polach `self.gpu_...`.
3.  Przy każdym kolejnym wywołaniu:
    *   Sprawdź, czy rozmiar obrazu się nie zmienił. Jeśli tak, musisz odtworzyć bufory wejściowy i wyjściowy.
    *   Użyj `gpu_context.queue.write_buffer(...)`, aby zaktualizować dane w istniejących `gpu_input_buffer` i `gpu_uniform_buffer`.
    *   Stwórz `BindGroup`, używając istniejącego `gpu_bind_group_layout` i zaktualizowanych buforów.
    *   Wykonaj `compute_pass`, używając istniejącego `gpu_pipeline`.
4.  Usuń nadmiarowe logowanie i skomplikowane sprawdzanie błędów na rzecz prostszej struktury.

---

### 3. Modyfikacja `gpu_thumbnails.rs`

**Cel:** Analogiczna optymalizacja dla procesora miniaturek.

**Zadania:**

#### a. Rozszerzenie struktury `GpuThumbnailProcessor`

Dodaj pola na reużywalne bufory.

**Kod po zmianie:**
```rust
use wgpu::{ComputePipeline, BindGroupLayout, Buffer}; // Dodaj Buffer

pub struct GpuThumbnailProcessor {
    gpu_context: GpuContext,
    compute_pipeline: ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    // Nowe pola
    input_buffer: Option<Buffer>,
    output_buffer: Option<Buffer>,
    uniform_buffer: Option<Buffer>,
    staging_buffer: Option<Buffer>,
}
```

#### b. Modyfikacja `GpuThumbnailProcessor::new`

Zainicjalizuj nowe pola.

**Kod po zmianie (fragment konstruktora):**
```rust
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
```

#### c. Refaktoryzacja `process_thumbnail`

Przebuduj funkcję, aby stała się `&mut self` i reużywała bufory.

**Logika po zmianie:**
1.  Sprawdź, czy istniejące bufory (`self.input_buffer` itd.) są wystarczająco duże dla bieżącego zadania.
2.  Jeśli bufor nie istnieje lub jest za mały, stwórz go na nowo i zapisz w `self`.
3.  Jeśli bufor istnieje i jest wystarczająco duży, użyj go ponownie.
4.  Zaktualizuj zawartość buforów za pomocą `queue.write_buffer`.
5.  Kontynuuj z resztą logiki (tworzenie `BindGroup`, `compute_pass` itd.).
