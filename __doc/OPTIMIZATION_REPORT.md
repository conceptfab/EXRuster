# Raport Optymalizacji Kodu `EXRuster`

Analiza kodu została przeprowadzona pod kątem maksymalizacji wydajności i responsywności aplikacji, z unikaniem nadmiernej inżynierii (over-engineering). Poniżej przedstawiono plan działań dla modelu AI, podzielony na logiczne etapy, które należy wdrożyć w celu optymalizacji.

## Etap 1: Usunięcie Blokowania Wątku UI (Najwyższy Priorytet)

Obecnie operacje eksportu (do formatów TIFF i PNG) są wykonywane w głównym wątku aplikacji, co powoduje zamrożenie interfejsu użytkownika na czas trwania operacji. Jest to najbardziej krytyczny problem do rozwiązania.

1.  **Zidentyfikuj blokujące operacje w `src/ui_handlers.rs`**: Funkcje `handle_export_convert`, `handle_export_beauty` oraz `handle_export_channels` zawierają pętle intensywnie przetwarzające piksele oraz operacje zapisu do plików. Cała ta logika musi zostać przeniesiona z wątku UI.

2.  **Przenieś logikę eksportu do wątku roboczego**: Dla każdej z trzech funkcji eksportu (`handle_export_*`), całą logikę przetwarzania i zapisu pliku należy opakować w `rayon::spawn` lub `std::thread::spawn`. Do komunikacji zwrotnej z UI (aktualizacja paska postępu, informowanie o zakończeniu lub błędzie) należy użyć `slint::invoke_from_event_loop`.

    *Plik do modyfikacji: `src/ui_handlers.rs`*

## Etap 2: Przyspieszenie Przetwarzania Obrazów

Główne algorytmy przetwarzania obrazu są już dobrze zoptymalizowane przy użyciu Rayon i SIMD. Istnieje jednak pole do dalszej poprawy.

3.  **Zrównoleglij generowanie MIP map**: Funkcja `build_mip_chain` w `src/image_cache.rs` działa jednowątkowo. Dla obrazów o bardzo wysokiej rozdzielczości jej wykonanie może być zauważalne. Chociaż pętla główna (poziomy MIP) musi być sekwencyjna, obliczenia dla pikseli wewnątrz każdego poziomu można łatwo zrównoleglić.

4.  **Zaimplementuj równoległe obliczanie poziomów MIP**: Wewnątrz pętli `for _ in 0..max_levels` w funkcji `build_mip_chain`, pętle `for y_out ... for x_out` należy zastąpić iteratorem `par_chunks_mut` z biblioteki Rayon, aby przetwarzać wiersze lub bloki pikseli nowej MIP mapy równolegle.

    *Plik do modyfikacji: `src/image_cache.rs`*

## Etap 3: Poprawa Płynności Interfejsu Użytkownika

Pewne operacje, mimo że wykonywane asynchronicznie, mogą powodować chwilowe przycięcia UI przy dużej ilości danych.

5.  **Zoptymalizuj aktualizację listy miniaturek**: W `load_thumbnails_for_directory` (`src/ui_handlers.rs`), konwersja danych pikseli na `slint::Image` i aktualizacja modelu `VecModel` odbywa się w jednej, dużej operacji wewnątrz `invoke_from_event_loop`. Dla folderów z setkami obrazów może to spowodować widoczne opóźnienie.

6.  **Wprowadź wsadową (batching) aktualizację miniaturek**: Zamiast aktualizować cały `VecModel` na raz, należy podzielić listę `ThumbItem` na mniejsze paczki (np. po 20-30 elementów) i dodawać je do modelu UI w kolejnych cyklach pętli zdarzeń przy użyciu `slint::Timer`. Zapewni to płynne pojawianie się miniaturek bez blokowania UI.

7.  **Popraw wydajność logowania do konsoli**: Funkcja `push_console` w `src/ui_handlers.rs` przy każdej nowej linii odczytuje i zapisuje ponownie całą zawartość pola tekstowego konsoli. Jest to nieefektywne przy dużej liczbie logów.

8.  **Zrefaktoryzuj `push_console`**: Należy usunąć z funkcji `push_console` kod modyfikujący `SharedString` (`ui.set_console_text`). Wystarczy operacja `console.push(...)` na `VecModel`. Aby to zadziałało poprawnie, w pliku `.slint` komponent `TextEdit` służący za konsolę powinien zostać zastąpiony przez `ListView`, który jest znacznie wydajniejszy do wyświetlania dynamicznych list.

    *Plik do modyfikacji: `src/ui_handlers.rs`*

## Etap 4: Odblokowanie Akceleracji GPU (Największy Potencjalny Wzrost Wydajności)

Projekt zawiera kod do obsługi GPU (`gpu_context.rs`) oraz gotowy shader obliczeniowy (`image_processing.wgsl`), ale nie są one używane do głównego przetwarzania obrazu. Uruchomienie tej ścieżki da największy skok wydajności.

9.  **Zintegruj potok renderowania GPU**: W `src/image_cache.rs` należy stworzyć nową ścieżkę wykonania w funkcjach `process_to_image` i `process_to_thumbnail`. Powinna ona sprawdzać, czy akceleracja GPU jest dostępna i włączona.

10. **Zaimplementuj logikę shadera obliczeniowego**: Jeśli GPU jest aktywne, aplikacja powinna:
    a. Utworzyć na GPU bufory: wejściowy (z `raw_pixels`), wyjściowy (na przetworzony obraz) i uniform (z parametrami `exposure`, `gamma` etc.).
    b. Skonfigurować i uruchomić shader `image_processing.wgsl` poprzez `wgpu::ComputePassEncoder`.
    c. Skopiować dane z bufora wyjściowego GPU z powrotem do pamięci CPU.
    d. Z otrzymanych danych stworzyć `slint::Image`.
    Istniejąca implementacja CPU (Rayon + SIMD) powinna pozostać jako alternatywa (fallback), gdy GPU jest niedostępne lub wyłączone.

    *Plik do modyfikacji: `src/image_cache.rs` (oraz potencjalnie `src/gpu_context.rs` na dodatkowe funkcje pomocnicze)*
