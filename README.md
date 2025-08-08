# rustExR - Przeglądarka plików EXR

Aplikacja do przeglądania i edycji plików EXR napisana w Rust z interfejsem Slint.

## Struktura projektu po refaktoryzacji

### Moduły

#### `src/main.rs`
- Główna funkcja aplikacji
- Inicjalizacja UI i callbacków
- Koordynacja między modułami

#### `src/image_cache.rs`
- Struktura `ImageCache` do cache'owania obrazów
- Wczytywanie plików EXR do pamięci
- Przetwarzanie obrazów z cache'a

#### `src/image_processing.rs`
- Algorytmy przetwarzania obrazów
- Tone mapping (Reinhard)
- Gamma correction
- Optymalizowane przetwarzanie pikseli

#### `src/file_operations.rs`
- Operacje na plikach
- Dialog wyboru plików
- Sprawdzanie obsługiwanych formatów
- Pobieranie nazw plików

#### `src/ui_handlers.rs`
- Obsługa callbacków UI
- Handlery dla ekspozycji i gammy
- Obsługa otwierania plików
- Zarządzanie stanem aplikacji

#### `ui/appwindow.slint`
- Definicja interfejsu użytkownika
- Layout aplikacji
- Kontrolki (slidery, przyciski)

## Funkcjonalności

- **Wczytywanie plików EXR** - obsługa formatu HDR
- **Korekta ekspozycji** - regulacja jasności w stopniach EV
- **Korekta gamma** - regulacja krzywej tonalnej
- **Cache obrazów** - szybkie przetwarzanie bez ponownego wczytywania
- **Przetwarzanie równoległe** - wykorzystanie biblioteki rayon
- **Interfejs Slint** - nowoczesny UI

## Technologie

- **Rust** - język programowania
- **Slint** - framework UI
- **exr** - biblioteka do obsługi plików EXR
- **rayon** - przetwarzanie równoległe
- **rfd** - dialogi wyboru plików

## Kompilacja

```bash
cargo build --release
```

## Uruchomienie

```bash
cargo run
```

## Refaktoryzacja

Projekt został poddany refaktoryzacji w celu:
- Lepszej organizacji kodu
- Separacji odpowiedzialności
- Łatwiejszego utrzymania
- Możliwości rozszerzania funkcjonalności

### Przed refaktoryzacją
- Cała logika w jednym pliku `main.rs` (279 linii)
- Mieszanie różnych odpowiedzialności
- Trudność w utrzymaniu i rozwijaniu

### Po refaktoryzacji
- Podział na 5 modułów według logiki funkcjonalnej
- Jasne rozdzielenie odpowiedzialności
- Łatwiejsze testowanie i rozwijanie
- Lepsza czytelność kodu
