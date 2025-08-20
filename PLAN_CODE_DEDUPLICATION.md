# Plan Realizacji: Code Deduplication - EXRuster

## **Faza 1: Fundamenty (Priorytet Wysoki) - 1-2 dni**

### 1.1 Stworzenie modułu error handling
- **Cel:** Eliminacja 35+ duplikacji obsługi błędów
- **Akcje:**
  - `src/utils/error_handling.rs` - makra i helpery
  - Trait `UiErrorReporter` dla spójnego raportowania
  - Makro `handle_ui_error!` dla standardowych wzorców

### 1.2 System logowania konsoli  
- **Cel:** Zastąpienie 25+ identycznych wzorców
- **Akcje:**
  - Trait `ConsoleLogger` z metodami `log_error`, `log_info`
  - Builder pattern dla formatowanych komunikatów
  - Extension methods dla UI

### 1.3 Refaktoryzacja konwersji histogramów
- **Cel:** Eliminacja duplikacji w `file_handlers.rs:237` i `setup.rs:59`
- **Akcje:**
  - `HistogramData::to_ui_models()` method
  - Generic converter `Vec<f32> → ModelRc<i32>`

## **Faza 2: Przetwarzanie Obrazu (Priorytet Średni) - 2-3 dni**

### 2.1 Unifikacja pipeline przetwarzania
- **Cel:** Łączenie funkcji `process_to_*` w `image_cache.rs`
- **Akcje:**
  - Wspólna funkcja `process_pixels_to_image<F>` z closurem
  - Builder pattern dla konfiguracji przetwarzania
  - Template function dla różnych output typów

### 2.2 Progress reporting system
- **Cel:** Zastąpienie 10+ powtórzeń `UiProgress`
- **Akcje:**
  - RAII wrapper `ScopedProgress`
  - Extension methods dla automatycznego zarządzania
  - Chainable API

### 2.3 Struktury metadata/info
- **Cel:** Ujednolicenie `LayerInfo`, `LayerMetadata`, `LazyLayerMetadata`
- **Akcje:**
  - Wspólny trait `LayerDescriptor`
  - Unified `LayerInfo` z optional fields
  - Conversion traits między typami

## **Faza 3: Zaawansowane (Priorytet Niski) - 1-2 dni**

### 3.1 SIMD processing patterns
- **Cel:** Generyczne wzorce wektoryzacji
- **Akcje:**
  - Generic trait `SimdProcessable<T>` gdzie T: f32 | f32x4
  - Makra generujące skalarne i SIMD wersje
  - Template functions dla pixel processing

### 3.2 Cache implementations
- **Cel:** Ujednolicenie różnych implementacji cache
- **Akcje:**
  - Generic `Cache<K, V>` trait
  - Wspólne eviction policies
  - Thread-safe wrappers

## **Struktura Nowych Modułów:**

```
src/utils/
├── error_handling.rs    # Makra i traits error handling
├── logging.rs          # Console logging system  
├── progress.rs         # Progress reporting RAII
├── conversions.rs      # Generic type converters
└── cache.rs           # Generic cache implementations

src/processing/
├── pipeline.rs         # Unified processing pipeline
└── traits.rs          # Wspólne traits dla przetwarzania

src/io/
└── metadata_traits.rs  # Unified metadata interfaces
```

## **Znalezione Duplikacje - Szczegóły:**

### **WYSOKIEJ PRIORYTETY:**

#### 1. **Error Handling i Obsługa Result/Option** 
**Lokalizacja:** Wszystkie moduły (ui/, processing/, io/, utils/)
**Problem:** Identyczne wzorce obsługi błędów powtarzają się w całym kodzie:
```rust
// Powtarzane w 15+ miejscach
match result {
    Ok(value) => { /* success handling */ },
    Err(e) => {
        ui.set_status_text(format!("Error: {}", e).into());
        push_console(&ui, &console, format!("[error] {}", e));
        prog.reset();
    }
}
```

#### 2. **Console Logging Pattern**
**Lokalizacja:** Używane w 25+ miejscach w ui/
**Problem:** Identyczny wzorzec aktualizacji konsoli i status:
```rust
// Powtarzane wszędzie:
push_console(&ui, &console, format!("[error] {}", error_msg));
ui.set_status_text(format!("Error: {}", error_msg).into());
```

#### 3. **Histogram UI Conversion**
**Lokalizacja:** 
- `src/ui/file_handlers.rs:237-245, 127-135`
- `src/ui/setup.rs:59-67`
**Problem:** Identyczna konwersja danych histogramu do UI

### **ŚREDNIE PRIORYTETY:**

#### 1. **Image Processing Pipeline**
**Lokalizacja:**
- `src/io/image_cache.rs:303` (`process_to_image`)
- `src/io/image_cache.rs:354` (`process_to_composite`) 
- `src/io/image_cache.rs:374` (`process_to_thumbnail`)
**Problem:** Niemal identyczne pipeline przetwarzania z drobnymi wariantami

#### 2. **Progress Reporting Pattern**
**Lokalizacja:**
- `src/ui/file_handlers.rs:30, 45, 138`
- `src/ui/thumbnails.rs:25, 75, 114`
- `src/ui/layers.rs:45, 138`
**Problem:** Repetytywne tworzenie i zarządzanie `UiProgress`

#### 3. **Layer/Channel Info Structures**
**Lokalizacja:**
- `src/io/image_cache.rs:44-47` (`LayerInfo`)
- `src/io/exr_metadata.rs:14-18` (`LayerMetadata`)
- `src/io/lazy_exr_loader.rs:13-20` (`LazyLayerMetadata`)
**Problem:** Podobne struktury do reprezentowania warstw i kanałów

### **NISKIE PRIORYTETY:**

#### 1. **SIMD Processing Patterns**
**Lokalizacja:**
- `src/processing/simd_processing.rs:35, 58`
- `src/processing/tone_mapping.rs:217-258`
**Problem:** Podobne wzorce wektoryzacji SIMD dla przetwarzania pikseli

#### 2. **Cache Implementations**
**Lokalizacja:**
- `src/io/thumbnails.rs:305-320` (`ThumbKey`, `ThumbValue`)
- `src/processing/color_processing.rs:8` (HashMap cache)
**Problem:** Różne implementacje cache'owania z podobną logiką

## **Metryki Sukcesu:**

- **Redukcja duplikacji:** ~35% → ~10%
- **Usunięte linie kodu:** ~1500-2000 linii
- **Nowe moduły utility:** 6-8 plików
- **Pokrycie testów:** >90% nowych modułów
- **Performance:** Bez degradacji, potencjalna poprawa

## **Ryzyko i Mitygacja:**

1. **Breaking changes** → Stopniowa migracja modułu po module
2. **Performance impact** → Benchmarki przed/po każdej zmianie  
3. **Complexity growth** → Zachowanie prostych API, ukrycie złożoności
4. **Testing effort** → Unit testy dla każdego nowego utility

## **Szacunek czasowy:**
- **Faza 1:** 1-2 dni (error handling, logging, histogram conversion)
- **Faza 2:** 2-3 dni (image processing, progress, metadata)
- **Faza 3:** 1-2 dni (SIMD patterns, cache unification)
- **Łącznie:** 4-7 dni roboczych

**Łączny szacunek redukcji:** ~35% kodu można zrefaktoryzować eliminując duplikacje, szczególnie w module UI gdzie jest najwięcej powtórzeń.