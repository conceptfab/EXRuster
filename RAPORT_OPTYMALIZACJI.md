# Raport Optymalizacji Kodu - EXRuster

Poniższy raport przedstawia zidentyfikowane obszary do optymalizacji w kodzie projektu. Celem jest maksymalne przyśpieszenie działania aplikacji przy jednoczesnym unikaniu nadmiernej inżynierii (over-engineering). Zmiany zostały podzielone na logiczne etapy.

## Etap 1: Optymalizacja operacji plikowych i cache'owania (I/O)

Celem tego etapu jest zminimalizowanie liczby operacji odczytu z dysku oraz usprawnienie sposobu, w jaki dane są przechowywane w pamięci po wczytaniu.

1.  **Wydajniejsze budowanie cache'u EXR (`full_exr_cache.rs`)**
    *   **Problem:** Funkcja `build_full_exr_cache` kopiuje dane pikseli z wczytanego pliku EXR do bufora `channel_data` pojedynczo, piksel po pikselu (`entry.3.push(...)`). Jest to nieefektywne dla dużych obrazów.
    *   **Zadanie:** Zmodyfikuj pętlę w `build_full_exr_cache` tak, aby kopiowała dane całego kanału za jednym razem, używając metody `extend_from_slice` lub podobnej, zamiast indywidualnego `push`. To znacząco zredukuje liczbę operacji i przyśpieszy budowanie pełnego cache'u pliku.

2.  **Cache'owanie macierzy transformacji kolorów (`image_cache.rs`)**
    *   **Problem:** Macierz konwersji kolorów (`color_matrix_rgb_to_srgb`) jest obliczana przy tworzeniu `ImageCache`, a następnie ponownie przy każdej zmianie warstwy (`load_layer`). Jest to zbędne, jeśli atrybuty `chromaticities` nie zmieniają się między warstwami.
    *   **Zadanie:** Zmodyfikuj `ImageCache`, aby przechowywać macierz transformacji dla każdej warstwy w `HashMap<String, Mat3>`. Obliczaj macierz tylko raz dla danej warstwy i odczytuj ją z mapy przy kolejnych przełączeniach.

## Etap 2: Optymalizacja przetwarzania obrazu na CPU

Ten etap koncentruje się na usprawnieniu algorytmów działających na CPU, głównie z wykorzystaniem Rayon i SIMD.

1.  **Równoległe tworzenie kompozytu RGBA (`image_cache.rs`)**
    *   **Problem:** Funkcja `compose_composite_from_channels`, która tworzy główny bufor `raw_pixels` (RGBA) z oddzielnych płaszczyzn kanałów (R, G, B, A), działa jednowątkowo, iterując po każdym pikselu.
    *   **Zadanie:** Zrównoleglij tę funkcję przy użyciu `rayon`. Przetwarzaj fragmenty bufora wyjściowego równolegle, co znacząco przyśpieszy tworzenie podglądu po wczytaniu nowej warstwy.

2.  **Przyspieszenie generowania miniaturek (`thumbnails.rs`)**
    *   **Problem:** Funkcja `generate_single_exr_thumbnail_work_new` używa filtra `Lanczos3` do skalowania obrazów. Jest to filtr wysokiej jakości, ale relatywnie wolny, co nie jest konieczne dla małych miniaturek.
    *   **Zadanie:** W funkcji `generate_single_exr_thumbnail_work_new` zmień filtr w `image::imageops::resize` z `FilterType::Lanczos3` na szybszy, np. `FilterType::Triangle`. Różnica w jakości będzie niezauważalna na miniaturkach, a zysk wydajności znaczący.

## Etap 3: Optymalizacja przetwarzania na GPU

Usprawnienia w potoku renderowania z użyciem `wgpu` w celu lepszej wydajności i jakości.

1.  **Poprawa jakości skalowania w shaderze miniaturek (`gpu_thumbnails.rs`)**
    *   **Problem:** Shader w `gpu_thumbnails.rs` używa interpolacji bilinearnej do skalowania obrazu w dół. Przy dużym zmniejszeniu może to prowadzić do aliasingu i utraty detali.
    *   **Zadanie:** Zmodyfikuj shader `THUMBNAIL_COMPUTE_SHADER`. Zamiast próbkować jeden punkt źródłowy, uśrednij wartości z bloku 2x2 lub 4x4 pikseli źródłowych (tzw. box filter). Zapewni to gładszy i bardziej reprezentatywny wygląd miniaturek.

2.  **Uproszczenie i stabilizacja potoku GPU (`image_cache.rs`)**
    *   **Problem:** Logika przetwarzania GPU w `process_to_image_gpu_internal` jest skomplikowana, podatna na błędy (panics) i nieefektywnie zarządza zasobami, próbując je odtwarzać w locie.
    *   **Zadanie:** Zrefaktoryzuj tę część. Stwórz dedykowaną, trwałą strukturę (np. `GpuProcessor`), która będzie przechowywać potok `wgpu` i bufory. Inicjalizuj ją raz i zmieniaj rozmiar buforów tylko w razie potrzeby. To uprości kod, wyeliminuje błędy i zwiększy wydajność przez unikanie rekreacji zasobów.

3.  **Dodanie transformacji kolorów w shaderze (`shaders/image_processing.wgsl`)**
    *   **Problem:** Główny shader do przetwarzania obrazu nie uwzględnia macierzy transformacji kolorów (`color_matrix`), przez co kolory na podglądzie GPU mogą różnić się od tych z CPU.
    *   **Zadanie:** Dodaj do shadera `image_processing.wgsl` nowy `uniform` typu `mat3x3<f32>` dla macierzy kolorów. Zastosuj tę transformację do wartości RGB piksela przed etapem ekspozycji i tone mappingu, aby zapewnić spójność kolorystyczną z resztą aplikacji.

## Etap 4: Ogólne usprawnienia i refaktoryzacja

Drobne zmiany poprawiające czytelność, redukujące duplikację kodu i wykorzystujące lepsze praktyki.

1.  **Refaktoryzacja formatowania atrybutów (`exr_metadata.rs`)**
    *   **Problem:** Logika formatowania wartości atrybutów (`AttributeValue`) na `String` jest zduplikowana w dwóch miejscach: raz dla atrybutów współdzielonych i raz dla atrybutów per-warstwa.
    *   **Zadanie:** Wydziel tę logikę do jednej, prywatnej funkcji pomocniczej, np. `format_attribute_value(value: &AttributeValue) -> String`, i używaj jej w obu pętlach, aby uniknąć powielania kodu.

2.  **Uproszczenie konwersji typów w `glam` (`color_processing.rs`)**
    *   **Problem:** Funkcje `bradford_adaptation_matrix` i `rgb_to_xyz_from_primaries` konwertują typy `DMat3` (f64) na `Mat3` (f32) poprzez ręczne tworzenie nowej macierzy i kopiowanie każdej składowej.
    *   **Zadanie:** Zastąp ręczną konwersję wbudowanymi metodami z biblioteki `glam`, takimi jak `as_mat3()` i `as_vec3()`. Kod stanie się czystszy, krótszy i potencjalnie wydajniejszy.
