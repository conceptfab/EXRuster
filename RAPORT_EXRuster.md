## Raport techniczny: EXRuster — analiza i rekomendacje rozwojowe

### Wprowadzenie
Celem jest podniesienie EXRuster do roli lidera w swojej niszy: szybki, „color‑accurate” viewer/eksporter EXR z profesjonalnym UX dla VFX/CGI. Analiza porównuje aktualny stan `src`, `ui` (Rust+Slint) z projektem referencyjnym `__viewer` (Qt/C++), identyfikując luki i konkretne działania.

### Zakres analizy
- Kod: `src/*`, `ui/*` (EXRuster) oraz `__viewer/*` (C++ referencyjny)
- Obszary: poprawność koloru, dekodowanie EXR (warstwy/kanały/part‑y), rendering/UI (zoom/pan/aspect/okna), eksport, wydajność.

### Kluczowe obserwacje
- **Kolor**: EXRuster nie stosuje `chromaticities` (primaries) → możliwe przekłamania barw; __viewer konwertuje RGB→XYZ→sRGB.
- **Formaty warstw**: Brak wsparcia Y/YC (luma + chroma subsampling) w EXRuster; __viewer ma ścieżki RGB/Y/YC i naprawy saturacji.
- **Wyświetlanie**: EXRuster nie uwzględnia `pixelAspectRatio` ani `dataWindow`/`displayWindow`; __viewer koryguje aspekt i pokazuje overlay okien.
- **Interakcja**: Brak zoom/pan, inspektora pikseli i drag&drop w EXRuster; __viewer ma komplet tych funkcji.
- **Eksport**: Działa (TIFF/PNG16), ale bez gwarancji właściwej kolejności transformacji koloru względem podglądu.
- **Wydajność**: EXRuster używa Rayon i throttlingu; __viewer stosuje QtConcurrent i OpenMP; oba skalują się, ale EXRuster może zyskać na GPU.

### Rekomendacje (zalecenie + komentarz)

- **Kolor (chromaticities)**
  - **Zalecenie**: Zastosować macierz RGB→XYZ (wg atrybutu `chromaticities`) i XYZ→sRGB przed tonemappingiem/gammą.
  - **Dlaczego**: „Color‑accurate” wyświetlanie zgodne z narzędziami DCC/VFX (Nuke/RV/Blender).
  - **Efekt**: Brak przekłamań saturacji/tonu; spójność w pipeline.
  - **Priorytet**: krytyczny; **Trudność**: średnia.

- **Warstwy Y/YC**
  - **Zalecenie**: Wykrywać `Y/RY/BY` i rekonstruować RGB (najpierw prosta interpolacja, potem lepsza).
  - **Dlaczego**: Poprawne wyświetlanie popularnych wariantów EXR (subsampling chromy).
  - **Efekt**: Brak „mydła”/artefaktów, właściwe kolory.
  - **Priorytet**: wysoki; **Trudność**: średnia.

- **Pixel aspect + okna wyświetlania**
  - **Zalecenie**: Korygować szerokość o `pixelAspectRatio`; dodać overlay `dataWindow`/`displayWindow` w UI.
  - **Dlaczego**: Prawidłowe kadrowanie; zgodność z compositingiem.
  - **Efekt**: Brak zniekształceń; lepsze dopasowanie do standardów.
  - **Priorytet**: wysoki; **Trudność**: niska/średnia.

- **Zoom/Pan/Fit/100%**
  - **Zalecenie**: Dodać zoom (scroll), pan (śr. przycisk), skróty 1 i Fit, wskaźnik zoomu.
  - **Dlaczego**: Ergonomia pracy na 4K/8K/16K; kontrola kadru.
  - **Efekt**: Płynna nawigacja; szybsza ocena szczegółów.
  - **Priorytet**: wysoki; **Trudność**: niska.

