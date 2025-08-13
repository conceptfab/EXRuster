## Plan wdrożenia wgpu w EXRuster

### Cele

- **przyspieszyć podgląd i operacje** na dużych obrazach EXR (4K–16K+)
- **zachować identyczny wygląd** (ACES/Reinhard/Linear + sRGB/gamma) jak w ścieżce CPU
- **zapewnić fallback CPU** i kontrolę feature‑flagą, aby nie ryzykować regresji

### Zakres wersji 1.0 (MVP GPU)

- compute: ekspozycja → tone‑map (ACES/Reinhard/Linear) → sRGB/gamma
- opcjonalna macierz kolorów 3×3 (primaries→sRGB) w shaderze
- GPU downscale dla miniaturek (box/bilinear)
- próg użycia GPU zależny od wielkości renderu; pełny fallback CPU
- podstawowa telemetria i testy zgodności obrazu GPU vs CPU

### Architektura (zarys)

- nowy moduł `src/gpu/` z inicjalizacją `wgpu::Device/Queue` (lazy, `OnceLock`)
- pipelines:
  - `tonemap_rgba32f_to_rgba8` (podgląd główny)
  - `downscale_box_rgba32f` (miniatury/MIP)
  - (później) `histogram_luma` + redukcja/prefix‑sum (percentyle/auto‑eksponowanie)
- wejście: bufor `RGBA32F` (po wczytaniu EXR i ewentualnej macierzy kolorów)
- wyjście: bufor `RGBA8` do `slint::Image::from_rgba8`
- parametry per‑dispatch: exposure, gamma, tonemap_mode, macierz 3×3 (uniform/push constants)
- próg GPU: używamy GPU, gdy liczba pikseli docelowego renderu > ~1–2M (konfigurowalne)
- fallback: brak GPU/device‑lost/mały obraz → ścieżka CPU SIMD

### Etapy wdrożenia

- **Etap 0: Przygotowanie**

  - dodać zależności (`wgpu`, `pollster`), feature `gpu` w `Cargo.toml`
  - detekcja backendu (Windows: D3D12, fallback Vulkan)
  - kryteria: kompiluje się z `--features gpu`; brak regresji bez flagi

- **Etap 1: Kontekst GPU**

  - `GpuContext` z inicjalizacją adaptera/device/queue + wybór backendu
  - metody: `is_available()`, `device()`, `queue()`; `OnceLock`/singleton
  - env/flag: `EXRUSTER_GPU=0` wymusza CPU
  - kryteria: kontekst inicjuje się jednokrotnie i zamyka bez wycieków

- **Etap 2: Tone‑mapping preview (MVP)**

  - WGSL: `tonemap_rgba32f_to_rgba8.wgsl` (ekspozycja → ACES/Reinhard/Linear → sRGB/gamma)
  - API: `try_tonemap_gpu(rgba32f, w, h, params) -> Option<Vec<u8>>`
  - integracja w `process_to_image`: jeśli GPU dostępne i warto, uruchom GPU; inaczej CPU
  - kryteria: zgodność wizualna (maks. błąd kanałowy ≤ 1 LSB), przyspieszenie ≥2× dla ≥4K

- **Etap 3: Miniatury i downscale**

  - WGSL: `downscale_box_rgba32f.wgsl` (box/bilinear); opcjonalnie łańcuch MIP
  - integracja w `process_to_thumbnail` i `thumbnails::generate_exr_thumbnails_in_dir`
  - kryteria: ≥3× szybciej dla batchu 50+ plików 8–16K; brak regresji jakościowej

- **Etap 4: Histogram/percentyle/auto‑ekspozycja (opcjonalnie priorytet 2)**

  - WGSL: `histogram_luma.wgsl` + redukcja/prefix‑sum po stronie GPU/CPU
  - integracja z `process_depth_image_with_progress` i opcją auto‑exposure
  - kryteria: wyznaczenie 1%/99% w <20 ms dla 8K

- **Etap 5: MIP/tile streaming (opcjonalne)**

  - prekomputacja MIP na GPU, render z najbliższego poziomu do rozmiaru widżetu
  - kryteria: minimalizacja readback, płynność przy zmianie rozmiaru okna

- **Etap 6: Kolor/metadane**

  - macierz 3×3 w shaderze; ewentualnie CLUT 3D (później)
  - kryteria: zgodność z CPU macierzy primaries→sRGB

- **Etap 7: Telemetria i QA**

  - pomiar czasu (GPU/CPU), bajty przeniesione, readback
  - testy golden images i property‑based dla ACES/Reinhard
  - kryteria: brak rozjazdów > ustalone progi

