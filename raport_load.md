### Raport: Przyspieszenie wczytywania plików EXR (Rust)

#### Cel

Maksymalnie skrócić czas „time-to-first-pixel” (TTFP) i czas generowania miniaturek dla plików EXR, przy zachowaniu jakości obrazu i funkcjonalności UI. Nowe rozwiązania mają być zrealizowane w Rust z użyciem istniejącego crate `exr` oraz narzędzi kompatybilnych z Windows.

---

### Porównanie podejść: `__viewer/` (C++) vs `src/` (Rust)

- **C++ / OpenEXR (szybkie):**

  - Bezpośrednie użycie `Imf::MultiPartInputFile` i `Imf::FrameBuffer` do selektywnego wczytania tylko potrzebnych kanałów.
  - Jedno przejście po danych: `part.setFrameBuffer(framebuffer)` + `part.readPixels(...)` ładuje cały zakres skanlinii do prealokowanych buforów.
  - Konwersje macierzą chromaticities i sRGB wykonywane wektorowo (OpenMP) w pamięci ciągłej `float[4*N]`.

- **Rust / crate `exr` (obecnie wolniejsze):**
  - Dwa ciężkie przejścia I/O: najpierw pełne `read_all_data_from_file` dla metadanych/warstw, potem `read_all_flat_layers_from_file` dla pikseli.
  - Pobieranie wartości pikseli przez `value_by_flat_index(i).to_f32()` w pętli per‑piksel i per‑kanał (duży narzut wywołań i skoków pamięci).
  - Bufor pikseli w formie `Vec<(f32,f32,f32,f32)>` utrudnia wektorowe przetwarzanie (gorsza lokalność pamięci niż SoA/interleaved).
  - Miniatury: dla każdego pliku pełny odczyt „najlepszej” warstwy w pełnej rozdzielczości.

---

### Wąskie gardła (Rust)

- Podwójny odczyt z dysku (nagłówki+warstwy → piksele) zwiększa TTFP.
- Per‑piksel `value_by_flat_index` i konwersje do `f32` powodują duży narzut CPU i cache misses.
- Układ pamięci `Vec<(f32,f32,f32,f32)>` ogranicza przepustowość podczas przetwarzania (brak ciągłości kanałów).
- Generowanie miniaturek z pełnej rozdzielczości dla każdego pliku w katalogu.

---

### Rekomendacje (Rust-first)

1. Seleksyjne wczytanie kanałów przez builder `exr::read()` (zamiast `read_all_flat_layers_from_file`)

   - Użyj API „reader builder” do wyboru konkretnej warstwy (po nazwie/heurystyce) i tylko kanałów R/G/B/A.
   - Wczytuj w jednym przebiegu skanlinie do prealokowanych buforów docelowych (SoA lub interleaved), bez per‑piksel `value_by_flat_index`.
   - Oczekiwany zysk: 2–5× szybsze ładowanie dużych obrazów (4K–16K), mniejsze zużycie CPU.

2. Metadane „header‑only” dla listy warstw/kanałów

   - Zamiast `read_all_data_from_file` użyć odczytu tylko nagłówków/atrybutów bez danych pikseli.
   - Utrzymać obecną logikę grupowania do UI, ale bez parsowania danych pikseli na tym etapie.

3. Mmap dla plików (Windows) — `memmap2`

   - Zmapować plik do pamięci i udostępnić jako `Read + Seek` dla crate `exr`.
   - Redukcja kopiowań i kosztów I/O w kernel/user, szczególnie dla bardzo dużych EXR.

4. Zmiana układu bufora pikseli

   - Z `Vec<(f32,f32,f32,f32)>` na:
     - interleaved `Vec<f32>` o długości `4 * N` (RGBA interleaved), lub
     - SoA: cztery `Vec<f32>` (R, G, B, A).
   - Lepsza lokalność i przepustowość pamięci w `process_to_*` i przy konwersjach kolorów.

