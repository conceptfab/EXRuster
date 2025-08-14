# Plan Implementacji Akceleracji GPU (wgpu) w EXRuster

## Wprowadzenie

Celem jest integracja biblioteki `wgpu` w celu akceleracji przetwarzania obrazów EXR na GPU. Główne operacje, takie jak korekcja ekspozycji, gamma i tone mapping, które obecnie są wykonywane na CPU z użyciem `rayon` i SIMD, zostaną przeniesione do compute shaderów w języku WGSL. Zapewni to znaczący wzrost wydajności, zwłaszcza przy pracy z obrazami o wysokiej rozdzielczości.

Poniższy plan dzieli proces implementacji na logiczne etapy, od konfiguracji po integrację z interfejsem użytkownika.

---

## Etap 1: Konfiguracja i Inicjalizacja `wgpu`

W tym etapie przygotujemy fundament pod komunikację z GPU.

1.  **Dodanie zależności do `Cargo.toml`**:
    Należy dodać następujące, najnowsze stabilne wersje bibliotek:
    ```toml
    wgpu = "26.0.1"
    pollster = "0.4.0"
    bytemuck = { version = "1.23.2", features = ["derive"] }
    ```

2.  **Stworzenie modułu `gpu_context.rs`**:
    *   Zdefiniowanie struktury `GpuContext`, która będzie zarządzać stanem `wgpu`: `instance`, `adapter`, `device`, `queue`.
    *   Implementacja funkcji `GpuContext::new()`, która asynchronicznie inicjalizuje `wgpu`. Funkcja ta powinna:
        *   Wybrać odpowiedni adapter (preferując dedykowane GPU o wysokiej wydajności).
        *   Poprosić o utworzenie `device` i `queue`.
        *   Obsłużyć błędy inicjalizacji, np. gdy w systemie nie ma kompatybilnego GPU.

3.  **Integracja `GpuContext` w `main.rs`**:
    *   W funkcji `main` zainicjalizować `GpuContext`.
    *   Przechowywać kontekst w `Arc<Mutex<Option<GpuContext>>>`, aby był dostępny globalnie i bezpieczny wątkowo, podobnie jak `ImageCache`.
    *   W przypadku niepowodzenia inicjalizacji, aplikacja powinna kontynuować działanie w trybie CPU, informując o tym użytkownika w konsoli.


    *   W `main.rs`, po inicjalizacji, pobrać nazwę wybranego adaptera (`adapter.get_info().name`).
    *   Dodać do UI (`appwindow.slint`) nową etykietę w status barze, np. `gpu_status_text`.
    *   W `ui_handlers.rs` stworzyć funkcję aktualizującą tę etykietę, np. "GPU: NVIDIA GeForce RTX 3080" lub "GPU: Not available (CPU fallback)".

---

## Etap 2: Stworzenie Compute Shadera (WGSL)

Sercem akceleracji będzie shader wykonujący obliczenia na GPU.

1.  **Utworzenie pliku `src/shaders/image_processing.wgsl`**:
    *   Plik ten będzie zawierał kod shadera. Można go wczytywać w czasie kompilacji za pomocą `include_str!` w kodzie Rusta.

2.  **Implementacja logiki shadera**:
    *   Shader będzie typu `compute`.
    *   **Definicja Uniformów (Bind Group 0)**:
        *   Struktura `Params` zawierająca: `exposure: f32`, `gamma: f32`, `tonemap_mode: u32`, `width: u32`, `height: u32`.
        *   Opcjonalna macierz `color_matrix: mat3x3<f32>` do transformacji przestrzeni barw.
    *   **Definicja Buforów (Bind Group 1)**:
        *   `input_pixels`: bufor `storage, read_only` przechowujący wejściowe piksele jako `array<vec4<f32>>`.
        *   `output_pixels`: bufor `storage, write_only` na wyjściowe piksele jako `array<vec4<u8>>` (znormalizowane do 0-255).
    *   **Funkcja główna `@compute @workgroup_size(8, 8, 1)`**:
        *   Pobranie ID wątku (`global_invocation_id`).
        *   Przeliczenie ID na indeks piksela.
        *   Wczytanie piksela `vec4<f32>` z `input_pixels`.
        *   Zastosowanie macierzy kolorów (jeśli jest aktywna).
        *   Przeniesienie logiki z `image_processing.rs` (`aces_tonemap`, `reinhard_tonemap`, korekcja gamma) do funkcji w WGSL.
        *   Zastosowanie ekspozycji, tone mappingu i gammy.
        *   Konwersja finalnego koloru `f32` (w zakresie 0-1) na `u8` (0-255).
        *   Zapisanie wyniku jako `vec4<u8>` do `output_pixels`.

---

## Etap 3: Integracja `wgpu` z logiką przetwarzania obrazu

Połączenie `GpuContext` i shadera z istniejącym kodem w `image_cache.rs`.

1.  **Rozszerzenie `ImageCache`**:
    *   Dodanie pola `gpu_context: Arc<Mutex<Option<GpuContext>>>` do struktury `ImageCache`.