- **Inspektor pikseli**
  - **Zalecenie**: Prezentować x,y + RGBA (lin i po view‑transform), kopiowanie do schowka.
  - **Dlaczego**: QA/debug; decyzje artystyczne/techniczne.
  - **Efekt**: Precyzyjna kontrola koloru i alf.
  - **Priorytet**: wysoki; **Trudność**: niska.

- **Drag&drop**
  - **Zalecenie**: Otwieranie plików przez upuszczenie.
  - **Dlaczego**: Szybszy workflow.
  - **Efekt**: Mniej klikania; standard w viewerach.
  - **Priorytet**: średni; **Trudność**: niska.

- **Multipart/part‑y**
  - **Zalecenie**: Umożliwić wybór partu i odczyt per‑part atrybutów.
  - **Dlaczego**: Złożone ujęcia VFX wymagają kontroli partów.
  - **Efekt**: Stabilność na zróżnicowanych materiałach.
  - **Priorytet**: średni; **Trudność**: średnia.

- **Eksport (Beauty/Channels)**
  - **Zalecenie**: Wyrównać kolejność transformacji z podglądem: linear → chroma matrix → exposure → tonemap → gamma; dodać batch‑export z szablonem nazw.
  - **Dlaczego**: „Production ready” pliki; powtarzalność.
  - **Efekt**: Eksport = to, co widzisz; automatyzacja.
  - **Priorytet**: wysoki; **Trudność**: niska/średnia.

- **GPU view transform (opcjonalnie)**
  - **Zalecenie**: Przenieść exposure/ACES/gamma/LUT do wgpu (shader); CPU do decode/IO.
  - **Dlaczego**: Płynność i niskie zużycie CPU.
  - **Efekt**: Natychmiastowe podglądy przy dużych obrazach.
  - **Priorytet**: „Pro”; **Trudność**: wysoka.

- **OCIO (opcjonalnie)**
  - **Zalecenie**: Integracja OpenColorIO (View/Display/LUT).
  - **Dlaczego**: Standard branżowy; przewaga konkurencyjna.
  - **Efekt**: Zaufanie w studiach; zgodność z show‑look.
  - **Priorytet**: „Pro”; **Trudność**: wysoka.

### Plan wdrożenia (priorytety)
- **Quick wins (1–2 dni)**: chromaticities (CPU), pixel inspector, zoom/pan + pixel aspect, drag&drop.
- **1–2 tygodnie**: Y/YC rekonstrukcja, overlay data/display, batch‑export, wybór partu.
- **„Pro”**: OCIO i/lub wgpu, tryby porównawcze (A/B, 2‑up/4‑up, diff/heatmap), prefetch/ROI.

### Mapowanie na kod (gdzie dotknąć)
- **`src/image_cache.rs`**: przechowywanie i zastosowanie macierzy chromaticities (3×3) w `process_to_image`/`process_to_composite`/`process_to_thumbnail`; detekcja i rekonstrukcja `Y/RY/BY`; przechowywanie `data_window`/`display_window` i `pixel_aspect_ratio`.
- **`src/exr_metadata.rs`**: odczyt `chromaticities` + helper do wyliczenia macierzy; ekspozycja tych danych do UI.
- **`src/ui_handlers.rs`**: inspektor pikseli (callback z koordynatami → zwrot wartości z `ImageCache`), obsługa zoom/pan (throttling już istnieje), drag&drop → `handle_open_exr_from_path`.
- **`ui/appwindow.slint`**: transformacja obrazu (scale/translate) lub overlay prostokątów okien; Menu View: Fit/100%, przełączniki display/data window.
- **Eksport**: porządek transformacji jak w podglądzie; batch/presety nazw.

### Kryteria akceptacji (DoD)
- Wyświetlanie identyczne (w granicach tolerancji) jak w narzędziu referencyjnym dla tych samych EXR.
- Płynny zoom/pan dla 8K; responsywność przy zmianie ekspozycji/gammy.
- Inspektor zwraca poprawne wartości lin/sRGB; Depth znormalizowany percentylowo.
- Y/YC render bez artefaktów; overlay okien zgodny z nagłówkiem.
- Eksport reprodukuje look widoku; batch działa ze wzorcem nazewnictwa.

