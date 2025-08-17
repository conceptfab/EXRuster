# Instrukcja Implementacji Optymalizacji `EXRuster`

Poniższa instrukcja opisuje krok po kroku, jak wdrożyć zmiany z `OPTIMIZATION_REPORT.md`. Zmiany zostały pogrupowane w te same etapy co w raporcie.

## Etap 1: Usunięcie Blokowania Wątku UI (Najwyższy Priorytet)

**Cel:** Przeniesienie operacji eksportu do osobnego wątku, aby interfejs użytkownika pozostał responsywny.

**Plik do modyfikacji:** `src/ui_handlers.rs`

### Krok 1: Modyfikacja `handle_export_convert`

1.  Odszukaj funkcję `handle_export_convert`.
2.  Logika do momentu uzyskania ścieżki docelowej `dst` pozostaje bez zmian.
3.  Sklonuj wszystkie zmienne, które będą potrzebne w nowym wątku. Będą to `ui_handle`, `console`, `image_cache`, `full_exr_cache` oraz `dst`.
4.  Całą logikę, która następuje **po** uzyskaniu `dst`, umieść wewnątrz bloku `rayon::spawn`.

**Przykład transformacji:**

```rust
// Kod PRZED zmianą (fragment)
// ...
if let Some(dst) = crate::file_operations::save_file_dialog(...) {
    push_console(&ui, &console, format!("[export] convert → {}", dst.display()));
    let prog = UiProgress::new(ui.as_weak());
    let guard = lock_or_recover(&image_cache);
    // ... cała reszta logiki eksportu ...
}

// Kod PO zmianie (fragment)
// ...
if let Some(dst) = crate::file_operations::save_file_dialog(...) {
    // Sklonuj potrzebne zasoby przed przekazaniem do wątku
    let ui_handle_clone = ui_handle.clone();
    let console_clone = console.clone();
    let image_cache_clone = image_cache.clone();
    let full_exr_cache_clone = full_exr_cache.clone();

    // Uruchom całą logikę eksportu w tle
    rayon::spawn(move || {
        // Wszystkie odwołania do UI muszą być wewnątrz invoke_from_event_loop
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_handle_clone.upgrade() {
                push_console(&ui, &console_clone, format!("[export] convert → {}", dst.display()));
            }
        });

        // Utwórz UiProgress wewnątrz wątku, przekazując mu weak handle do UI
        let prog = UiProgress::new(ui_handle_clone.clone());
        
        // Cała reszta logiki eksportu (tworzenie pliku, pętle po warstwach, zapis)
        // ...
        
        // Pamiętaj, aby każdą aktualizację UI (np. prog.set(...), ui.set_status_text(...))
        // również opakować w `invoke_from_event_loop` lub wywoływać z metod,
        // które już to robią (jak `UiProgress`).
    });
}
```

### Krok 2: Modyfikacja `handle_export_beauty` i `handle_export_channels`

Powtórz ten sam wzorzec dla funkcji `handle_export_beauty` i `handle_export_channels`. Zawsze klonuj potrzebne `Arc` i `Weak`, a następnie umieść logikę przetwarzania i zapisu w `rayon::spawn`.

---

## Etap 2: Przyspieszenie Przetwarzania Obrazów

**Cel:** Zrównoleglenie generowania MIP map dla szybszego ładowania dużych obrazów.

**Plik do modyfikacji:** `src/image_cache.rs`

### Krok 1: Modyfikacja `build_mip_chain`

1.  Odszukaj funkcję `build_mip_chain`.
2.  Upewnij się, że na górze pliku znajduje się `use rayon::prelude::*;`.
3.  Znajdź pętle iterujące po pikselach nowej MIP mapy:
    ```rust
    for y_out in 0..(new_h as usize) {
        // ...
        for x_out in 0..(new_w as usize) {
            // ...
        }
    }
    ```
4.  Zastąp te pętle iteratorem `par_chunks_mut` z Rayon, który będzie przetwarzał piksele równolegle.

**Przykład transformacji:**

```rust
// Kod PRZED zmianą (fragment)
// ...
for y_out in 0..(new_h as usize) {
    let y0 = (y_out * 2).min(height as usize - 1);
    let y1 = (y0 + 1).min(height as usize - 1);
    for x_out in 0..(new_w as usize) {
        // ... logika uśredniania 4 pikseli ...
        next[out_base + c] = acc;
    }
}
// ...

// Kod PO zmianie (fragment)
// ...
use rayon::prelude::*;
// ...
next.par_chunks_mut(new_w as usize * 4) // Przetwarzaj wierszami (szerokość * 4 kanały)
    .enumerate()
    .for_each(|(y_out, row_chunk)| {
        let y0 = (y_out * 2).min(height as usize - 1);
        let y1 = (y0 + 1).min(height as usize - 1);
        for x_out in 0..(new_w as usize) {
            let x0 = (x_out * 2).min(width as usize - 1);
            let x1 = (x0 + 1).min(width as usize - 1);
            let base0 = (y0 * (width as usize) + x0) * 4;
            let base1 = (y0 * (width as usize) + x1) * 4;
            let base2 = (y1 * (width as usize) + x0) * 4;
            let base3 = (y1 * (width as usize) + x1) * 4;
            let out_base = x_out * 4; // Indeks wewnątrz `row_chunk`
            
            for c in 0..4 {
                let acc = (prev[base0 + c] + prev[base1 + c] + prev[base2 + c] + prev[base3 + c]) * 0.25;
                row_chunk[out_base + c] = acc;
            }
        }
    });
// ...
```

---

## Etap 3: Poprawa Płynności Interfejsu Użytkownika

**Cel:** Zmniejszenie przycięć UI podczas operacji na dużej liczbie elementów.

