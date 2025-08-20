# Faza 2: Dekompozycja dużych modułów - Szczegółowy Plan

## 🎯 STATUS: 3/7 kroków ukończone (43%) + Cleanup

### ✅ UKOŃCZONE:
- **Krok 1:** State Management - pełny sukces ✅
- **Krok 2:** Layer Operations - pełny sukces ✅  
- **Krok 3:** Image Controls - pełny sukces ✅
- **Cleanup:** Wszystkie błędy kompilacji i warningi naprawione ✅

### 🔧 GOTOWE DO DALSZEGO REFAKTORINGU:
- Kompilacja: 0 błędów, 0 warningów 
- Struktura modułów czysta i gotowa
- State management w pełni funkcjonalny

## Analiza obecnej struktury

### ui_handlers.rs (679 linii, było 981) - Główne problemy:
1. **Zbyt dużo odpowiedzialności** - ~~obsługa UI, state management~~ ✅ EXTRACTED, async operations
2. **Globalne static zmienne** - ~~ITEM_TO_LAYER, DISPLAY_TO_REAL_LAYER~~ ✅ MOVED TO STATE, ~~LAST_PREVIEW_LOG~~ ✅ MOVED TO IMAGE_CONTROLS
3. **Mieszane concerns** - ~~UI callbacks~~ ✅ PARTIALLY EXTRACTED, business logic, async spawning
4. **Duże funkcje** - load_thumbnails_for_directory (150+ linii), handle_open_exr_from_path (270+ linii)

### main.rs (483 linii, było 477) - Problemy:
1. **Zbyt dużo setup logiki** - wszystkie callbacks w main
2. **Brak separacji** - inicjalizacja, konfiguracja i setup w jednym miejscu
3. **Powtarzające się wzorce** - podobne callback setups

## Plan dekompozycji - 7 kroków

### ✅ Krok 1: Wyodrębnienie State Management - UKOŃCZONY
**Cel:** Usunięcie globalnych static i centralizacja stanu
**Pliki:** `src/ui/state.rs` ✅

```rust
// state.rs
pub struct UiState {
    pub item_to_layer: HashMap<String, String>,
    pub display_to_real_layer: HashMap<String, String>, 
    pub current_file_path: Option<PathBuf>,
    pub last_preview_log: Option<Instant>,
}

pub type SharedUiState = Arc<Mutex<UiState>>;
```

### ✅ Krok 2: Wyodrębnienie Layer Operations - UKOŃCZONY
**Cel:** Izolacja logiki obsługi warstw
**Pliki:** `src/ui/layers.rs` ✅

**Funkcje przeniesione:** ✅
- ✅ `handle_layer_tree_click()` - obsługa kliknięć w drzewo warstw
- ✅ `create_layers_model()` - tworzenie modelu warstw dla UI  
- ✅ Logika mapowania warstw (display ↔ real)

### ✅ Cleanup Phase - UKOŃCZONY
**Cel:** Usunięcie wszystkich błędów kompilacji i warningów
**Wykonane działania:** ✅
- ✅ Usunięto starą funkcję `handle_layer_tree_click` z ui_handlers.rs (~180 linii)
- ✅ Usunięto nieużywaną funkcję `create_layers_model` z layers.rs  
- ✅ Poprawiono scope errors dla `ui_state` w main.rs
- ✅ Usunięto nieużywane importy (normalize_channel_name, ModelRc, Color)
- ✅ Dodano `#[allow(dead_code)]` dla elementów state.rs (przygotowane na dalszy refaktoring)
- ✅ Kompilacja: 0 błędów, 0 warningów

### ✅ Krok 3: Wyodrębnienie Image Controls - UKOŃCZONY
**Cel:** Separacja kontroli parametrów obrazu
**Pliki:** `src/ui/image_controls.rs` ✅

