# Raport optymalizacji wydajności

Poniższa lista przedstawia zadania dla modelu AI w celu poprawy szybkości działania aplikacji. Zmiany koncentrują się na redukcji operacji wejścia-wyjścia (I/O) oraz optymalizacji przetwarzania danych, unikając jednocześnie nadmiernej inżynierii.

### Zadania do wykonania:

1.  **Zoptymalizuj odczyt metadanych EXR.**
    W wielu miejscach (`color_processing.rs`, `exr_metadata.rs`, `image_cache.rs`) kod wczytuje całe pliki EXR wraz z danymi pikseli tylko po to, by odczytać metadane (atrybuty, nazwy warstw). Należy zmodyfikować te funkcje, aby używały dedykowanych metod z biblioteki `exr` do wczytywania wyłącznie nagłówków, co drastycznie zredukuje operacje I/O i zużycie pamięci.

2.  **Używaj bezpośrednio typowanych atrybutów.**
    W plikach `color_processing.rs` i `exr_metadata.rs` atrybut `chromaticities` jest odczytywany poprzez konwersję do formatu tekstowego (`Debug`), a następnie parsowany. Jest to powolne i podatne na błędy. Zmień kod tak, aby bezpośrednio korzystał z wariantu `AttributeValue::Chromaticities` i jego pól, co jest znacznie szybsze i bezpieczniejsze.

3.  **Zoptymalizuj eksport danych przez jednorazowy odczyt.**
    Funkcje eksportu w `ui_handlers.rs` (`handle_export_convert`, `handle_export_channels`) wczytują dane z pliku wielokrotnie w pętli dla każdej warstwy lub kanału. Zrefaktoryzuj logikę, aby wszystkie potrzebne dane pikseli z pliku EXR były wczytywane do pamięci **jeden raz**. Następnie wszystkie operacje eksportu powinny być wykonywane na danych znajdujących się już w pamięci.

4.  **Przyspiesz generowanie miniatur.**
    W `thumbnails.rs` proces tworzenia miniatur powinien wczytywać dane pikseli tylko raz na plik. Co ważniejsze, jeśli pliki EXR zawierają mip-mapy, wykorzystaj je, wczytując najmniejszy odpowiedni poziom rozdzielczości zamiast obrazu w pełnej jakości. Jeśli mip-mapy nie są dostępne, obecne skalowanie jest akceptowalne, ale początkowy odczyt danych wciąż musi być zoptymalizowany (patrz punkt 1).

5.  **Wyeliminuj zbędne operacje I/O przy zmianie kanału.**
    W `ui_handlers.rs` przełączanie widoku na pojedynczy kanał w `handle_layer_tree_click` powoduje ponowne wczytanie danych z dysku. Zmodyfikuj `ImageCache`, aby po wczytaniu warstwy przechowywał w pamięci wszystkie jej kanały. Dzięki temu przełączanie widoków (np. z kompozytu RGB na widok pojedynczego kanału w skali szarości) będzie odbywać się wyłącznie na danych w pamięci, bez zbędnych operacji I/O.
