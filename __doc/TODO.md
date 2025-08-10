1. Wyswietlanie pełnej nazwy po najechaniu na nazwę
2. filtr wyświetlania w panelu


Chcesz, żebym dodał opcjonalną kompresję TIFF (LZW/Deflate) lub wybór bit‑głębi (f32/16‑bit) zgodnie z export.md?

Jeśli wolisz od razu pełne „warstwy Photoshopa” (34377) lub eksport PSD, napisz którą opcję wybierasz.

Krótko: jak zaimplementuję eksport PSD
Format docelowy: PSD (Big-Endian), RGB kolor, wiele warstw.
Warstwy:
EXR RGBA → warstwa PSD z kanałami R, G, B, A.
EXR RGB → warstwa PSD z R, G, B (A pełne 255).
EXR 1-kanałowe (Z/Maski) → warstwa „grayscale” jako RGB z równymi kanałami R=G=B (A pełne 255).
Nazwy warstw z EXR do nazwy warstwy PSD.
Głębia i kompresja:
Domyślnie 16-bit na kanał (najszersza kompatybilność).
Kompresja: Raw (0) – najprostsza i stabilna; opcjonalnie PackBits (RLE, 1) później.
Przekształcenie danych:
Wariant 1 (bezpieczny dla edycji): zapis 16-bit LINEAR (bez gamma), tylko klip do [0,1] i skalowanie do 0..65535.
Wariant 2 (pod edycję w sRGB): exposure/gamma oraz ewentualna macierz RGB→sRGB jak w viewer, potem 16-bit.
Struktura PSD, którą zapiszę:
Header: „8BPS”, wersja 1, tryb kolorów RGB.
Color Mode Data: puste.
Image Resources: minimalne (opcjonalnie później dodamy metadane).
Layer and Mask Info:
N warstw z rekordami kanałów i prostokątami całego obrazu.
Blend mode „norm”, opacity 255, widoczność on, nazwy warstw.
Dane kanałów: kompresja 0 (Raw), linia po linii.
Composite Image (flattened): zapiszę z „Beauty” (lub pierwszej RGB) jako obraz scalony (wymagane przez część programów).
Implementacja:
Nowy moduł src/psd_writer.rs korzystający z byteorder (Big-Endian).
API: write_psd(path, layers: Vec<PsdLayer>, composite: Composite).
Wykorzystam istniejące wczytywanie danych z EXR (image_cache) do zbudowania buforów warstw.
Dopytuję, żeby wprowadzić właściwe ustawienia:
Bitowa głębia: 16-bit ok?
Kompresja: Raw (0) czy RLE (PackBits, 1)?
Transformacja: zapisywać liniowo (bez gamma) czy zastosować exposure/gamma + macierz do sRGB jak w viewer?
Po Twoim wyborze zaimplementuję i podłączę eksport PSD w ui_handlers.rs (nowe menu „Export PSD”) i zbuduję projekt.