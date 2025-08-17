# Plan Poprawek: III. Poprawki Ogólne i Porządkowe

Na podstawie `RAPORT_REFAKTORYZACJI.md`, poniżej znajduje się szczegółowy plan krok po kroku dla zadań z sekcji III.

---

### 1. Usunięcie duplikatów funkcji do normalizacji nazw kanałów

**Cel:** Używanie jednej, spójnej funkcji `normalize_channel_name` z modułu `utils`.

**Kroki:**

1.  **Zlokalizuj i usuń duplikat:**
    *   Otwórz plik `src/ui_handlers.rs`.
    *   Znajdź i usuń całą definicję funkcji `normalize_channel_display_to_short`.

2.  **Zastąp wywołania starych funkcji:**
    *   W pliku `src/ui_handlers.rs` znajdź wszystkie miejsca, gdzie używana była funkcja `normalize_channel_display_to_short`.
    *   Zamień każde wywołanie na `utils::normalize_channel_name`. Upewnij się, że `use crate::utils;` jest obecne na górze pliku.
    *   Sprawdź, czy logika przekazywania argumentów i obsługi wartości zwracanej jest nadal poprawna.

---

### 2. Przeniesienie kodu specyficznego dla Windows

**Cel:** Wydzielenie logiki specyficznej dla platformy do osobnego modułu, aby oczyścić `main.rs`.

**Kroki:**

1.  **Utwórz nowy plik modułu:**
    *   Stwórz nowy plik w lokalizacji: `src/platform_win.rs`.

2.  **Przenieś funkcję:**
    *   Otwórz `src/main.rs`.
    *   Wytnij (Cut) całą definicję funkcji `try_set_runtime_window_icon`.
    *   Wklej (Paste) skopiowany kod do nowego pliku `src/platform_win.rs`.
    *   W `src/platform_win.rs`, zmień sygnaturę funkcji na `pub fn try_set_runtime_window_icon(...)`, aby była widoczna dla innych modułów.

3.  **Zaktualizuj `main.rs`:**
    *   Na górze pliku `src/main.rs` dodaj deklarację nowego modułu, opakowaną w atrybut `cfg`:
        ```rust
        #[cfg(target_os = "windows")]
        mod platform_win;
        ```
    *   Znajdź miejsce, gdzie `try_set_runtime_window_icon` była wywoływana.
    *   Popraw wywołanie, aby wskazywało na nowy moduł:
        ```rust
        #[cfg(target_os = "windows")]
        platform_win::try_set_runtime_window_icon(&main_window);
        ```

---

### 3. Weryfikacja i usunięcie nieużywanego kodu (`dead_code`)

**Cel:** Oczyszczenie bazy kodu z funkcji i struktur, które nie są już używane.

**Kroki:**

1.  **Globalne wyszukiwanie:**
    *   Przeszukaj cały projekt (foldery `src`, `tool`) pod kątem frazy `#[allow(dead_code)]`.

2.  **Analiza i usunięcie (ze szczególnym uwzględnieniem modułów GPU):**
    *   **Plik `src/gpu_context.rs`:**
        *   Sprawdź, które z funkcji pomocniczych (np. do tworzenia buforów, pipeline'ów) są faktycznie wywoływane.
        *   Usuń te, które nie mają żadnych odwołań w kodzie.
    *   **Plik `src/gpu_thumbnails.rs`:**
        *   Zweryfikuj, czy jakakolwiek funkcja z tego modułu jest używana po wyłączeniu ścieżki `generate_thumbnails_gpu`. Jeśli nie, cały moduł może być kandydatem do usunięcia lub oznaczenia jako `#[deprecated]`.
    *   **Inne pliki:**
        *   Przejrzyj pozostałe znalezione wystąpienia `#[allow(dead_code)]`.
        *   Dla każdego z nich zadaj pytanie: "Czy ten kod jest tymczasowo wyłączony, czy jest to pozostałość po starej logice?".
        *   Jeśli jest to pozostałość, usuń kod. Jeśli jest tymczasowo wyłączony, dodaj komentarz `// TODO: Re-enable when feature X is implemented` dla jasności.

3.  **Kompilacja:**
    *   Po usunięciu kodu, uruchom `cargo check` lub `cargo build`, aby upewnić się, że nie powstały żadne błędy kompilacji.
