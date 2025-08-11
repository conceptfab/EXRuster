# Raport z analizy kodu i planowane poprawki

Na podstawie analizy kodu w folderach `src/` i `ui/` zidentyfikowano następujące obszary do poprawy, z naciskiem na unikanie over-engineeringu i poprawę wizualizacji procesów.

### Pliki wymagające modyfikacji:
- `src/image_cache.rs`
- `src/thumbnails.rs`
- `src/exr_metadata.rs`
- `src/ui_handlers.rs`
- `src/main.rs`
- `src/exr_layers.rs` (do usunięcia)
- `src/psd_writer.rs` (do usunięcia)

---

### Planowane zadania:

1.  **Usunięcie duplikacji kodu zarządzania kolorem.**
    - **Problem:** Logika do obliczania macierzy transformacji kolorów (funkcja `compute_rgb_to_srgb_matrix_from_file_for_layer` i jej funkcje pomocnicze) jest skopiowana w dwóch plikach: `src/image_cache.rs` i `src/thumbnails.rs`.
    - **Rozwiązanie:** Stworzyć nowy moduł, np. `src/color_processing.rs`, przenieść do niego całą zduplikowaną logikę i importować ją w obu miejscach. Zapewni to spójność i ułatwi przyszłe modyfikacje.

2.  **Usunięcie nieużywanych i niezaimplementowanych plików.**
    - **Problem:** Pliki `src/exr_layers.rs` i `src/psd_writer.rs` zawierają jedynie placeholdery dla niezrealizowanej funkcji zapisu do formatu PSD.
    - **Rozwiązanie:** Usunąć oba pliki z projektu, aby uprościć bazę kodu i usunąć martwy kod.

3.  **Refaktoryzacja i poprawa wizualizacji postępu przy generowaniu miniatur.**
    - **Problem:** Generowanie miniatur w `src/thumbnails.rs` raportuje postęp tylko na początku i na końcu. Przy dużej liczbie plików użytkownik nie wie, na jakim etapie jest proces.
    - **Rozwiązanie:** Zmodyfikować funkcję `generate_exr_thumbnails_in_dir`, aby raportowała postęp po przetworzeniu każdego pliku (lub co kilka plików). Można to osiągnąć, używając `AtomicUsize` do zliczania ukończonych zadań w pętli równoległej i aktualizując pasek postępu (`progress.set(completed / total, ...)`).

4.  **Refaktoryzacja wczytywania metadanych i poprawa odporności na błędy.**
    - **Problem:** W `src/exr_metadata.rs`, funkcje `pretty_*` ręcznie parsowały atrybuty ze stringów, co jest podatne na błędy.
    - **Rozwiązanie:** Zmodyfikować funkcję `read_and_group_metadata`, aby korzystała z typowanych atrybutów dostarczanych przez bibliotekę `exr` (np. `Attribute::Chromaticities`, `Attribute::V2f`) zamiast parsowania ich reprezentacji tekstowej. Zwiększy to niezawodność i uprości kod.

5.  **Usprawnienie wizualizacji postępu podczas eksportu wielowarstwowego.**
    - **Problem:** Funkcje eksportu w `src/ui_handlers.rs` (np. `handle_export_channels`, `handle_export_convert`) iterują po wielu warstwach/kanałach, ale pasek postępu jest widoczny jako nieokreślony przez większość czasu.
    - **Rozwiązanie:** Zmodyfikować te funkcje, aby raportowały postęp w trakcie iteracji. Należy zliczyć całkowitą liczbę operacji (warstw/kanałów do przetworzenia) i aktualizować pasek postępu po każdej z nich.

6.  **Ujednolicenie logiki wczytywania miniatur.**
    - **Problem:** Logika wczytywania miniatur przy starcie aplikacji z argumentem (w `src/main.rs`) jest bardzo podobna do tej przy wyborze folderu roboczego (w `src/ui_handlers.rs`).
    - **Rozwiązanie:** Wydzielić tę logikę do jednej, wspólnej funkcji (np. w `ui_handlers.rs`), która przyjmuje ścieżkę do katalogu i uchwyt do UI, a następnie wywoływać ją w obu miejscach.
