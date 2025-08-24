Przeanalizuj kod Rust pod kątem błędów spójności parametrów renderowania obrazu. Skup się na:

  ## 1. HARDCODED WARTOŚCI vs UI DEFAULTS
  Szukaj miejsc gdzie:
  - Hardcoded wartości liczbowe (0, 1, 2) dla tone mapping modes
  - Domyślne parametry exposure, gamma, tonemap_mode różnią się od UI
  - Fallback wartości w match/if statements

  **Przykład błędu:**
  ```rust
  // UI: tonemap-mode: 2 (Linear)
  // Kod: tonemap_mode = 0  // BŁĄD: ACES zamiast Linear

  2. NIESPÓJNOŚĆ MIĘDZY PODGLĄDEM A EKSPORTEM

  Sprawdź czy:
  - UI używa tych samych parametrów co eksport
  - Thumbnail generation używa tych samych parametrów
  - Processing pipeline ma spójne domyślne wartości
  - Checkbox states poprawnie przekazują parametry

  Szukaj wzorców:
  - if !apply_corrections { WARTOŚĆ }
  - ToneMapMode::ACES jako default
  - tonemap_mode: 0 (ACES) vs tonemap_mode: 2 (Linear)
  - Różne wartości gamma/exposure w różnych miejscach

  3. ENUM DEFAULTS I FALLBACKS

  Sprawdź:
  - impl Default for ... czy używa poprawnych wartości
  - impl From<i32> fallback case (_ =>)
  - Match arms z placeholder wartościami
  - Test assertions oczekujące złych wartości

  Wzorce błędów:
  _ => Self::ACES,        // Powinno być Linear
  ToneMapMode::Local => use_aces_fallback()  // Niespójne

  4. KOMENTARZE VS IMPLEMENTACJA

  Szukaj komentarzy mówiących jedno, a kod robiący coś innego:
  // Comment: "should match preview (Linear)"
  // Code: uses ACES  // BŁĄD!

  5. CONFIG FILES I UI DEFINITIONS

  W .slint/.json sprawdź:
  - Default values property bindings
  - Reset button behaviors
  - Initial state values

  Raportuj każdy znaleziony błąd z:
  - Lokalizacja (plik:linia)
  - Obecna wartość vs oczekiwana
  - Wpływ na użytkownika
  - Sugerowana poprawka

  Priorytet błędów:
  1. 🔴 KRYTYCZNY: Różnice między podglądem a eksportem
  2. 🟡 ŚREDNI: Niespójne domyślne wartości
  3. 🟢 NISKI: Nieaktualne komentarze/testy

  Ten prompt systematycznie znajdzie wszystkie podobne błędy w kodzie renderowania obrazu.

0. optymalizacja WCZYTYWANIA
1. Logowanie wyczyścic
1. Preferencje / GPU / progi plików
1. About
1. reportGPU - aktualizacja/weryfikacja