**Funkcje przeniesione:** ✅
- ✅ `ThrottledUpdate` struct i implementacja (39 linii)
- ✅ `handle_parameter_changed_throttled()` (31 linii)
- ✅ `update_preview_image()` (49 linii)
- ✅ Logika exposure/gamma/tonemap wraz z LAST_PREVIEW_LOG
- ✅ Re-eksporty dla zachowania kompatybilności

### Krok 4: Wyodrębnienie Thumbnail Operations
**Cel:** Izolacja operacji na miniaturkach  
**Pliki:** `src/ui/thumbnails.rs`

**Funkcje do przeniesienia:**
- `load_thumbnails_for_directory()` - ładowanie miniaturek
- Async processing logic dla folderów
- Thumbnail navigation logic

### Krok 5: Wyodrębnienie File Operations
**Cel:** Centralizacja operacji na plikach
**Pliki:** `src/ui/file_handlers.rs`

**Funkcje do przeniesienia:**
- `handle_open_exr()` 
- `handle_open_exr_from_path()`
- `load_metadata()`
- File dialog handling

### Krok 6: Wyodrębnienie Callback Setup
**Cel:** Organizacja setup logiki z main.rs
**Pliki:** `src/ui/setup.rs`

**Funkcje do przeniesienia z main.rs:**
- `setup_menu_callbacks()`
- `setup_image_control_callbacks()`  
- `setup_panel_callbacks()`
- `setup_ui_callbacks()`

### Krok 7: Refaktor ui_handlers.rs
**Cel:** Pozostawienie tylko kodu koordynującego
**Zawartość finalna:**
- Utility functions (safe_lock, lock_or_recover)
- Constants (THUMBNAIL_HEIGHT)
- Re-exports z innych modułów
- Główne typy (ImageCacheType, etc.)

## Hierarchia zależności po refaktorze

```
src/ui/
├── mod.rs              # Re-exports, typy publiczne ✅
├── state.rs            # Zarządzanie stanem (0 deps UI) ✅
├── layers.rs           # Obsługa warstw (dep: state) ✅
├── progress.rs         # Progress handling ✅
├── image_controls.rs   # Kontrole obrazu (dep: state) ✅
├── thumbnails.rs       # Miniaturki (dep: state) ❌
├── file_handlers.rs    # Pliki (dep: state, layers) ❌
├── setup.rs            # Callbacks setup (dep: wszystkie) ❌
└── ui_handlers.rs      # Utils + koordinacja (dep: wszystkie) ⚠️
```

## Korzyści

### ✅ **Już osiągnięte:**
1. **Clean compilation** - 0 błędów, 0 warningów
2. **Centralized state** - usunięto globalne static zmienne
3. **Better organization** - layer operations i image controls wydzielone
4. **Reduced code duplication** - usunięto duplikaty funkcji
5. **Smaller files** - ui_handlers.rs: 981→679 linii (-302 linii)
6. **Image controls separation** - throttling i preview logic w osobnym module

### 🎯 **Do osiągnięcia (kroki 4-7):**
1. **Łatwiejsze utrzymanie** - każdy moduł ma jedną odpowiedzialność
2. **Lepsze testowanie** - można testować moduły w izolacji  
3. **Redukcja coupling** - czyste interfejsy między modułami
4. **Przyszłe rozwój** - łatwiejsze dodawanie funkcji
5. **Code reuse** - funkcje można używać w innych kontekstach

## Migracja bez breaking changes

- Zachowanie wszystkich publicznych API
- Postupowa migracja z re-exports
- Wsteczna kompatybilność dla users
- Zero impact na existing imports

## Effort

### ✅ **Wykonane (3.5h):**
- Krok 1: State Management (1h)
- Krok 2: Layer Operations (1h)  
- Krok 3: Image Controls (1h)
- Cleanup: Błędy i warningi (0.5h)

### 🎯 **Pozostało (1.5-2h):**
- Kroki 4-7: Thumbnails, File Handlers, Setup, Final Refactor

**Każdy krok można wykonać i przetestować niezależnie.**