### Ryzyka i mitigacje
- **Chromaticities**: brak gotowych utili jak w Imf/Imath → lokalne wyliczenie macierzy; test A/B z referencją.
- **Y/YC**: start od prostej rekonstrukcji; później dopracowanie (pozioma/pionowa, naprawy saturacji).
- **GPU/OCIO**: opcjonalne; zachować pełny fallback CPU.
- **Multipart**: zależne od `exr` crate; w UI umożliwić ręczny wybór warstwy/partu.

### Podsumowanie
Wdrożenie powyższych zmian zamienia EXRuster z „dobrego viewer’a” w narzędzie klasy produkcyjnej: poprawne kolory, płynna interakcja, stabilny eksport i opcjonalne moduły „Pro” (OCIO/GPU). Priorytetem są: chromaticities, zoom/pan + aspect, inspektor pikseli, Y/YC, overlay okien i uporządkowany eksport.

### Lista priorytetów — największa wartość dla użytkownika

1) Krytyczne (natychmiastowy, największy wpływ)

- Zoom/Pan + tryby Fit/100% + korekcja pixelAspectRatio
- Inspektor pikseli (x,y + RGBA lin/sRGB)
- Porządek transformacji w eksporcie = zgodny z podglądem

2) Wysokie
- Y/YC: wykrywanie i rekonstrukcja do RGB
- Overlay `dataWindow`/`displayWindow` (przełączany w View)
- Drag&drop otwierania pliku do okna

3) Średnie
- Multipart/part‑y: wybór partu i per‑part atrybuty
- Batch‑export kanałów/warstw z szablonem nazewnictwa

4) „Pro” (przewaga konkurencyjna)
- OCIO (View/Display/LUT) — zgodność ze standardami studiów VFX
- GPU view transform (wgpu) — płynność na bardzo dużych obrazach
- Tryby porównawcze (A/B, 2‑up/4‑up, diff/heatmap)

Uzasadnienie kolejności: pozycja 1 daje natychmiastową poprawę jakości (color accuracy), ergonomii (nawigacja, inspektor) i zgodności eksportu z podglądem; pozycja 2 domyka poprawność dekodowania i kadr; pozycja 3 zwiększa kompatybilność w złożonych produkcjach; pozycja 4 wzmacnia przewagę i wiarygodność w środowiskach profesjonalnych.

### Instrukcja źródeł i weryfikacji implementacji (dla modelu)

- **Źródła prawdy (kolejność ważności)**
  - **OpenEXR/Imath spec i referencyjna implementacja**: definicje `chromaticities`, `dataWindow`, `displayWindow`, Y/YC (RgbaYca), `pixelAspectRatio`.
  - **Kod referencyjny w `__viewer` (C++) – weryfikowany, niekopiowany**:
    - Chromaticities i macierz: `__viewer/src/model/framebuffer/RGBFramebufferModel.cpp` (blok przeliczenia RGB→XYZ i XYZ→RGB) oraz `__viewer/src/util/ColorTransform.*` (sRGB).
    - Y/YC (rekonstrukcja): `RGBFramebufferModel.cpp` (ścieżka `Layer_YC`, w tym `Imf::RgbaYca` użycie i `fixSaturation`).
    - Pixel aspect i okna: `__viewer/src/view/GraphicsView.*` (korekta szerokości, autoscale, overlay rysunków okien).
    - Multipart i nagłówki: `__viewer/src/model/OpenEXRImage.*` + modele `attribute/*`.
  - **Dokumentacja crate `exr` (Rust)**: API do odczytu warstw/kanałów i atrybutów, odpowiedniki atrybutów OpenEXR.
  - **OCIO (jeśli użyte)**: konfiguracje View/Display, kolejność transformacji, LUT 3D.

