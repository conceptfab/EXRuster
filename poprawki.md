# Raport analizy kodu EXRuster

**Data:** 2026-02-15
**Zakres:** Wydajność, martwy kod, zależności, jakość kodu, zarządzanie pamięcią, współbieżność

---

## 1. Problemy wydajnościowe

### 1.1 Pełny odczyt pliku EXR przy każdym ładowaniu warstwy

**Plik:** `src/io/lazy_exr_loader.rs`, linie 175–231

`load_layer_from_disk()` wywołuje `exr::read_all_flat_layers_from_file()` nawet gdy potrzebna jest tylko jedna warstwa. Cały plik jest dekompresowany do pamięci, a następnie przeszukiwany w pętli. Dodatkowo LRU eviction (linia 143) w komentarzu opisany jako "TODO", w praktyce usuwa arbitralny pierwszy klucz z `HashMap` (nie LRU, lecz losowy).

**Priorytet:** Wysoki
**Rozwiązanie:** Użyć selektywnego odczytu warstwy z API `exr` lub strumieniowania; naprawić eviction policy w cache.

---

### 1.2 Ponowne parsowanie nagłówka EXR przy każdym eksporcie

**Plik:** `src/ui/export_handlers.rs`, linie 90 i 293

`export_base_layer_impl()` i `export_layer_group_impl()` wywołują `extract_layers_info(&file_path)`, która ponownie otwiera i parsuje nagłówek EXR. Te dane są już dostępne w `AppState::full_exr_cache`.

**Priorytet:** Średni
**Rozwiązanie:** Użyć danych z istniejącego cache zamiast ponownego odczytu z dysku.

---

### 1.3 `compose_composite_from_channels` działa jednowątkowo na gorącej ścieżce

**Plik:** `src/io/image_cache.rs`, linie 599–611

Wewnętrzna pętla per-pixel jest jednowątkowa. Dla obrazu 4K (8M pikseli) to ~8M iteracji na każdą zmianę warstwy lub parametrów. Analogicznie `load_channel` (linie 709–715).

**Priorytet:** Wysoki
**Rozwiązanie:** Użyć `par_chunks_exact_mut` z rayon na buforze wyjściowym.

---

### 1.4 `ThrottledUpdate` odpytuje timer co 16ms nawet gdy UI jest bezczynne

**Plik:** `src/ui/image_controls.rs`, linie 30–38

`slint::Timer` odpala się co 16ms niezależnie od tego, czy są oczekujące aktualizacje. W długich sesjach bez aktywności użytkownika marnuje czas wątku UI.

**Priorytet:** Niski
**Rozwiązanie:** Timer jednorazowy (one-shot), uzbrajany tylko gdy wartość się zmienia.

---

### 1.5 `apply_gamma_lut_simd` — nie jest faktycznie SIMD

**Plik:** `src/processing/tone_mapping.rs`, linie 221–240

Pomimo nazwy `_simd`, funkcja wypakowuje `f32x4` do tablicy i wywołuje `powf` w jawnej pętli. LLVM może auto-wektoryzować, ale brak gwarancji. Dodatkowe problemy:
- Nadmiarowa zmienna lokalna `gamma_inv_scalar` (linia 231) — tylko kopiuje parametr.
- `srgb_oetf_simd` (linie 195–202) używa `ln()` + `exp()` do aproksymacji `powf(x, 1/2.4)` — wolniejsze i mniej dokładne niż bezpośrednie `powf`.

**Priorytet:** Średni
**Rozwiązanie:** Użyć natywnych instrukcji SIMD lub przynajmniej usunąć redundancje.

---

### 1.6 Wielokrotne przeliczanie stałej w domknięciu per-piksel

**Plik:** `src/io/thumbnails.rs`, linie 240–244

```rust
// Wewnątrz domknięcia na każdy piksel:
let exposure_mult = 2.0_f32.powf(exposure);
```

Wartość jest stała dla całego obrazu. Powinna być obliczona raz przed domknięciem i przechwycona przez kopię.

**Priorytet:** Średni
**Rozwiązanie:** Wynieść obliczenie przed closure.

---

### 1.7 Mutex dla LRU cache miniatur blokuje równoległe generowanie

**Plik:** `src/io/thumbnails.rs`, linie 370–374, 384–436

