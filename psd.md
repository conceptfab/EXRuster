## EXR → PSD: plan prostego narzędzia CLI (drag & drop)

### Cel

- Samodzielny EXE bez UI: przeciągasz `*.exr` na `exr2psd.exe`, otrzymujesz `*.psd` z warstwami.
- Minimalne zależności, prosta implementacja, nacisk na zgodność z Photoshop/GIMP/Affinity.

### Architektura (bez osobnego projektu)

- Dodać drugi binarny target w tym repo: `src/bin/exr2psd.rs` (nie workspace).
- Modularnie: `src/psd_writer.rs` (niski poziom PSD), `src/exr_layers.rs` (odczyt EXR → bufory warstw).
- Budowanie: `cargo build --release --bin exr2psd`.

### Interfejs użytkownika (CLI/drag&drop)

- Wywołanie:
  - `exr2psd.exe input.exr` → zapisze `input.psd` obok pliku źródłowego.
  - `exr2psd.exe input.exr output.psd` → zapis w podanej ścieżce.
- Drag&drop (Windows): ścieżki trafiają do `std::env::args()` (działa bez dodatkowego kodu).

### Mapowanie EXR → warstwy PSD

- Dla każdej warstwy EXR:
  - RGBA → warstwa PSD z kanałami R,G,B,Alpha.
  - RGB → warstwa PSD z R,G,B (Alpha = pełne 255).
  - 1‑kanał (np. Z/Maska) → warstwa RGB z R=G=B=val, Alpha = 255, nazwa z EXR.
- Nazwy warstw: z atrybutów EXR (warstwa pusta → „Beauty”).
- Założenie 1. iteracji: wszystkie warstwy mają identyczny rozmiar; inne pomijamy z ostrzeżeniem (log).

### Głębia, kolory, kompresja

- Głębia: 16 bit na kanał (najszersza kompatybilność edytorów).
- Kompresja kanałów: Raw (0) – najprostsza i stabilna.
- Transformacja kolorów (v1 – prosta):
  - Linear clamp do [0,1] → skala do 0..65535 → zapis u16.
  - Bez gamma/sRGB na starcie (opcjonalne w v2: przełącznik `--srgb`).

### Struktura pliku PSD (wymagana minimalna)

- Header:
  - Signature: `8BPS`, Version: 1, Channels: 3/4 (dla composite), Height/Width (u32), Depth: 16, ColorMode: RGB.
- Color Mode Data: puste.
- Image Resources: puste (opcjonalnie dodać ICC później).
- Layer and Mask Information:
  - Lista warstw (top‑down w PSD): każda warstwa ma rekord z:
    - Rect = pełny obraz, Blend mode: `norm`, Opacity: 255, Visible: on, Nazwa warstwy (pascal string + padding).
    - Kanały: -1 (Alpha, jeśli występuje), 0 (R), 1 (G), 2 (B); kompresja 0 (Raw), dane linia po linii.
  - Global layer mask: pusta.
- Image Data (Composite):
  - Flattened obraz (np. z warstwy „Beauty” albo pierwszej RGB), 16‑bit, kompresja 0 (Raw).

### Moduł `psd_writer.rs` (API i odpowiedzialności)

- Zależność: `byteorder` (Big‑Endian) do zapisu strumieniowego.
- API proponowane:
  - `pub struct PsdLayer { pub name: String, pub width: u32, pub height: u32, pub r: Vec<u16>, pub g: Vec<u16>, pub b: Vec<u16>, pub a: Option<Vec<u16>> }`
  - `pub fn write_psd<W: Write>(out: W, layers: &[PsdLayer], composite: &PsdLayer) -> anyhow::Result<()>`
- Implementacja:
  - Funkcje pomocnicze: zapis headera, sekcji Image Resources, Layer & Mask (rekordy, kanały, długości), Image Data.
  - Kanały zapisywane w kolejności PSD, każda linia bez kompresji (Raw), big‑endian u16.