- **Zasady korzystania ze źródeł**
  - **Nie kopiować 1:1 kodu C++**. Zrozumieć algorytm, odtworzyć w idiomatycznym Rust (bezpieczeństwo wątków, precyzja float).
  - **Każdy wniosek z `__viewer` potwierdzić w spec OpenEXR/Imath lub dokumentacji `exr`**. Jeśli brak jasności – dodać test A/B i opisać decyzję w komentarzu kodu (krótko, co i dlaczego).
  - **Kolejność transformacji kolorów**: linear → (opcjonalnie) primaries matrix → exposure → tone mapping (ACES) → gamma/sRGB. Zweryfikować, że eksport stosuje identyczny porządek jak viewer.

- **Procedura wdrożenia każdej nowej funkcji**
  - **Discovery**: odczytaj odpowiednie pliki `__viewer` i sekcje specyfikacji. Zanotuj wymagane atrybuty EXR i ich mapowanie na API `exr` (Rust).
  - **Projekt**: zaplanuj API w `ImageCache`/`ui_handlers.rs`/`appwindow.slint` (właściwości, callbacki). Uwzględnij wątki (Rayon) i throttling UI.
  - **Implementacja**: napisz kod w Rust, z jasnym podziałem decode (CPU) vs. view transform (CPU/GPU). Zadbaj o czytelność (nazwy, brak skrótów).
  - **Weryfikacja techniczna**:
    - Test jednostkowy funkcji przekształceń (np. macierz primaries dla znanych chromaticities – sRGB musi być tożsamościowa w granicach tolerancji; DCI‑P3/ACEScg da przewidywalne wyniki).
    - Test A/B obrazka: porównaj wyświetlenie EXR w EXRuster i w `__viewer`/innym referencyjnym narzędziu; dopuszczalne odchyłki ≤ 1 LSB w 8‑bit po końcowej prezentacji.
    - Test integracyjny eksportu: „to co widzisz = to co zapisujesz”.
  - **Weryfikacja użytkowa**:
    - Próbki: EXR z różnymi primaries (sRGB, ACEScg), z `Y/RY/BY`, z niestandardowym `pixelAspectRatio`, z nietrywialnym `dataWindow`/`displayWindow`, z wieloma part‑ami.
    - Płynność UI: ekspozycja/gamma w czasie rzeczywistym dla ≥ 8K.

- **Ścieżki i sygnatury, które należy odczytać przy implementacji**
  - **Chromaticities**: odczyt atrybutu w Rust przez `exr` w `exr_metadata.rs`; zastosowanie macierzy w `ImageCache::process_to_image` i `process_to_composite`.
  - **Y/YC**: detekcja zestawu kanałów `Y/RY/BY` w `load_specific_layer`; rekonstrukcja do RGB (najpierw prosta interpolacja, potem wariant jak w `Imf::RgbaYca`).
  - **Pixel aspect, okna**: atrybuty z nagłówka (Rust `exr`) i prezentacja w UI (`appwindow.slint` overlay lub transform szerokości). Odnieść się do logiki z `GraphicsView` w `__viewer`.
  - **Multipart**: jeżeli `exr` ujawnia part‑y, dodać selektor partu w UI i ładować atrybuty per‑part (analogicznie do `OpenEXRImage`).

- **Testy i artefakty weryfikacyjne**
  - **Golden images**: zestaw EXR + PNG referencyjne (z `__viewer`/narzędzia trzeciego) do porównań pikselowych.
  - **Raport metryk**: log czasu decode/transform (już istnieje w konsoli) + liczby pikseli; zapisać w konsoli JSON‑like w celu późniejszej analizy.
  - **CI**: testy jednostkowe i integracyjne uruchamiane w pipeline; awaria przy odchyłce koloru lub złym porządku transformacji.

- **Zasady jakości**
  - **Czytelność ponad „magiczne” optymalizacje**: nazwy pełne, bez skrótów, brak nadmiernych one‑linerów.
  - **Brak regresji UI**: po dodaniu funkcji zachować istniejące skróty i nawyki pracy.
  - **Fallback**: w razie braku atrybutu/obsługi – zachowaj dotychczasowy behavior i wyraźny komunikat w konsoli.