`THUMB_CACHE` to `Mutex<LruCache<...>>`. `generate_thumbnails_cpu_raw` iteruje pliki z `par_iter()` (linia 108) i wywołuje `c_get()` / `put_thumb_cache()` wewnątrz równoległego iteratora — każdy wątek musi przejąć ten sam mutex, serializując wszystkie operacje na cache.

**Priorytet:** Średni
**Rozwiązanie:** Użyć `DashMap` lub `RwLock` z oddzielnym cache per-shard.

---

### 1.8 `push_console` wykonuje O(n) klon stringa przy każdej linii logu

**Plik:** `src/ui/ui_handlers.rs`, linie 13–21

Każde wywołanie klonuje cały tekst konsoli ze stanu UI, dołącza jedną linię i zapisuje z powrotem. Przy długich sesjach rozmiar stringa rośnie proporcjonalnie.

**Priorytet:** Niski
**Rozwiązanie:** Ring buffer o stałym rozmiarze lub lazy rendering.

---

### 1.9 `load_channel_config` wywoływana wewnątrz pętli po warstwach

**Plik:** `src/ui/layers.rs`, linie 334–349

```rust
for layer in &cache.layers_info {
    let config = load_channel_config().unwrap_or_else(|_| get_fallback_config());
    // ...
}
```

`load_channel_config()` wykonuje I/O na dysku (otwiera i parsuje JSON) wewnątrz pętli po wszystkich warstwach. Przy 50 warstwach to 50 odczytów tego samego pliku.

**Priorytet:** Średni
**Rozwiązanie:** Wywołać `load_channel_config()` raz przed pętlą.

---

## 2. Martwy kod

### 2.1 `load_all_channels_for_layer` — nieużywana ścieżka odczytu

**Plik:** `src/io/image_cache.rs`, linia 464
Oznaczona `#[allow(dead_code)]`. Nie jest wywoływana nigdzie w drzewie wywołań.

---

### 2.2 `LayerExportConfig::BaseOnly` — nieużywany wariant enum

**Plik:** `src/processing/layer_export.rs`, linie 13–17

```rust
#[allow(dead_code)]
pub enum LayerExportConfig {
    BaseOnly,
}
```

Enum nigdy nie jest konstruowany ani matchowany w kodzie produkcyjnym.

---

### 2.3 `UiLayerInfo` i `LazyLayerInfo` — martwe drzewa struktur

**Plik:** `src/io/metadata_traits.rs`, linie 7–66

Oba typy oznaczone `#[allow(dead_code)]`. Implementacje `LayerDescriptor` (linie 88–128) i konwersje do `UnifiedLayerInfo` (linie 266–283) nie są wywoływane w produkcji.

---

### 2.4 `get_color_matrix_cache_stats()` — nigdy nie wywoływana

**Plik:** `src/processing/color_processing.rs`, linie 201–215

```rust
#[allow(dead_code)]
pub fn get_color_matrix_cache_stats() -> (u64, u64, f32)
```

Używana tylko w testach. Żaden kod produkcyjny nie pyta o statystyki cache.

---

### 2.5 `safe_lock` — nigdy nie wywoływana poza modułem

**Plik:** `src/ui/ui_handlers.rs`, linie 33–42

```rust
#[allow(dead_code)]
pub(crate) fn safe_lock<'a, T>(...)
```

Wszędzie używany jest `lock_or_recover`. `safe_lock` nigdy nie jest wywoływana.

---

### 2.6 `reset_progress_on_error` — tylko w testach

**Plik:** `src/utils/error_handling.rs`, linia 113

Makro `handle_ui_error_with_progress!` referuje tę funkcję, ale samo makro nigdy nie jest rozwijane w kodzie produkcyjnym.

---

### 2.7 Zakomentowane importy — brud w kodzie

| Plik | Linia | Import |
|------|-------|--------|
| `src/io/image_cache.rs` | 9 | `// use crate::color_processing::compute_rgb_to_srgb_matrix...` |
| `src/processing/histogram.rs` | 3 | `// use std::sync::Arc;` |

---

### 2.8 `ExrDataSource::Lazy` nigdy nie jest konstruowany

**Plik:** `src/io/image_cache.rs`, linie 70–75

Wariant `Lazy` istnieje, ale żaden kod w `file_handlers.rs` go nie tworzy — ścieżka "light" wciąż buduje `FullExrCacheData` (minimalny). `clear_data_cache()` ma `#[allow(dead_code)]` z tego powodu.