### Odczyt EXR → warstwy (`exr_layers.rs`)

- Wykorzystanie `exr` crate (jak w aplikacji):
  - Iteracja po „flat layers”, przypisanie kanałów do R/G/B/A; 1‑kanał → duplikacja do RGB.
  - Konwersja f32 → u16: `val_u16 = (val.clamp(0.0, 1.0) * 65535.0).round() as u16`.
  - Zwrócenie `Vec<PsdLayer>` i wybranie „composite” (Beauty/pierwsza RGB).

### `src/bin/exr2psd.rs` (orchestracja)

- Parsowanie args: 1 lub 2 argumenty.
- Odczyt EXR → `Vec<PsdLayer>` + `composite`.
- Zapis PSD przez `psd_writer::write_psd` do pliku wyjściowego.
- Log błędów i ostrzeżeń w `stderr`.

## Struktura drugiego EXE (exr2psd)

### Drzewo plików

```
.
├─ src/
│  ├─ bin/
│  │  └─ exr2psd.rs        # punkt wejścia CLI bez UI
│  ├─ psd_writer.rs        # niskopoziomowy zapis PSD (BE, warstwy, composite)
│  └─ exr_layers.rs        # odczyt EXR → wewnętrzne bufory warstw (u16)
├─ psd.md                  # specyfikacja (ten dokument)
└─ dist/                   # artefakty (exr2psd.exe) – tworzone przez build.py
```

Uwaga: dla `src/bin/exr2psd.rs` nie trzeba zmieniać `Cargo.toml` – Cargo automatycznie wykryje drugi bin po ścieżce.

### Interfejs CLI (kontrakt)

- Wywołanie: `exr2psd <input.exr> [output.psd]`
- Kody wyjścia:
  - 0: sukces
  - 2: brak/wiele niepoprawnych argumentów
  - 3: błąd odczytu EXR / brak warstw
  - 4: błąd zapisu PSD
- Zasady:
  - Gdy `output.psd` nie podano → twórz obok wejścia jako `<nazwa>.psd`
  - Nazwy warstw: z EXR (pusta → "Beauty")
  - Głębia: 16‑bit, kompresja Raw, bez gamma (v1)

### Minimalne szkielety modułów (podgląd API)

```rust
// src/bin/exr2psd.rs
fn main() -> anyhow::Result<()> {
    // parse args (1..=2), build paths
    // exr_layers::read_layers(input) -> (Vec<PsdLayer>, PsdLayer /*composite*/)
    // psd_writer::write_psd(File::create(out)?, &layers, &composite)
    Ok(())
}
```

```rust
// src/exr_layers.rs
pub struct PsdLayer { pub name: String, pub width: u32, pub height: u32, pub r: Vec<u16>, pub g: Vec<u16>, pub b: Vec<u16>, pub a: Option<Vec<u16>> }
pub fn read_layers(path: &std::path::Path) -> anyhow::Result<(Vec<PsdLayer>, PsdLayer)> {
    // odczyt EXR (flat layers), mapowanie RGBA/RGB/1‑kanał, f32→u16
    unimplemented!()
}
```

```rust
// src/psd_writer.rs
use std::io::Write;
use crate::exr_layers::PsdLayer;
pub fn write_psd<W: Write>(mut out: W, layers: &[PsdLayer], composite: &PsdLayer) -> anyhow::Result<()> {
    // header → color mode → resources → layer & mask (warstwy) → image data (composite)
    unimplemented!()
}
```

### Budowanie drugiego EXE

- Bezpośrednio z Cargo:
  - `cargo build --release --bin exr2psd`
- Przez skrypt:
  - `python build.py --bin exr2psd`
  - Wynik: `dist/exr2psd.exe`

### Kryteria ukończenia v1