2.  **Stworzenie nowej metody `process_to_image_gpu`**:
    *   Metoda ta będzie alternatywą dla `process_to_image`.
    *   **Kroki wewnątrz metody**:
        1.  Pobranie `device` i `queue` z `GpuContext`.
        2.  **Utworzenie buforów**:
            *   `input_buffer`: na podstawie `self.raw_pixels`.
            *   `output_buffer`: o rozmiarze `width * height * 4` bajtów, z flagami `STORAGE | COPY_SRC`.
            *   `staging_buffer`: do odczytu wyników z GPU na CPU, z flagą `MAP_READ | COPY_DST`.
            *   `uniform_buffer`: na parametry (ekspozycja, gamma, etc.).
        3.  **Utworzenie `BindGroupLayout` i `ComputePipeline`** (można je cachować):
            *   Zdefiniowanie layoutu zgodnego z shaderem.
            *   Stworzenie modułu shadera z kodu WGSL.
            *   Skonfigurowanie i utworzenie `ComputePipeline`.
        4.  **Utworzenie `BindGroup`**:
            *   Powiązanie buforów z odpowiednimi bindingami.
        5.  **Wysłanie komend do GPU**:
            *   Utworzenie `CommandEncoder`.
            *   Rozpoczęcie `ComputePass`.
            *   Ustawienie pipeline'u i bind group.
            *   Wywołanie `dispatch_workgroups` z odpowiednią liczbą grup roboczych.
            *   Skopiowanie danych z `output_buffer` do `staging_buffer`.
            *   Zakończenie enkodera i wysłanie komend do `queue.submit()`.
        6.  **Odczyt wyników**:
            *   Zmapowanie `staging_buffer` do odczytu (`staging_buffer.slice(...).map_async`).
            *   Oczekiwanie na zakończenie operacji GPU (`device.poll(wgpu::Maintain::Wait)`).
            *   Pobranie zmapowanych danych.
            *   Stworzenie `SharedPixelBuffer<Rgba8Pixel>` i skopiowanie do niego wyników.
            *   Unmapowanie bufora.
        7.  Zwrócenie `slint::Image`.

---

## Etap 4: Integracja z Interfejsem Użytkownika

Umożliwienie użytkownikowi kontroli nad nową funkcjonalnością.

1.  **Modyfikacja plików `.slint`**:
    *   Dodanie nowego menu "GPU" lub sekcji w istniejącym menu "Options".
    *   Dodanie przełącznika (checkbox) "Enable GPU Acceleration".
    *   Opcjonalnie: lista rozwijana do wyboru adaptera GPU, jeśli dostępnych jest kilka.

2.  **Modyfikacja `ui_handlers.rs`**:
    *   Dodanie callbacku dla nowego przełącznika.
    *   Wprowadzenie globalnego stanu (np. w `AppWindow` lub w `Arc<Mutex<bool>>`), który przechowuje informację o włączeniu akceleracji GPU.
    *   Zmiana w `handle_parameter_changed_throttled` i innych miejscach, gdzie wywoływane jest przetwarzanie obrazu:
        ```rust
        if gpu_acceleration_enabled {
            cache.process_to_image_gpu(...)
        } else {
            cache.process_to_image(...)
        }
        ```
    *   Aktualizacja etykiety statusu (`gpu_status_text`) przy zmianie trybu.

3.  **Obsługa błędów i fallback**:
    *   Jeśli `process_to_image_gpu` zwróci błąd, aplikacja powinna automatycznie przełączyć się na tryb CPU i poinformować użytkownika o problemie (np. "GPU processing failed, falling back to CPU.").

---

## Etap 5: Refinement i Optymalizacje

Dopracowanie implementacji w celu uzyskania maksymalnej wydajności i stabilności.

1.  **Zarządzanie buforami**:
    *   Zamiast tworzyć bufory `wgpu` przy każdym odświeżeniu, przechowywać je w `ImageCache` i zmieniać ich rozmiar tylko wtedy, gdy jest to konieczne (np. po załadowaniu nowego obrazu). Pozwoli to uniknąć kosztownej alokacji pamięci na GPU.

2.  **Asynchroniczność**:
    *   Wykorzystać w pełni asynchroniczną naturę `wgpu`. Zamiast blokować główny wątek za pomocą `pollster`, można użyć `SubmissionIndex` i `on_submitted_work_done` do odczytu wyników bez zacinania UI.

3.  **Akceleracja generowania miniatur**:
    *   Zastosować ten sam potok `wgpu` do akceleracji generowania miniatur w `thumbnails.rs`. Logika przetwarzania jest bardzo podobna, więc można ponownie wykorzystać ten sam shader i większość kodu Rusta.

4.  **Testy i walidacja**:
    *   Dokładnie przetestować nową implementację na różnych platformach (Windows, Linux, macOS) i z różnymi kartami graficznymi (NVIDIA, AMD, Intel).
    *   Porównać wyniki renderowania CPU i GPU, aby upewnić się, że są wizualnie identyczne.