---

### 2.9 Identyczne gałęzie w `format_attribute_value`

**Plik:** `src/io/exr_metadata.rs`, linie 371–386

```rust
AttributeValue::F32(v) => {
    if normalized_key.eq_ignore_ascii_case("pixel_aspect") {
        format!("{:.3}", *v as f64)  // identyczne
    } else {
        format!("{:.3}", *v as f64)  // identyczne
    }
}
```

Obie gałęzie `if` dają ten sam wynik — warunek jest martwy.

---

## 3. Zależności — aktualizacje i zbędne pakiety

| Pakiet | Stan | Problem | Zalecenie |
|--------|------|---------|-----------|
| `once_cell` | Redundantny | Wyparta przez `std::sync::OnceLock` i `std::sync::LazyLock` (stable od Rust 1.80). Kod już używa stdlib w wielu miejscach. | Usunąć z `Cargo.toml`, zastąpić stdlib |
| `instant` | Zbędny | Shim kompatybilności z WASM; EXRuster działa tylko na Windows. Zero użyć w kodzie źródłowym. | Usunąć, użyć `std::time::Instant` |
| `paste` | Zbędny | Zero użyć makra `paste!` w całym kodzie źródłowym. | Usunąć z `Cargo.toml` |
| `tokio` | Nadmiarowy | Użyty z `features = ["full", "test-util"]` — włącza networking, filesystem, sygnały itd. Aplikacja używa `rayon` i `std::thread`. `test-util` w zależnościach produkcyjnych to anomalia. | Usunąć lub drastycznie ograniczyć do `features = ["rt"]` jeśli potrzebny |
| `glam` | 0.30.5 | Aktualna wersja; sprawdzić patch release'y | Monitorować |
| `lru` | 0.16 | Aktualna | OK |
| `dashmap` | 6.1 | Aktualna | OK |
| `windows` | 0.61.3 | Aktualna | OK |
| `memmap2` | 0.9 | Aktualna | OK |

**Oszczędność czasu kompilacji** po usunięciu `once_cell`, `instant`, `paste` i odchudzeniu `tokio`: szacunkowo 10–20% szybsza kompilacja zimna.

---

## 4. Jakość kodu

### 4.1 Zduplikowana logika kompozycji RGB w trzech miejscach

Ta sama logika wyboru indeksów R/G/B z buforów kanałowych jest skopiowana w:
- `src/io/image_cache.rs` linie 558–585 (`compose_composite_from_channels`)
- `src/processing/layer_export.rs` linie 238–269 (`compose_rgb_from_channels`)
- `src/io/thumbnails.rs` (oddzielna, ale konceptualnie tożsama)

Każda poprawka (np. wsparcie kanałów XYZ) musi być wykonana w wielu miejscach.

**Rozwiązanie:** Wynieść do wspólnej funkcji pomocniczej w `src/processing/`.

---

### 4.2 Magic number dla progu rozmiaru pliku

**Plik:** `src/ui/file_handlers.rs`, linia 79

```rust
let use_light = force_light || file_size_bytes > 700 * 1024 * 1024;
```

700 MB zakodowane na stałe bez nazwanej stałej. Kryterium jest arbitralne i nie uwzględnia liczby warstw.

**Rozwiązanie:** `const LIGHT_MODE_FILE_SIZE_THRESHOLD: u64 = 700 * 1024 * 1024;`

---

### 4.3 Globalny mutowalny stan jako most między modułami UI

**Plik:** `src/ui/file_handlers.rs`, linie 21–25

```rust
pub static ITEM_TO_LAYER: LazyLock<Mutex<HashMap<String, String>>> = ...
pub static DISPLAY_TO_REAL_LAYER: LazyLock<Mutex<HashMap<String, String>>> = ...
```

Dwie globalne hashmaps jako most między `create_layers_model` a `handle_layer_tree_click`. Okno między `clear()` a ponownym wypełnieniem jest potencjalnym wyścigiem. Komentarz: "to be moved to state in future refactoring" — do wykonania.

---

### 4.4 Rozbieżność implementacji Hable tonemapping

**Plik 1:** `src/processing/simd_processing.rs`, linie 60–76
**Plik 2:** `src/processing/tone_mapping.rs`, linie 105–119

