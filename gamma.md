Aktualny proces:
   1. Ekspozycja: Stosowana jest standardowa formuła 2.0 ^ EV.
   2. Tonemapping i Gamma: Obraz HDR jest najpierw mapowany do zakresu widzialnego przy
      użyciu algorytmu ACES, a następnie aplikowana jest korekcja gamma (precyzyjna krzywa
      sRGB dla wartości ~2.2 lub funkcja potęgowa dla innych).

Zgadzam się z sugestią i rozszerzę funkcjonalność, dodając nowe tryby tonemappingu, co da
   użytkownikowi większą kontrolę nad finalnym wyglądem obrazu.

Planowane zmiany:
   1. Wprowadzę nowe tryby do wyboru:
       * ACES (domyślny): Obecny, wysokiej jakości tryb.
       * Reinhard: Popularna alternatywa o nieco innym kontraście.
       * Linear: Brak tonemappingu, użyteczny do inspekcji technicznej.
   2. Dodam odpowiedni przełącznik w interfejsie użytkownika.
   3. Zmodyfikuję kod w image_processing.rs, image_cache.rs oraz ui_handlers.rs, aby obsłużyć
       nową funkcjonalność.

---

### Szczegółowy Plan Implementacji

#### Etap 1: Optymalizacja wydajności korekcji Gamma (implementacja LUT)

*   **Cel:** Zastąpienie kosztownej obliczeniowo operacji `powf()` szybszym mechanizmem opartym o tablicę przeglądową (LUT).
*   **Pliki:** `src/image_processing.rs`
*   **Kroki:**
    1.  Zaimplementować `thread_local` cache dla tablicy LUT, aby uniknąć konfliktów przy przetwarzaniu równoległym (Rayon).
    2.  Stworzyć nową funkcję `apply_gamma_lut`, która:
        *   Generuje 1D LUT (1024 elementy) przy pierwszej zmianie wartości `gamma` w danym wątku.
        *   W kolejnych wywołaniach odczytuje wartości z LUT, stosując interpolację liniową dla zachowania precyzji.
    3.  W funkcji `process_pixel` zamienić wywołanie `apply_gamma_fast` na nową funkcję `apply_gamma_lut`.

#### Etap 2: Wprowadzenie trybów Tonemappingu - Logika Aplikacji

*   **Cel:** Rozszerzenie silnika przetwarzania obrazu o nowe algorytmy tonemappingu.
*   **Pliki:** `src/image_processing.rs`, `src/image_cache.rs`, `src/thumbnails.rs`, `src/ui_handlers.rs`
*   **Kroki:**
    1.  **`image_processing.rs`**:
        *   Dodać funkcję `reinhard_tonemap`.
        *   Zmodyfikować `process_pixel`, aby przyjmowała nowy parametr `tonemap_mode: i32` i na jego podstawie wybierała algorytm (ACES, Reinhard, Linear/Clamp).
    2.  **`image_cache.rs`**:
        *   Zaktualizować sygnatury funkcji `process_to_image`, `process_to_composite`, `process_to_thumbnail`, dodając parametr `tonemap_mode`.
        *   Przekazać ten parametr do `process_pixel`.
    3.  **`thumbnails.rs`**:
        *   Zaktualizować sygnatury `generate_exr_thumbnails_in_dir` i `generate_single_exr_thumbnail_work`, dodając `tonemap_mode`.
        *   Przekazać parametr w dół aż do `process_pixel`.
    4.  **`ui_handlers.rs` (Eksport)**:
        *   Zmodyfikować `handle_export_beauty`, aby podczas zapisu do PNG uwzględniała wybrany tryb tonemappingu.

#### Etap 3: Integracja z Interfejsem Użytkownika

*   **Cel:** Umożliwienie użytkownikowi wyboru trybu tonemappingu z poziomu UI.
*   **Pliki:** `ui/appwindow.slint`, `src/ui_handlers.rs`, `src/main.rs`
*   **Kroki:**
    1.  **`ui/appwindow.slint`**:
        *   Dodać nową właściwość `property <int> tonemap_mode: 0;`.
        *   Dodać nowy `callback tonemap_mode_changed(int);`.
        *   Wstawić komponenty `Checkbox` obok suwaków ekspozycji/gammy, z opcjami "ACES", "Reinhard", "Linear". Powiązać go z nową właściwością i callbackiem.
    2.  **`main.rs` / `ui_handlers.rs`**:
        *   Dodać nową funkcję obsługi zdarzenia `on_tonemap_mode_changed`.
        *   Wewnątrz tej funkcji, odświeżyć obraz, wywołując `process_to_image` (lub `process_to_thumbnail`) z nowym trybem.
    3.  **`ui_handlers.rs`**:
        *   We wszystkich miejscach, gdzie wywoływane są funkcje z `ImageCache` (np. `handle_open_exr_from_path`, `handle_layer_tree_click`, `load_thumbnails_for_directory`), pobrać aktualną wartość `tonemap_mode` z UI i przekazać ją jako parametr.
