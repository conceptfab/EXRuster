# Raport optymalizacji EXRuster

## Pliki wymagające poprawek

---

### `src/processing/tone_mapping.rs`

1. **[DONE]** **`srgb_oetf_simd` / `apply_gamma_lut_simd` (linie ~193-239)** — funkcje "SIMD" wypakowują `f32x4` do tablicy `[f32;4]`, wywołują skalarny `powf` 4x, i pakują z powrotem. To najgorętszy hot-path (wywoływany na każdy piksel). Fix: użyć LUT 4096-entry dla gamma lub aproksymację wielomianową `exp2(log2(x)*exp)` — eliminuje `powf` całkowicie.

2. **[DONE]** **`hable_tonemap_simd` / `hable_tonemap` (linie ~103-190)** — `white_scale` jest wyliczany z samych stałych (`a=0.15, b=0.50` itd.) ale obliczany na nowo dla każdych 4 pikseli. Fix: wyliczyć `const WHITE_SCALE_HABLE: f32 = ...` raz i użyć `Simd::splat(WHITE_SCALE_HABLE)`.

---

### `src/processing/simd_processing.rs`

3. **[SKIP]** **`process_rgba_chunk_optimized` (linie ~385-469)** — `SIMD_CHUNK_SIZE=16` przetwarza tylko 4 piksele RGBA na iterację (`f32x4`). Nowoczesne CPU mają AVX2 (8 lane) / AVX-512 (16 lane). Fix: dodać ścieżkę `f32x8` dla exposure/clamp/tonemap i podnieść `SIMD_CHUNK_SIZE` do 32+. *(Wymaga feature-gated nightly SIMD f32x8, zbyt inwazyjne)*

---

### `src/io/lazy_exr_loader.rs`

4. **[DONE]** **`load_layer_from_disk` (linie ~185-265)** — "lazy" loader wywołuje `.all_channels().all_layers()` co dekoduje WSZYSTKIE warstwy z pliku EXR, a potem szuka jednej. Dla pliku z 20 warstwami czyta 20x za dużo danych. Fix: albo użyć selective-layer API crate `exr`, albo pre-loadować cache pełny w tle po wyświetleniu pierwszej warstwy. *(Zrealizowane: LayerNameFilter + read_single_layer_by_name, fallback na all_layers)*

5. **[DONE]** **`to_layer_channels` (linie ~297-306)** — `channel_names: Vec<String>` jest głęboko klonowany (deep copy wszystkich stringów) przy każdym dostępie do warstwy. Fix: zmienić typ na `Arc<Vec<String>>` — klon staje się O(1).

---

### `src/io/image_cache.rs`

6. **[DONE]** **`find_best_layer` (linie ~419-427)** — `to_lowercase()` alokuje nowy `String` dla każdej warstwy × każdą nazwę priorytetową (30 warstw × 8 nazw = 240 alokacji). Fix: pre-lowercase nazwy warstw raz do `Vec<String>` przed pętlą, lub użyć `eq_ignore_ascii_case()`.

7. **[DONE]** **`load_all_channels_for_layer_from_full` (linie ~455-501)** — `layer.name.to_lowercase()` alokuje nowy `String` przy każdym przełączeniu warstwy. Fix: dodać pole `name_lower: String` w `FullLayer` obliczone raz przy budowie cache, lub użyć `eq_ignore_ascii_case()`.

8. **[DONE]** **`channel_alias_to_short` (linie ~16-32)** — `to_ascii_uppercase()` alokuje nowy `String` na każde wywołanie (wywoływane w pętli po kanałach). Fix: użyć `eq_ignore_ascii_case("R")` itd. zamiast uppercase + porównania.

9. **[DONE]** **`compose_composite_into_buffer` (linie ~540-566)** — Rayon z `par_chunks_exact_mut(4)` daje każdemu taskowi tylko 4 floaty — granulacja za drobna. Fix: sekwencyjny SIMD loop dla obrazów < 2MP, Rayon tylko dla większych.

---

### `src/io/full_exr_cache.rs`

10. **[DONE]** **`build_full_exr_cache` (linie ~79-86)** — `channel_data: Vec<f32>` zaczyna pusty i rośnie przez `.extend()` bez `reserve`. Dla 4K z 8 kanałami to ~284 MB z wielokrotnymi realokacjami. Fix: `entry.3.reserve(num_channels * pixel_count)` przed pętlą kanałów.

---

### `src/io/thumbnails.rs`

11. **[DONE]** **`generate_single_exr_thumbnail_work_new` (linie ~212-318)** — dwa odczyty pliku: `extract_layers_info` (headery) + `read_first_rgba_layer_from_file` (piksele). Plus `pixels.into_iter().flat_map().collect::<Vec<u8>>()` materializuje pełny rozmiar obrazu przed resize. Fix: wyeliminować `extract_layers_info` (metadata dostępna z readera), użyć `image::imageops::thumbnail` zamiast pełnej alokacji + resize.

---

### `src/ui/ui_handlers.rs`