Dwie implementacje matematycznie równoważne, ale różne (`curr / white_scale` vs `curr * (1/white_scale)`). Duplikacja jest źródłem potencjalnych błędów przy przyszłych modyfikacjach.

---

### 4.5 Duplikat `ConsoleModel` type alias w dwóch modułach

**Plik 1:** `src/ui/layers.rs`, linia 11
**Plik 2:** `src/ui/ui_handlers.rs`, linia 8

```rust
pub type ConsoleModel = std::rc::Rc<VecModel<SharedString>>;
```

Identyczna definicja w dwóch modułach — możliwe niejasne komunikaty błędów typów.

**Rozwiązanie:** Jedną definicję w module root lub `src/ui/mod.rs`.

---

### 4.6 Hardkodowane wymiary 1920×1080 zamiast odczytu z metadanych

**Plik:** `src/io/fast_exr_metadata.rs`, linie 371–378

```rust
width: 1920, // Default values - could be extracted from display_window
height: 1080,
```

Komentarz wprost przyznaje, że wartości są błędne. `fast_meta.display_window` jest już sparsowany i zawiera właściwe wymiary.

**Priorytet:** Wysoki — powoduje błędne metadane w UI.

---

## 5. Zarządzanie pamięcią

### 5.1 `unsafe transmute` w `BufferPool.return_buffer`

**Plik:** `src/utils/buffer_pool.rs`, linie 103–107

```rust
let buffer_f32: Vec<f32> = unsafe { std::mem::transmute(buffer) };
```

`mem::transmute` między `Vec<T>` i `Vec<f32>` technicznie poprawne, gdy T = f32 przez TypeId, ale omija system typów. Jeśli bound generyczny zostanie poluzowany, kod staje się UB.

**Rozwiązanie:** Oddzielna funkcja `return_f32_buffer(Vec<f32>)` bez generyczności i bez `unsafe`.

---

### 5.2 `PooledBuffer.into_inner()` łamie cykl zwrotu do puli

**Plik:** `src/utils/buffer_pool.rs`, linie 20–22

Po wywołaniu `into_inner()`, destruktor nie zwraca bufora do puli (bo `data` = `None`). W `image_cache.rs` (linia 537–542) każde wywołanie puli pobiera bufor i natychmiast wywołuje `into_inner()` — pula faktycznie nigdy niczego nie recyklinguje.

```rust
let mut out = if let Some(pool) = get_buffer_pool() {
    let mut buffer = pool.get_f32_buffer(buffer_size);
    buffer.clear();
    buffer.reserve(buffer_size);
    buffer.into_inner() // ← tutaj pula traci bufor
} else {
    Vec::with_capacity(buffer_size)
};
```

**Priorytet:** Wysoki — system buforowania jest praktycznie niedziałający.
**Rozwiązanie:** Zachować `PooledBuffer` (nie wywoływać `into_inner()`), używać `buffer.as_mut_slice()` lub implementować `DerefMut`.

---

### 5.3 Niepotrzebna kopia danych kanału z `Arc<[f32]>` do `Vec<f32>`

**Plik:** `src/ui/file_handlers.rs`, linie 111–118 (ścieżka light)

```rust
channel_data: lc.channel_data.to_vec(), // Arc<[f32]> → Vec<f32>: pełna kopia
```

`to_vec()` tworzy nową alokację heap i kopiuje wszystkie dane, po czym oryginalne `Arc` jest porzucane. Podwójne zużycie pamięci.

**Rozwiązanie:** `FullLayer` powinien przechowywać `Arc<[f32]>` zamiast `Vec<f32>`.

---

### 5.4 `raw_pixels` realokowany przy każdej zmianie warstwy

**Plik:** `src/io/image_cache.rs`, linie 155–159

```rust
self.raw_pixels = compose_composite_from_channels(&layer_channels);
```

Nowy `Vec<f32>` alokowany i zwalniany przy każdej zmianie warstwy. Dla 4K RGBA: ~135 MB per operacja.

**Rozwiązanie:** `resize()` istniejącego bufora zamiast nowej alokacji.

---

### 5.5 Nieograniczony wzrost tekstu konsoli

**Plik:** `src/ui/ui_handlers.rs`, linie 15–20

`SharedString` konsoli rośnie bez ograniczeń. Po intensywnym użyciu może osiągnąć setki KB, a wszystkie operacje na nim są O(n).

---

## 6. Współbieżność

