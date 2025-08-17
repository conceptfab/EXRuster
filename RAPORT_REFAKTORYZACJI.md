### Raport Poprawek Kodu - EXRuster

Poniżej znajduje się lista zaleceń dotyczących refaktoryzacji i poprawy kodu projektu. Celem jest zwiększenie czytelności, redukcja duplikacji oraz uproszczenie logiki.

#### I. Uproszczenie i Refaktoryzacja Głównej Logiki (`main.rs` i `ui_handlers.rs`)

1.  **Wydzielenie logiki aktualizacji podglądu:** Stworzyć nową, prywatną funkcję w `ui_handlers.rs`, np. `update_preview_image(&AppWindow, &ImageCacheType, &ConsoleModel)`, która będzie zawierała całą logikę renderowania podglądu. Ta funkcja będzie wywoływana z `handle_parameter_changed_throttled`, `on_tonemap_mode_changed` i `on_preview_geometry_changed`, eliminując duplikację kodu.
2.  **Refaktoryzacja `handle_layer_tree_click`:** Uprościć logikę opartą na parsowaniu stringów. Zamiast tego przekazywać z UI (Slint) bardziej strukturalne dane, np. indeks warstwy i indeks kanału, lub przynajmniej czyste nazwy bez dekoracji (np. "📁" czy "•"). To uczyni kod bardziej niezawodnym.
3.  **Podział dużych funkcji w `ui_handlers.rs`:**
    *   `handle_open_exr_from_path`: Podzielić na mniejsze funkcje: `load_metadata`, `load_image_data`, `update_ui_after_load`.
    *   `handle_export_convert` i `handle_export_channels`: Przenieść logikę zapisu plików (TIFF, PNG) do nowego modułu, np. `src/exporters.rs`. `ui_handlers.rs` powinien tylko wywoływać funkcje z tego modułu.
4.  **Uproszczenie callbacków w `main.rs`:** Zredukować boilerplate klonowania `Arc` w funkcjach `setup_*_callbacks` poprzez grupowanie powiązanych callbacków lub użycie makra, jeśli to możliwe.

#### II. Oczyszczenie Modułów Przetwarzania Obrazu (`image_cache.rs`, `thumbnails.rs`)

5.  **Usunięcie nieużywanego kodu w `image_cache.rs`:** Usunąć nieużywane funkcje `load_specific_layer` i `load_first_rgba_layer`, które są oflagowane `#[allow(dead_code)]`.
6.  **Konsolidacja generowania miniaturek w `thumbnails.rs`:** Pozostawić tylko jedną, główną implementację generowania miniaturek (prawdopodobnie `generate_thumbnails_cpu_raw` jako backend dla `generate_thumbnails_cpu`). Usunąć starsze i nieużywane funkcje (`generate_single_exr_thumbnail_work`).
7.  **Ujednolicenie logiki GPU:**
    *   Usunąć niekompletną implementację GPU z `image_cache.rs` (`process_to_image_gpu` i powiązane pola).
    *   W `thumbnails.rs` usunąć nieaktywną ścieżkę `generate_thumbnails_gpu`.
    *   Docelowo, cała logika GPU powinna być w jednym miejscu (np. w `gpu_processing.rs`), a nie rozproszona i częściowo wyłączona. Na razie, dla uproszczenia, można całkowicie usunąć kod związany z GPU, jeśli nie jest on w pełni funkcjonalny.

#### III. Poprawki Ogólne i Porządkowe

8.  **Usunięcie duplikatów w `utils.rs` i `ui_handlers.rs`:** Usunąć funkcję `normalize_channel_display_to_short` z `ui_handlers.rs` i wszędzie używać `normalize_channel_name` z `utils.rs`.
9.  **Przeniesienie kodu platformowego:** Przenieść funkcję `try_set_runtime_window_icon` z `main.rs` do nowego pliku `src/platform_win.rs` i wywoływać ją z `main.rs` pod `#[cfg(target_os = "windows")]`.
10. **Weryfikacja `dead_code`:** Przejrzeć cały projekt pod kątem ostrzeżeń `#[allow(dead_code)]` i usunąć nieużywane funkcje i struktury, aby oczyścić kod. Dotyczy to zwłaszcza `gpu_context.rs` i `gpu_thumbnails.rs`, gdzie wiele funkcji pomocniczych może nie być używanych.
