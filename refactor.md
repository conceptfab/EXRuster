# Plan Refaktoryzacji i Optymalizacji Kodu

Poniżej znajduje się szczegółowa, techniczna instrukcja krok po kroku dotycząca wprowadzenia poprawek optymalizacyjnych zidentyfikowanych w `RAPORT_OPTYMALIZACJI.md`.

---

## 1. Plik: `src/full_exr_cache.rs`

### 1.1. Wydajniejsze budowanie cache'u EXR

**Cel:** Zastąpienie pętli `for` kopiującej piksele pojedynczo metodą `extend`, która operuje na iteratorach, co jest bardziej idiomatyczne i potencjalnie szybsze.

**Instrukcja:**

W funkcji `build_full_exr_cache`, znajdź i zamień ten fragment kodu:

```rust
// Stary kod
for i in 0..pixel_count {
    entry.3.push(layer.channel_data.list[idx].sample_data.value_by_flat_index(i).to_f32());
}
```

Na następujący:

```rust
// Nowy kod
let samples = (0..pixel_count)
    .map(|i| layer.channel_data.list[idx].sample_data.value_by_flat_index(i).to_f32());
entry.3.extend(samples);
```

---

## 2. Plik: `src/image_cache.rs`

### 2.1. Cache'owanie macierzy transformacji kolorów

**Cel:** Uniknięcie wielokrotnego obliczania macierzy kolorów dla tej samej warstwy.

**Instrukcja:**

1.  Dodaj nowe pole do struktury `ImageCache` do przechowywania macierzy dla każdej warstwy:

    ```rust
    // W struct ImageCache
    color_matrices: HashMap<String, Mat3>,
    ```

2.  W funkcji `ImageCache::new_with_full_cache`, zainicjalizuj nową mapę i oblicz macierz tylko dla pierwszej, najlepszej warstwy:

    ```rust
    // W new_with_full_cache
    let mut color_matrices = HashMap::new();
    let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, &best_layer).ok();
    if let Some(matrix) = color_matrix_rgb_to_srgb {
        color_matrices.insert(best_layer.clone(), matrix);
    }

    // ... w return Ok(ImageCache { ... })
    color_matrix_rgb_to_srgb, // zachowaj dla bieżącej warstwy
    color_matrices, // dodaj nowe pole
    // ...
    ```

3.  W funkcji `ImageCache::load_layer`, sprawdź, czy macierz dla danej warstwy jest już w cache'u, zanim ją obliczysz:

    ```rust
    // W load_layer
    if self.color_matrices.contains_key(layer_name) {
        self.color_matrix_rgb_to_srgb = self.color_matrices.get(layer_name).cloned();
    } else {
        self.color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name).ok();
        if let Some(matrix) = self.color_matrix_rgb_to_srgb {
            self.color_matrices.insert(layer_name.to_string(), matrix);
        }
    }
    ```

### 2.2. Równoległe tworzenie kompozytu RGBA

**Cel:** Przyspieszenie funkcji `compose_composite_from_channels` przez zrównoleglenie jej za pomocą `rayon`.

**Instrukcja:**

Zastąp obecną, jednowątkową implementację funkcji `compose_composite_from_channels` następującą wersją zrównolegloną:

```rust
use rayon::prelude::*;

fn compose_composite_from_channels(layer_channels: &LayerChannels) -> Vec<f32> {
    let pixel_count = (layer_channels.width as usize) * (layer_channels.height as usize);
    let mut out: Vec<f32> = vec![0.0; pixel_count * 4];

    // ... (logika pick_exact_index, pick_prefix_index, r_idx, g_idx, b_idx, a_idx pozostaje bez zmian)

    let r_idx = ...;
    let g_idx = ...;
    let b_idx = ...;
    let a_idx = ...;

    let base_r = r_idx * pixel_count;
    let base_g = g_idx * pixel_count;
    let base_b = b_idx * pixel_count;
    let a_base_opt = a_idx.map(|ai| ai * pixel_count);

    let r_plane = &layer_channels.channel_data[base_r..base_r + pixel_count];
    let g_plane = &layer_channels.channel_data[base_g..base_g + pixel_count];
    let b_plane = &layer_channels.channel_data[base_b..base_b + pixel_count];
    let a_plane = a_base_opt.map(|ab| &layer_channels.channel_data[ab..ab + pixel_count]);

    out.par_chunks_mut(4).enumerate().for_each(|(i, chunk)| {
        chunk[0] = r_plane[i];
        chunk[1] = g_plane[i];
        chunk[2] = b_plane[i];
        chunk[3] = if let Some(a) = a_plane { a[i] } else { 1.0 };
    });

    out
}
```