### 6.1 `RwLock<LruCache>` — semantyka LRU zepsuta przy odczycie

**Plik:** `src/processing/color_processing.rs`, linie 10–11, 172–179

`peek()` pobiera wartość bez aktualizacji kolejności LRU (celowo, żeby nie wymuszać write lock). W efekcie często używane wpisy nie są promowane i są eksmitowane w kolejności FIFO — struktura nazwie się LRU, a zachowuje jak FIFO.

---

### 6.2 `std::thread::spawn` zamiast rayon dla ładowania miniatur

**Plik:** `src/ui/thumbnails.rs`, linia 45

Spawnjuje `std::thread`, który blokuje się czekając na wyniki rayon `par_iter()`. Zewnętrzny wątek jest zbędny — praca mogłaby być przesłana bezpośrednio przez `rayon::spawn`.

---

### 6.3 Rayon thread pool blokowany przez I/O przy ładowaniu pliku

**Plik:** `src/ui/file_handlers.rs`, linie 267–434

Ładowanie pliku EXR (blokujące I/O z dysku) jest realizowane przez wątek rayon — przeznaczone dla CPU-bound work. Wątek I/O blokuje pulę rayon dostępną dla przetwarzania. Obliczenia histogramu (linie 313–334) wykonywane są w `invoke_from_event_loop` — na wątku UI, co może zamrażać interfejs.

**Rozwiązanie:** Użyć `tokio::fs` lub `std::thread::spawn` dla I/O; zachować rayon dla CPU-bound processing.

---

### 6.4 `Mutex<Option<Instant>>` dla throttlingu logu — można zastąpić atomem

**Plik:** `src/ui/image_controls.rs`, linia 9

```rust
static LAST_PREVIEW_LOG: std::sync::Mutex<Option<Instant>> = ...;
```

Mutex na każde wywołanie `update_preview_image`. `std::sync::atomic::AtomicU64` z nanosekund byłby lock-free.

---

### 6.5 `Arc<Mutex<ThrottledUpdate>>` z `!Send` zawartością

**Plik:** `src/ui/setup.rs`, linie 232–243

`ThrottledUpdate` posiada `slint::Timer` (`!Send`). Owinięcie w `Arc<Mutex<...>>` i klonowanie do domknięć jest bezpieczne tylko dlatego, że wszystkie są wywoływane z wątku UI. Jeśli ktokolwiek wywoła `throttled_update.lock()` z wątku tła — UB w Slint event loop. System typów tego nie chroni.

---

## Podsumowanie priorytetów

| # | Problem | Plik | Priorytet |
|---|---------|------|-----------|
| 1 | `PooledBuffer.into_inner()` łamie pooling — system niedziałający | `buffer_pool.rs` | KRYTYCZNY |
| 2 | Hardkodowane 1920×1080 w metadanych | `fast_exr_metadata.rs:374` | WYSOKI |
| 3 | Pełny odczyt EXR przy każdym ładowaniu warstwy | `lazy_exr_loader.rs:176` | WYSOKI |
| 4 | `compose_composite_from_channels` jednowątkowe | `image_cache.rs:599` | WYSOKI |
| 5 | `raw_pixels` realokacja przy każdej zmianie warstwy | `image_cache.rs:156` | WYSOKI |
| 6 | Usunięcie zbędnych zależności (`instant`, `paste`, `once_cell`) | `Cargo.toml` | ŚREDNI |
| 7 | `load_channel_config` w pętli po warstwach | `layers.rs:340` | ŚREDNI |
| 8 | Redundantna kopia Arc→Vec w light mode | `file_handlers.rs:115` | ŚREDNI |
| 9 | `unsafe transmute` w buffer pool | `buffer_pool.rs:105` | ŚREDNI |
| 10 | Rayon thread pool blokowany przez I/O | `file_handlers.rs:270` | ŚREDNI |
| 11 | Zduplikowana logika kompozycji RGB | `image_cache.rs`, `layer_export.rs` | NISKI |
| 12 | `ThrottledUpdate` timer odpalany co 16ms zawsze | `image_controls.rs:30` | NISKI |
| 13 | Martwy kod (`load_all_channels_for_layer`, `UiLayerInfo` itd.) | Wiele plików | NISKI |
| 14 | Nieograniczony wzrost konsoli | `ui_handlers.rs:15` | NISKI |
