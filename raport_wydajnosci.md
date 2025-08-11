# Raport Wydajności - Propozycje Optymalizacji

Poniższa lista przedstawia rekomendowane zmiany w kodzie aplikacji `EXRuster` w celu znaczącego przyśpieszenia jej działania, zwłaszcza podczas wczytywania obrazów, generowania miniatur i interaktywnej zmiany parametrów. Zmiany koncentrują się na kluczowych obszarach wydajnościowych, unikając jednocześnie nadmiernej komplikacji kodu.

### 1. Użycie biblioteki `glam` do operacji wektorowych i macierzowych (SIMD)

*   **Problem:** Ręcznie implementowana matematyka macierzowa w `color_processing.rs` (np. `mul3x3`) nie wykorzystuje sprzętowego przyśpieszenia (instrukcji SIMD), które są standardem w wydajnych aplikacjach graficznych. Każde mnożenie macierzy jest wykonywane sekwencyjnie.
*   **Rozwiązanie:** Należy zintegrować lekką bibliotekę `glam`, która jest standardem w ekosystemie Rust do wydajnej matematyki wektorowej. Zastąpienie typów `[[f32; 3]; 3]` i ręcznych funkcji mnożenia przez `glam::Mat3` i jej operatory pozwoli na automatyczne wykorzystanie instrukcji SIMD, co znacząco przyśpieszy obliczenia transformacji kolorów wykonywane dla każdego piksela.
*   **Pliki do modyfikacji:** `Cargo.toml`, `color_processing.rs`, `image_cache.rs`.

### 2. Optymalizacja korekcji gamma przez Lookup Table (LUT)

*   **Problem:** Funkcja `value.powf(gamma_inv)` w `image_processing.rs` jest wywoływana dla każdego piksela podczas zmiany parametrów i eksportu. Jest to operacja kosztowna obliczeniowo.
*   **Rozwiązanie:** Należy zaimplementować mechanizm tablicy przeglądowej (Lookup Table, LUT). Zamiast obliczać `powf` za każdym razem, należy stworzyć jednowymiarową tablicę (np. o rozmiarze 1024) przy każdej zmianie wartości gamma. Następnie w funkcji `apply_gamma_fast` wartość piksela (z zakresu 0-1) byłaby mapowana na indeks w tej tablicy, a wynik odczytywany i ewentualnie interpolowany liniowo. Jest to technika wielokrotnie szybsza.
*   **Pliki do modyfikacji:** `image_processing.rs`.

### 3. Zmniejszenie operacji I/O podczas ładowania warstwy

*   **Problem:** Funkcja `load_all_channels_for_layer` w `image_cache.rs` wczytuje *wszystkie* kanały z danej warstwy, nawet jeśli do początkowego podglądu kompozytu (RGB) potrzebne są tylko 3-4 kanały. W plikach z wieloma kanałami (np. Cryptomatte, AOV) powoduje to niepotrzebne, powolne operacje I/O.
*   **Rozwiązanie:** Należy stworzyć nową, wyspecjalizowaną funkcję, która wczytuje tylko jawnie wskazane kanały (np. "R", "G", "B", "A") na potrzeby domyślnego widoku. Pełne wczytanie wszystkich kanałów z warstwy powinno następować "leniwie" - dopiero wtedy, gdy użytkownik jawnie zażąda podglądu konkretnego, jeszcze niezaładowanego kanału.
*   **Pliki do modyfikacji:** `image_cache.rs`.

### 4. Optymalizacja generowania miniatur (ścieżka awaryjna)

*   **Problem:** Główna metoda generowania miniatur w `thumbnails.rs` jest wydajna, ale jej ścieżka awaryjna (fallback) wczytuje całą warstwę w pełnej rozdzielczości do pamięci (`load_specific_layer`), co jest bardzo nieefektywne dla dużych plików.
*   **Rozwiązanie:** Należy zmodyfikować ścieżkę awaryjną, aby zamiast wczytywać cały obraz, wczytywała tylko niezbędne dane. Można to osiągnąć, czytając co N-tą linię skanowania (strided read) z pliku EXR, co drastycznie zmniejszy zużycie pamięci i przyśpieszy operację dla dużych obrazów.
*   **Pliki do modyfikacji:** `thumbnails.rs`, `image_cache.rs`.

### 5. Refaktoryzacja i centralizacja logiki przetwarzania pikseli

*   **Problem:** W `image_cache.rs` logika przetwarzania pikseli (mnożenie przez macierz kolorów, aplikacja ekspozycji i gammy) jest częściowo powielona w kilku miejscach: `process_to_image`, `process_to_composite`, `process_to_thumbnail`. Utrudnia to wprowadzanie i weryfikację optymalizacji.
*   **Rozwiązanie:** Należy stworzyć jedną, zunifikowaną, zrównolegloną funkcję (np. `process_raw_pixels`), która przyjmuje surowe piksele i wszystkie parametry (ekspozycja, gamma, macierz kolorów, tryb - kolor/grayscale). Funkcja ta zwracałaby gotowy do wyświetlenia bufor pikseli. To uprości kod i zapewni, że optymalizacje (jak LUT dla gammy czy SIMD dla macierzy) będą konsekwentnie zastosowane w każdym trybie podglądu.
*   **Pliki do modyfikacji:** `image_cache.rs`, `image_processing.rs`.