5. Miniatury z minimalnym kosztem

   - Jeżeli obraz ma mip‑mapy/tiling, czytać najniższy dostępny poziom (lub sub‑sample co k‑tą linię/kolumnę podczas odczytu).
   - Odczytywać wyłącznie R/G/B (i ewentualnie A) oraz wykonywać down‑sampling w trakcie wypełniania bufora.
   - Oczekiwany zysk: 3–10× szybsze miniatury w katalogach z wieloma EXR.

6. Opcjonalnie: przechowywanie surowych pikseli w `f16` (half)

   - Przechowywać piksele jako half (np. `half` crate), konwertować do `f32` tylko na krawędzi renderingu do RGBA8.
   - Zmniejsza zużycie pamięci i obciążenie cache, przy zachowaniu jakości dla HDR.

7. Dalsze mikro‑optymalizacje
   - Wektorowe/macierze kolorów stosować przed tone‑mappingiem (jedno przejście po danych).
   - Utrzymać throttling podglądu (już zaimplementowany) i ścieżkę „thumbnail preview” dla dużych obrazów.

---

### Plan wdrożenia (etapy)

1. `image_cache::load_specific_layer` — selektywne czytanie kanałów

   - Zastąpić `read_all_flat_layers_from_file` builderem `read()` z wyborem warstwy i kanałów.
   - Prealokować bufor interleaved/SoA i wypełniać go bezpośrednio w callbackach czytnika.
   - Bez zmiany publicznego API `ImageCache`.

2. `extract_layers_info` i metadane „header‑only`

   - Odczyt tylko nagłówków i kanałów bez danych pikseli.
   - Zachować istniejące formatowanie metadanych do UI.

3. `thumbnails` — szybka ścieżka miniaturek

   - Czytanie najniższego poziomu lub sub‑sample podczas odczytu.
   - Tylko R/G/B (+A opcjonalnie), bez pełnych konwersji na wczesnym etapie.

4. Bufor pikseli i przetwarzanie

   - Migracja na interleaved `Vec<f32>` lub SoA i adaptacja `process_to_image`/`process_to_composite`.
   - Ewentualna wersja z `f16` za flagą feature.

5. Mmap (opcjonalny feature na Windows)
   - Dodać `memmap2` i wariant loadera korzystający z mapowania pamięci.

---

### Kryteria akceptacji i pomiar

- Zmniejszenie TTFP o ≥2× dla plików 4K+ (mierzone logami czasu w `ui_handlers::handle_open_exr_from_path`).
- Miniatury dla katalogu 50 EXR 4K w ≤1/3 obecnego czasu (log w `[folder] ... thumbnails ...`).
- Brak regresji jakości podglądu (wizualna zgodność z obecną ścieżką i C++ viewer’em dla referencyjnych plików).

---

### Ryzyka i kompatybilność

- API crate `exr` (builder `read()`) różni się semantyką od wysokopoziomowych helperów; konieczne uważne mapowanie warstw/kanałów do buforów.
- Pliki z nietypowym układem (tiled/multi‑part, brak klasycznych R/G/B) wymagają fallbacku (pierwsza RGBA lub heurystyki jak dotychczas).
- `memmap2` może być ograniczone uprawnieniami AV/EDR — feature powinien być opcjonalny.

---

### Dalsze uwagi implementacyjne (Rust)

- Zachować obecną heurystykę wyboru „najlepszej” warstwy; selekcję zastosować już na etapie konfiguracji czytnika.
- Przetwarzanie kolorów: zastosować macierz primaries→sRGB przed tone‑mappingiem i gamma (jedno przejście, dobre dla cache).
- Równoległość: pozostawić `rayon` w konwersjach; odczyt linii z `exr` wykonywać sekwencyjnie (zgodnie z API), ale wypełnianie buforów można realizować blokowo.

---

### Następne kroki

1. Zaimplementować etap 1 (selektywny odczyt kanałów) w `image_cache` i dodać pomiar czasu do logów.
2. Zmienić metadane na ścieżkę „header‑only”.
3. Usprawnić `thumbnails` (najniższy level/sub‑sample) i porównać czasy na zestawie testowym.
4. Rozważyć migrację bufora pikseli i opcjonalny `f16`.

Proszę o akceptację wdrożenia etap 1–3. Po akceptacji wprowadzimy zmiany w `src/` zgodnie z wytycznymi projektu.