- [ ] Uruchomienie `exr2psd input.exr` tworzy `input.psd` z widocznymi warstwami w PS/GIMP/Affinity
- [ ] Obsługa RGBA/RGB/1‑kanał
- [ ] 16‑bit, kompresja Raw
- [ ] Composite poprawny

### Testy ręczne / walidacja

- Otworzyć wynik w Photoshop, GIMP, Affinity:
  - Warstwy widoczne, nazwy poprawne, alpha działa.
  - Composite poprawnie widoczny jako podgląd.

### Rozszerzenia (po v1)

- Kompresja kanałów RLE (PackBits=1) – mniejsze pliki, zgodność dobra.
- ICC (sRGB) i gamma flagi – poprawna interpretacja w edytorach.
- Warstwy o różnych rozmiarach: umieszczanie z offsetem (Rect != canvas), opcjonalnie przezroczyste rozszerzanie.
- Maski warstw, grupy warstw, tryby mieszania inne niż `norm`.

### Ryzyka / pułapki

- PSD wymaga ścisłych długości bloków i paddingu – należy rozważnie liczyć sumy i wyrównania do parzystych bajtów.
- Różne edytory mają odchylenia w interpretacji – testy A/B na PS, GIMP, Affinity.
- Duże obrazy i wiele warstw → pamięć i rozmiar pliku (Raw 16‑bit).

### Kamienie milowe (krótki plan prac)

1. `psd_writer.rs`: header + composite zapis (RGB 16‑bit Raw).
2. Warstwy: rekordy + kanały Raw; alpha.
3. `exr_layers.rs`: mapowanie EXR → PsdLayer (RGBA/RGB/1‑kanał).
4. `src/bin/exr2psd.rs`: CLI, obsługa ścieżek, błędy.
5. Testy ręczne na 2–3 plikach EXR.
6. (opcjonalnie) flaga `--srgb` i RLE.

## Status prac

### Zrobione

- Dodano dokument `psd.md` (plan, architektura, kontrakt CLI, struktura plików, kryteria v1).
- Przygotowano szkielety drugiego EXE:
  - `src/bin/exr2psd.rs` – parsowanie argumentów, integracja `exr_layers::read_layers` i `psd_writer::write_psd` (na razie placeholdery).
  - `src/exr_layers.rs` – definicja `PsdLayer`, placeholder `read_layers(...)`.
  - `src/psd_writer.rs` – placeholder `write_psd(...)`.
- Skrypt `build.py` buduje finalny artefakt i kopiuje do `dist/`; dodano obowiązkowe czyszczenie `target` przed buildem; wsparcie dla `--bin exr2psd`.

### Do zrobienia (v1)

- Zaimplementować `exr_layers::read_layers(path)`:
  - Odczyt „flat layers” przez `exr`.
  - Mapowanie kanałów do R/G/B/A (1‑kanał → duplikacja do RGB).
  - Walidacja rozmiarów; wybór `composite` (Beauty/pierwsza RGB).
  - Konwersja f32 → u16 (clamp [0,1], skala 0..65535).
- Zaimplementować `psd_writer::write_psd(out, layers, composite)`:
  - Header (8BPS, v1, RGB, 16‑bit) + puste Color Mode Data/Image Resources.
  - Layer & Mask Info: rekordy warstw (rect = canvas, blend `norm`, opacity 255, nazwy), kanały (-1 A, 0 R, 1 G, 2 B), dane wierszami, big‑endian u16, kompresja Raw (0).
  - Image Data (Composite) 16‑bit Raw.
  - Poprawne długości bloków i padding do parzystych bajtów.
- Testy ręczne: Photoshop/GIMP/Affinity (widoczność warstw, alpha, composite).

### Opcjonalnie (po v1)

- Flaga `--srgb` (transformacja do sRGB + gamma) i/lub RLE (PackBits=1).
- Warstwy o różnych rozmiarach (rect z offsetem) i maski warstw.
- ICC (sRGB) w Image Resources; dodatkowe metadane.