### Krok 1: Wsadowa aktualizacja miniaturek

**Plik do modyfikacji:** `src/ui_handlers.rs`

1.  Odszukaj funkcję `load_thumbnails_for_directory`.
2.  Znajdź blok `slint::invoke_from_event_loop`, w którym tworzona jest lista `items` i ustawiany jest model `ui.set_thumbnails`.
3.  Zamiast tworzyć i ustawiać wszystkie elementy na raz, zaimplementuj mechanizm wsadowy.

**Instrukcja:**

1.  Wewnątrz `invoke_from_event_loop`, po konwersji `sorted_works` na `Vec<ThumbItem>`, wyczyść istniejący model w UI: `ui.get_thumbnails().as_any().downcast_ref::<VecModel<ThumbItem>>().unwrap().set_vec(vec![]);`.
2.  Przenieś `Vec<ThumbItem>` do `Arc<Mutex<...>>`, aby można było bezpiecznie się do niego odwoływać z timera.
3.  Utwórz `slint::Timer`, który będzie uruchamiał się cyklicznie (np. co 16ms).
4.  Wewnątrz pętli timera, pobieraj małą paczkę (np. 20) elementów z `Vec<ThumbItem>` i dodawaj je do modelu UI.
5.  Gdy wektor z elementami będzie pusty, zatrzymaj timer.

### Krok 2: Optymalizacja konsoli

**Plik do modyfikacji:** `src/ui_handlers.rs` oraz plik `.slint`

1.  W funkcji `push_console` usuń cały blok odpowiedzialny za ręczne składanie stringa i aktualizację `ui.set_console_text`. Pozostaw jedynie linię: `console.push(line.clone().into());`.
2.  W pliku `.slint` (prawdopodobnie `ui/appwindow.slint`) znajdź definicję konsoli. Obecnie jest to zapewne `TextEdit`. Zastąp go komponentem `ListView`.
3.  W `AppWindow` w pliku `.slint` dodaj nową właściwość `in-out property <[string]> console_model;`.
4.  W `main.rs`, przekaż `console_model` do UI: `ui.set_console_model(ModelRc::new(console_model.clone()));`.
5.  W `ListView` w pliku `.slint` ustaw `model: root.console_model;` i zdefiniuj wygląd każdego wiersza.

---

## Etap 4: Odblokowanie Akceleracji GPU

**Cel:** Wykorzystanie istniejącego shadera do przetwarzania obrazu na GPU.

**Plik do modyfikacji:** `src/image_cache.rs` (głównie), `src/gpu_context.rs` (ew. funkcje pomocnicze).

### Krok 1: Dodanie ścieżki GPU

1.  W `src/image_cache.rs`, w funkcjach `process_to_image` i `process_to_thumbnail`, dodaj warunek sprawdzający, czy akceleracja GPU jest włączona i dostępna.
2.  Do tego celu będziesz potrzebować dostępu do globalnego kontekstu GPU i flagi włączającej (można je pobrać z `ui_handlers`).
3.  Jeśli warunek jest spełniony, wywołaj nową, dedykowaną funkcję, np. `process_image_gpu(...)`. W przeciwnym razie, wykonaj istniejący kod CPU (Rayon+SIMD).

### Krok 2: Implementacja `process_image_gpu`

Ta nowa funkcja będzie sercem operacji na GPU.

1.  **Definicja struktury parametrów**: W Rust stwórz strukturę `Params` odpowiadającą tej z shadera WGSL. Użyj `#[repr(C)]` i dodaj `use bytemuck::{Pod, Zeroable};`, aby łatwo konwertować ją na bajty.
2.  **Tworzenie buforów**: Użyj `device.create_buffer_init` do stworzenia buforów na GPU:
    *   `input_buffer`: na dane wejściowe z `self.raw_pixels`.
    *   `params_buffer`: na dane ze struktury `Params`.
    *   `output_buffer`: pusty bufor na wyniki, z flagami `STORAGE | COPY_SRC`.
    *   `staging_buffer`: bufor do odczytu wyników z powrotem na CPU, z flagami `MAP_READ | COPY_DST`.
3.  **Tworzenie potoku (pipeline)**:
    *   Załaduj kod shadera `image_processing.wgsl`.
    *   Stwórz `wgpu::ShaderModule`, `BindGroupLayout`, `PipelineLayout` i na końcu `wgpu::ComputePipeline`.
4.  **Tworzenie `BindGroup`**: Stwórz `BindGroup`, która połączy utworzone bufory z odpowiednimi bindingami (`@binding(...)`) zdefiniowanymi w shaderze.
5.  **Wykonywanie shadera**:
    *   Stwórz `wgpu::CommandEncoder`.
    *   Rozpocznij `compute_pass`.
    *   Ustaw potok i `BindGroup`.
    *   Wywołaj `dispatch_workgroups`, obliczając liczbę grup roboczych na podstawie wymiarów obrazu i rozmiaru grupy z shadera (8x8).
    *   Zakończ `compute_pass`.
    *   Dodaj komendę kopiowania danych z `output_buffer` do `staging_buffer`.
    *   Wyślij komendy do wykonania na GPU (`queue.submit`).
6.  **Odczyt wyników**:
    *   Asynchronicznie zmapuj `staging_buffer` do pamięci CPU (`staging_buffer.slice(...).map_async(...)`).
    *   Poczekaj na zakończenie operacji przez GPU (`device.poll(wgpu::Maintain::Wait)`).
    *   Pobierz zmapowane dane, przekonwertuj je na `Vec<u8>`, a następnie na `slint::Image`.
    *   Pamiętaj o zwolnieniu mapowania (`unmap`).

Implementacja tej części jest najbardziej złożona, ale przyniesie największe korzyści wydajnościowe na kompatybilnym sprzęcie.
