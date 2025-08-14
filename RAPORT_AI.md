# Raport Poprawek Kodu - EXRuster

Poniżej znajduje się lista zalecanych zmian w kodzie projektu, mających na celu poprawę jego jakości, wydajności i użyteczności, zgodnie z Twoimi wytycznymi.

### Pliki wymagające uwagi:
- `src/exr_metadata.rs`
- `src/image_cache.rs`
- `src/gpu_context.rs`
- `src/thumbnails.rs`
- `src/ui_handlers.rs`
- `src/full_exr_cache.rs`
- `src/utils.rs`

---

### Plan działania:

1.  **Refaktoryzacja modułu `exr_metadata.rs`:**
    *   Wydzielenie logiki normalizacji i formatowania atrybutów EXR do dedykowanych funkcji pomocniczych. Obecnie ten sam kod jest powielony w dwóch miejscach – raz dla atrybutów globalnych (`shared_attributes`) i drugi raz dla atrybutów specyficznych dla warstwy (`own_attributes`).
    *   Usunięcie nieużywanej struktury `LayerChannelsGroup` oraz pola `channel_groups` ze struktury `LayerMetadata`. Logika grupowania kanałów (`GroupBuckets`, `classify_channel_group`) jest obliczana, ale jej wyniki nie są nigdzie wyświetlane w interfejsie użytkownika.

2.  **Refaktoryzacja modułu `image_cache.rs`:**
    *   Połączenie zduplikowanej logiki normalizacji nazw kanałów z funkcji `image_cache::channel_alias_to_short` i `utils::normalize_channel_name` w jedną, wspólną funkcję w module `utils.rs`.
    *   Refaktoryzacja funkcji `process_to_composite` i `process_to_image` w celu usunięcia powielonego kodu. Obie funkcje realizują ten sam potok przetwarzania pikseli i mogą zostać połączone w jedną, bardziej elastyczną funkcję.
    *   Zunifikowanie logiki przetwarzania w `process_to_thumbnail`, aby generowanie miniatur ponownie wykorzystywało potok z `process_to_image`, co pozwoli uniknąć powtarzania kodu transformacji kolorów i korekcji gamma/tone mappingu.
    *   Zrównoleglenie generowania mipmap w funkcji `build_mip_chain` przy użyciu biblioteki Rayon, co przyspieszy tworzenie podglądów dla dużych obrazów.

3.  **Refaktoryzacja ścieżki GPU (`gpu_context.rs` i `image_cache.rs`):**
    *   Przeniesienie logiki tworzenia zasobów WGPU (potoków, bind groups, etc.) z funkcji `image_cache::process_to_image_gpu` do dedykowanych, ale obecnie nieużywanych, metod w `GpuContext`. Scentralizuje to operacje GPU i uporządkuje kod w `image_cache.rs`.
    *   Usunięcie atrybutów `#[allow(dead_code)]` z metod w `gpu_context.rs`, które po refaktoryzacji będą używane.
    *   Zastąpienie wywołań `println!` w ścieżce renderowania GPU wywołaniami funkcji `push_console`, aby komunikaty diagnostyczne były widoczne w konsoli aplikacji, a nie tylko w terminalu.

4.  **Refaktoryzacja modułu `thumbnails.rs`:**
    *   Ścieżka awaryjna (fallback) w funkcji `generate_single_exr_thumbnail_work` powiela logikę przetwarzania pikseli z `image_cache.rs`. Należy ją zrefaktoryzować, aby wywoływała scentralizowaną funkcję, co zmniejszy duplikację kodu.

5.  **Usprawnienie wizualizacji postępu (Progress Bar):**
    *   **`full_exr_cache::build_full_exr_cache`**: Dodanie bardziej szczegółowych aktualizacji paska postępu wewnątrz pętli przetwarzających piksele i kanały. Obecne aktualizacje są zbyt rzadkie dla tak długiej operacji.
    *   **`ui_handlers::handle_open_exr_from_path`**: Funkcja ta orkiestruje wieloetapowy proces ładowania pliku. Każdy krok (odczyt metadanych, budowa cache, przetwarzanie obrazu) powinien raportować swój postęp do `UiProgress`, aby użytkownik miał płynniejszy feedback.
    *   **`ui_handlers::handle_export_beauty`**: Eksport do PNG dla dużych obrazów może być czasochłonny i obecnie nie posiada paska postępu. Należy dodać wizualizację postępu dla tej operacji.

6.  **Ogólne porządki w kodzie:**
    *   **`full_exr_cache.rs`**: Usunięcie nieużywanej funkcji `FullLayer::channel_slice`.
    *   **`ui_handlers.rs`**: Plik jest bardzo duży i zawiera logikę dla wielu różnych części aplikacji. Warto rozważyć jego podział na mniejsze, bardziej wyspecjalizowane moduły (np. `export_handlers.rs`, `view_handlers.rs`), co znacząco poprawi czytelność i łatwość utrzymania kodu.