---


### 3.1. Przyspieszenie generowania miniaturek

**Cel:** Zmiana filtra skalującego na szybszy odpowiednik.

**Instrukcja:**

W funkcji `generate_single_exr_thumbnail_work_new`, znajdź linię:

```rust
// Stary kod
let thumbnail = image::imageops::resize(&img, thumb_width, thumb_height, image::imageops::FilterType::Lanczos3);
```

I zamień ją na:

```rust
// Nowy kod
let thumbnail = image::imageops::resize(&img, thumb_width, thumb_height, image::imageops::FilterType::Triangle);
```

---

## 4. Plik: `src/gpu_thumbnails.rs`

### 4.1. Poprawa jakości skalowania w shaderze

**Cel:** Zastąpienie interpolacji bilinearnej uśrednianiem (box filter) dla lepszej jakości miniaturek.

**Instrukcja:**

Zmodyfikuj stałą `THUMBNAIL_COMPUTE_SHADER`. Zastąp fragment odpowiedzialny za interpolację nową logiką uśredniającą.

**Fragment do zastąpienia (od `let scale_x` do `let pixel = mix(p0, p1, fy);`):**

```wgsl
// Stary kod (interpolacja bilinearna)
let scale_x = f32(params.input_width) / f32(params.output_width);
let scale_y = f32(params.input_height) / f32(params.output_height);
// ... (cała logika interpolacji)
let p1 = mix(p01, p11, fx);
let pixel = mix(p0, p1, fy);
```

**Nowy kod (box filter):**

```wgsl
// Nowy kod (uśrednianie 2x2)
let scale_x = f32(params.input_width) / f32(params.output_width);
let scale_y = f32(params.input_height) / f32(params.output_height);

// Współrzędne centralnego piksela w obrazie źródłowym
let src_cx = (f32(x) + 0.5) * scale_x;
let src_cy = (f32(y) + 0.5) * scale_y;

// Współrzędne dla 4 próbek (2x2) wokół centrum
let x0 = u32(floor(src_cx - 0.5));
let x1 = x0 + 1u;
let y0 = u32(floor(src_cy - 0.5));
let y1 = y0 + 1u;

// Funkcja pomocnicza do bezpiecznego pobierania piksela
fn get_pixel(px: u32, py: u32) -> vec4<f32> {
    let safe_x = min(px, params.input_width - 1u);
    let safe_y = min(py, params.input_height - 1u);
    let idx = (safe_y * params.input_width + safe_x) * 4u;
    return vec4<f32>(input_pixels[idx], input_pixels[idx+1], input_pixels[idx+2], input_pixels[idx+3]);
}

// Pobierz 4 piksele i uśrednij
let p00 = get_pixel(x0, y0);
let p10 = get_pixel(x1, y0);
let p01 = get_pixel(x0, y1);
let p11 = get_pixel(x1, y1);

let pixel = (p00 + p10 + p01 + p11) * 0.25;
```

---

## 5. Plik: `src/shaders/image_processing.wgsl`

### 5.1. Dodanie transformacji kolorów w shaderze

**Cel:** Zapewnienie spójności kolorystycznej między podglądem CPU i GPU.

**Instrukcja:**

1.  Dodaj macierz kolorów i flagę jej użycia do struktury `Params`:

    ```wgsl
    struct Params {
        exposure: f32,
        gamma: f32,
        tonemap_mode: u32,
        width: u32,
        height: u32,
        color_matrix: mat3x3<f32>, // Nowe pole
        has_color_matrix: u32,    // Nowe pole (0 lub 1)
    }
    ```
    *(Uwaga: Wymaga to również aktualizacji struktury `Params` po stronie Rust w `image_cache.rs`)*

