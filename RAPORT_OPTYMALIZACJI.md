# Raport optymalizacji kodu `EXRuster`

Poniższy raport przedstawia zidentyfikowane obszary do optymalizacji w kodzie projektu. Celem jest maksymalne przyśpieszenie działania aplikacji, zwłaszcza w zakresie przetwarzania obrazów i interakcji z GPU, przy jednoczesnym unikaniu nadmiernej inżynierii.

---

### Etap 1: Krytyczna optymalizacja potoku GPU

Największy potencjał do optymalizacji leży w sposobie zarządzania zasobami GPU. Obecne podejście, polegające na tworzeniu buforów, pipeline'ów i bind group przy każdym przetwarzaniu obrazu, jest skrajnie nieefektywne.

1.  **`image_cache.rs` (`process_to_image_gpu_internal`)**:
    *   **Zadanie:** Zrefaktoryzować funkcję tak, aby zasoby GPU (pipeline, bind group layout, bufory uniformów) były tworzone tylko raz i przechowywane w strukturze `ImageCache`. Przy każdym wywołaniu należy jedynie aktualizować zawartość buforów (`input_pixels`, `uniforms`) i ponownie wykorzystywać istniejący pipeline.
    *   **Cel:** Drastyczne zmniejszenie narzutu na komunikację z GPU, co przyśpieszy aktualizację podglądu przy zmianie parametrów (ekspozycja, gamma).

2.  **`gpu_thumbnails.rs` (`process_thumbnail`)**:
    *   **Zadanie:** Zastosować tę samą strategię co powyżej. Struktura `GpuThumbnailProcessor` powinna przechowywać gotowy pipeline i layout. Funkcja `process_thumbnail` powinna jedynie tworzyć specyficzne dla zadania bufory (wejściowy, wyjściowy) i bind group, a następnie dispatchować zadanie na istniejącym pipeline.
    *   **Cel:** Przyśpieszenie generowania miniatur na GPU, gdy ta funkcja zostanie w pełni zintegrowana.

3.  **`shaders/image_processing.wgsl`**:
    *   **Zadanie:** Uprościć funkcje `srgb_oetf` i `apply_gamma`, zastępując warunki `if` funkcją `select` w celu clampowania wartości do przedziału `[0, 1]`. Jest to bardziej idiomatyczne dla kodu shaderów i może być wydajniejsze.
    *   **Cel:** Poprawa czytelności i potencjalnie wydajności shadera.

---

### Etap 2: Optymalizacja przetwarzania danych po stronie CPU

Operacje na CPU, zwłaszcza te związane z przygotowaniem danych i algorytmami, również mogą zostać zoptymalizowane.

1.  **`image_cache.rs` (`build_mip_chain`)**:
    *   **Zadanie:** Obecna implementacja generowania MIP-map jest jednowątkowa. Należy ją zrównoleglić przy użyciu `rayon`, przetwarzając wiersze lub bloki pikseli równolegle.
    *   **Cel:** Znaczne przyśpieszenie generowania podglądów o niższej rozdzielczości, co poprawi responsywność UI przy manipulacji dużymi obrazami.

2.  **`image_cache.rs` (Struktura `ImageCache`)**:
    *   **Zadanie:** Zmienić typ pola `raw_pixels` z `Vec<(f32, f32, f32, f32)>` (tablica struktur) na `Vec<f32>` (interleaved, `[R,G,B,A,R,G,B,A,...]`).
    *   **Cel:** Poprawa lokalności danych w pamięci cache procesora. Upraszcza to i przyśpiesza transfer danych do buforów GPU oraz operacje SIMD, które mogą wczytywać ciągłe bloki pamięci.

3.  **`full_exr_cache.rs` (`build_full_exr_cache`)**:
    *   **Zadanie:** Pętla kopiująca piksele (`for i in 0..pixel_count`) jest nieefektywna. Należy zbadać, czy biblioteka `exr` oferuje dostęp do danych pikseli jako ciągłego slice'a, co pozwoliłoby na użycie szybszej operacji hurtowej, np. `copy_from_slice`.
    *   **Cel:** Przyśpieszenie wczytywania całego pliku EXR do pamięci.

---

### Etap 3: Refaktoryzacja i czyszczenie kodu

Uproszczenie kodu i usunięcie duplikacji poprawi jego utrzymywalność i zmniejszy ryzyko błędów.

1.  **`exr_metadata.rs` (`read_and_group_metadata`)**:
    *   **Zadanie:** Wyodrębnić zduplikowany kod formatujący wartości atrybutów (`AttributeValue::Chromaticities`, `F32`, `F64` itd.) do jednej, prywatnej funkcji pomocniczej.
    *   **Cel:** Zmniejszenie redundancji i poprawa czytelności.

2.  **`image_cache.rs` (`find_best_layer`, `load_specific_layer`)**:
    *   **Zadanie:** Usunąć liczne, pozostawione w kodzie instrukcje `println!` służące do debugowania.
    *   **Cel:** Oczyszczenie kodu produkcyjnego i uniknięcie zaśmiecania konsoli.

3.  **`ui_handlers.rs` (`handle_export_convert`, `handle_export_channels`)**:
    *   **Zadanie:** Logika znajdowania indeksów kanałów (R, G, B, A) w warstwie jest powtórzona w kilku miejscach. Należy stworzyć jedną, wspólną funkcję pomocniczą w `full_exr_cache.rs` lub `utils.rs`, która przyjmuje `&FullLayer` i zwraca zmapowane indeksy.
    *   **Cel:** Uniknięcie duplikacji kodu i centralizacja logiki mapowania kanałów.

4.  **`main.rs`**:
    *   **Zadanie:** Uprościć inicjalizację GPU. Zamiast ręcznego tworzenia wątku i używania `pollster`, można wykorzystać prostsze `block_on` z `futures` lub `tokio` (jeśli zostanie dodane jako zależność) bezpośrednio w funkcji `main` przed uruchomieniem pętli UI.
    *   **Cel:** Poprawa czytelności i uproszczenie logiki startowej aplikacji.