- **Etap 8: Stabilizacja i DX**
  - obsługa device‑lost, timeouts, logowanie błędów
  - dokumentacja i (opcjonalnie) flaga w UI “GPU preview”
  - kryteria: brak crashy; jasne logi i powrót do CPU

### Zmiany w strukturze projektu

- nowe pliki:

  - `src/gpu/mod.rs`
  - `src/gpu/context.rs`
  - `src/gpu/pipelines.rs`
  - `src/gpu/shaders/tonemap_rgba32f_to_rgba8.wgsl`
  - `src/gpu/shaders/downscale_box_rgba32f.wgsl`
  - (później) `src/gpu/shaders/histogram_luma.wgsl`

- modyfikacje:
  - `Cargo.toml`: feature `gpu`, zależności `wgpu`, `pollster`
  - `src/image_cache.rs`: warunkowe użycie GPU w `process_to_image`/`process_to_thumbnail`
  - `src/thumbnails.rs`: opcjonalne wywołania GPU dla batcha miniaturek

### API (propozycja)

```rust
pub struct GpuContext { /* adapter, device, queue, limits, features */ }
pub fn gpu_context() -> Option<&'static GpuContext>; // None => fallback CPU

pub struct TonemapParams {
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: i32, // 0=ACES,1=Reinhard,2=Linear
    pub color_matrix_3x3: Option<[f32; 9]>,
}

pub fn try_tonemap_gpu(
    rgba32f: &[f32],
    width: u32,
    height: u32,
    params: &TonemapParams,
) -> Option<Vec<u8>>;

pub fn try_downscale_gpu(
    rgba32f: &[f32],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    params: &TonemapParams,
) -> Option<Vec<u8>>;
```

### WGSL (szkic – tonemap)

```wgsl
struct Params {
  exposure: f32,
  gamma: f32,
  tonemap_mode: u32,
  use_matrix: u32,
  color_m: mat3x3<f32>,
};
@group(0) @binding(0) var<storage, read>  in_pixels: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> out_pixels: array<vec4<u32>>;
@group(0) @binding(2) var<uniform> U: Params;

fn aces(x: f32) -> f32 {
  let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
  return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

fn reinhard(x: f32) -> f32 { return clamp(x / (1.0 + x), 0.0, 1.0); }

fn srgb_oetf(x: f32) -> f32 {
  return select(1.055 * pow(x, 1.0/2.4) - 0.055, 12.92 * x, x <= 0.0031308);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let i = id.x;
  if (i >= arrayLength(&in_pixels)) { return; }
  var rgba = in_pixels[i];

  if (U.use_matrix != 0u) {
    let rgb = U.color_m * rgba.xyz;
    rgba = vec4<f32>(rgb, rgba.w);
  }

  let mul = pow(2.0, U.exposure);
  var r = max(rgba.x * mul, 0.0);
  var g = max(rgba.y * mul, 0.0);
  var b = max(rgba.z * mul, 0.0);

  if (U.tonemap_mode == 0u) { r = aces(r); g = aces(g); b = aces(b); }
  else if (U.tonemap_mode == 1u) { r = reinhard(r); g = reinhard(g); b = reinhard(b); }
  else { r = clamp(r, 0.0, 1.0); g = clamp(g, 0.0, 1.0); b = clamp(b, 0.0, 1.0); }

  r = srgb_oetf(r); g = srgb_oetf(g); b = srgb_oetf(b);

  let a = clamp(rgba.w, 0.0, 1.0);
  out_pixels[i] = vec4<u32>(
    u32(round(r * 255.0)), u32(round(g * 255.0)), u32(round(b * 255.0)), u32(round(a * 255.0))
  );
}
```

### Testy i akceptacja

- golden images: porównanie GPU vs CPU (SSIM ≥ 0.999 lub maks. błąd kanałowy ≤ 1)
- benchmarki: 4K/8K/16K – czas renderu podglądu i generacja miniaturek batch
- stabilność: testy device‑lost i readback (timeout/retry, automatyczny fallback)

### Ryzyka i mitigacje

- koszt readback dla małych obrazów → próg GPU + adaptacyjne decyzje
- rozbieżności numeryczne float → tolerancje i testy
- device‑lost/kompatybilność backendów → obsługa błędów + powrót do CPU

### Szacunki

- Etap 0–1: 0.5–1 dnia
- Etap 2 (MVP preview): 1–2 dni
- Etap 3 (miniatury): 1 dzień
- Etap 4 (histogram/percentyle): 1–2 dni
- Etapy 5–6: opcjonalnie wg priorytetu

### Konfiguracja/feature‑flagi

- Cargo: `--features gpu`
- Env: `EXRUSTER_GPU=0|1`
- (opcjonalnie) UI: przełącznik “GPU preview (beta)”