2.  W funkcji `main` shadera, przed wywołaniem `tone_map_and_gamma`, zastosuj macierz:

    ```wgsl
    // W funkcji main
    let input_pixel = input_pixels[pixel_index];
    var color = input_pixel.rgb;

    if (params.has_color_matrix != 0u) {
        color = params.color_matrix * color;
    }

    let processed_color = tone_map_and_gamma(
        color.r,
        color.g,
        color.b,
        params.exposure,
        params.gamma,
        params.tonemap_mode
    );
    ```

---

## 6. Plik: `src/exr_metadata.rs`

### 6.1. Refaktoryzacja formatowania atrybutów

**Cel:** Usunięcie zduplikowanego kodu formatującego wartości atrybutów.

**Instrukcja:**

1.  Stwórz nową, prywatną funkcję w module:

    ```rust
    fn format_attribute_value(value: &AttributeValue, normalized_key: &str) -> String {
        match value {
            AttributeValue::Chromaticities(ch) => {
                let r = (ch.red.x() as f64, ch.red.y() as f64);
                let g = (ch.green.x() as f64, ch.green.y() as f64);
                let b = (ch.blue.x() as f64, ch.blue.y() as f64);
                let w = (ch.white.x() as f64, ch.white.y() as f64);
                format!(
                    "R: ({:.3},{:.3})  G: ({:.3},{:.3})  B: ({:.3},{:.3})  W: ({:.3},{:.3})",
                    r.0, r.1, g.0, g.1, b.0, b.1, w.0, w.1
                )
            }
            AttributeValue::F32(v) => {
                if normalized_key.eq_ignore_ascii_case("pixel_aspect") {
                    format!("{:.3}", *v as f64)
                } else {
                    format!("{:.3}", *v as f64)
                }
            }
            AttributeValue::F64(v) => {
                if normalized_key.eq_ignore_ascii_case("pixel_aspect") {
                    format!("{:.3}", v)
                } else {
                    format!("{:.3}", v)
                }
            }
            other => format!("{:?}", other),
        }
    }
    ```

2.  W funkcji `read_and_group_metadata`, zastąp zduplikowane bloki `match value { ... }` (zarówno w pętli po `shared.other`, jak i w pętli po `header.own_attributes.other`) wywołaniem nowej funkcji:

    ```rust
    // Przykład zastąpienia
    let pretty_value = format_attribute_value(value, &normalized_key);
    ```

---

## 7. Plik: `src/color_processing.rs`

### 7.1. Uproszczenie konwersji typów w `glam`

**Cel:** Zastąpienie ręcznej konwersji macierzy i wektorów wbudowanymi metodami `glam`.

**Instrukcja:**

1.  W funkcji `rgb_to_xyz_from_primaries`, zmień:

    ```rust
    // Stary kod
    Mat3::from_cols(
        Vec3::new(scaled.x_axis.x as f32, scaled.x_axis.y as f32, scaled.x_axis.z as f32),
        Vec3::new(scaled.y_axis.x as f32, scaled.y_axis.y as f32, scaled.y_axis.z as f32),
        Vec3::new(scaled.z_axis.x as f32, scaled.z_axis.y as f32, scaled.z_axis.z as f32),
    )
    ```

    Na:

    ```rust
    // Nowy kod
    scaled.as_mat3()
    ```

2.  W funkcji `bradford_adaptation_matrix`, zmień:

    ```rust
    // Stary kod
    Mat3::from_cols(
        Vec3::new(tmp.x_axis.x as f32, tmp.x_axis.y as f32, tmp.x_axis.z as f32),
        Vec3::new(tmp.y_axis.x as f32, tmp.y_axis.y as f32, tmp.y_axis.z as f32),
        Vec3::new(tmp.z_axis.x as f32, tmp.z_axis.y as f32, tmp.z_axis.z as f32),
    )
    ```

    Na:

    ```rust
    // Nowy kod
    tmp.as_mat3()
    ```