12. **[DONE]** **`push_console` (linie ~12-20)** — każde wywołanie czyta cały tekst konsoli z Slint, alokuje nowy `String`, dopisuje linię, ustawia z powrotem — O(N) na każdy log. Po 100 liniach każdy append kopiuje ~8 KB. Dodatkowo `line.clone()` jest zbędny. Fix: trzymać `String` po stronie Rusta (nie czytać z Slint), dopisywać do niego, ustawiać `set_console_text` raz. Dodać limit np. 500 linii (obcinać od góry).

---

### `src/ui/file_handlers.rs`

13. **[DONE]** **Histogram trzykrotnie zduplikowany (linie ~142-165, ~303-325, plus setup.rs ~144-166)** — identyczny blok `update_histogram` + `apply_to_ui` + percentyle skopiowany 3x. Fix: wyekstrahować do `fn apply_histogram_to_ui(ui, app_state)`.

14. **[DONE]** **Channel classification na każdym przebudowaniu drzewa (linie ~465-629)** — `determine_channel_group_with_config` wywoływany na każdy klik expand/collapse. Wewnątrz: `config.basic_rgb_channels.contains(&channel_name.to_string())` alokuje `String` na każde sprawdzenie. Sort używa `iter().position()` w komparatorze — O(n^2). Fix: cache'ować mapę kanał→grupa w `AppState`, użyć `HashMap` dla priorytetów sortu. *(Zrealizowane: eq_ignore_ascii_case zamiast to_string; group_priority_map dla O(1) sortu; rgba_order bez to_uppercase)*

15. **[DONE]** **Wielokrotne blokady write lock w jednym closure (linie ~117-165, ~269-341)** — `app_state.write()` pobierany 3-4x sekwencyjnie w `invoke_from_event_loop`. Fix: skonsolidować w jeden scope write lock. *(Zrealizowane: jeden write lock dla lazy i full cache path)*

---

### `src/ui/layers.rs`

16. **[DONE]** **`toggle_all_layer_groups` (linie ~332-356)** — `load_channel_config()` czyta plik konfiguracyjny z dysku na KAŻDE naciśnięcie strzałki góra/dół. Ten sam plik czytany też w `create_layers_model` zaraz potem — 2x na key press. Fix: cache'ować `ChannelConfig` w `AppState`, ładować raz przy otwarciu pliku.

---

### `src/ui/setup.rs`

17. **[DONE]** **`on_preview_geometry_changed` (linie ~270-310)** — pełny re-render `process_to_image` + verbose log do konsoli przy każdym resize tick (co 100ms z timera). Fix: dodać debounce (np. 200ms `SingleShot` timer) zamiast natychmiastowego renderowania. *(Zrealizowane: DebouncedGeometry 200ms SingleShot)*

18. **[DONE]** **Dead code: `calculate-layout` callback (linie ~68-89)** — oblicza `cached-right-panel-width` i `cached-right-panel-x`, ale te property nigdzie nie są czytane w `.slint`. Fix: usunąć callback, rejestrację i property.

---

### `ui/appwindow.slint`

19. **[DONE]** **Polling timer 100ms always-on (linie ~459-469)** — `Timer { interval: 100ms; running: true; }` pali 10x/s nawet gdy nic się nie zmienia. `self.running = true` w callbacku to dead code. Fix: usunąć timer, powiązać `preview_area_width/height` bezpośrednio z property binding do `parent.width/height` lub użyć `changed` handlera.

20. **[DONE]** **`window_width` / `window_height` (linie ~115-116)** — eksportowane property używane wyłącznie do debug-loga w `setup.rs`. Fix: usunąć property, w callbacku użyć `ui.window().size()` jeśli debug log w ogóle potrzebny.

---

### `ui/components/HistogramWindow.slint`

21. **[DONE]** **4 × N Rectangle elements (linie ~227-268)** — 4 pętle `for` tworzą do 1024 osobnych Rectangle (256 binów × 4 kanały). Elementy z `opacity: 0` wciąż są layoutowane. Fix: owinąć każdą pętlę w `if` sprawdzający `selected-channel`, żeby instancjonować tylko aktywne kanały.

---

### `src/ui/thumbnails.rs`

22. **[DONE]** **Kopiowanie pikseli na UI thread (linie ~126-161)** — `SharedPixelBuffer::new` + pętla kopiowania pikseli (~5MB+ dla 50 miniatur) wykonywana w `invoke_from_event_loop` blokując UI. Fix: przenieść tworzenie `SharedPixelBuffer` i kopiowanie pikseli do wątku tła, na UI thread zostawić tylko `Image::from_rgba8` + `set_thumbnails`. *(Zrealizowane: konwersja Vec<u8>→Rgba8Pixel w tle; na UI tylko alloc + copy_from_slice)*

---

### `src/ui/image_controls.rs`

23. **[DONE]** **`ThrottledUpdate::new()` (linie ~30-38)** — timer `SingleShot` startuje natychmiast przy konstrukcji (16ms), odpala callback z pustymi danymi. Fix: nie startować timera w konstruktorze, startować dopiero w `update_exposure` / `update_gamma`.

---

## Podsumowanie

| Status | Ilość |
|--------|-------|
| DONE | 22 |
| PARTIAL | 0 |
| SKIP | 1 |
| **Razem** | **22/23 zoptymalizowane, 1 pominięty** |
