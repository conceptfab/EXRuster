cPropozycje wdrożeń (na podstawie claude.md)

1. LRU cache miniaturek

- Cel: przyspieszyć ponowne wczytywanie miniaturek w katalogach.
- Zakres: cache RGBA8 po kluczu (ścieżka pliku, czas modyfikacji, preset: tonemap/exp/gamma).
- Korzyści: szybsze otwieranie folderów, mniejsze zużycie CPU przy powrotach do tych samych katalogów.

2. Nieblokujące wczytywanie pełnego EXR

- Cel: uniknąć blokowania UI przy dużych plikach EXR.
- Zakres: przenieść `build_full_exr_cache` do wątku roboczego (Rayon); w UI pokazywać postęp i gotowość po zakończeniu; bezpieczne przekazanie wyniku do wątku UI.
- Korzyści: responsywny interfejs nawet dla bardzo dużych plików.

3. Cache MIP (przeskalowanych podglądów) dla bieżącej warstwy

- Cel: szybsza reakcja na zmiany ekspozycji/gammy/tonemappingu i przełączanie trybów podglądu.
- Zakres: utrzymywać 1–2 poziomy przeskalowanych buforów float (np. 1/2 i 1/4 rozdzielczości) wyliczanych raz, a następnie przetwarzanych tylko w tonemap/gamma.
- Korzyści: mniejsza latencja aktualizacji podglądu, lżejsze obciążenie CPU.

4. Tryb „light open” (bez pełnego cache na starcie)

- Cel: skrócić czas otwarcia i zmniejszyć zużycie pamięci dla dużych EXR.
- Zakres: strumieniowo wczytywać tylko wybraną warstwę do podglądu; pozostałe warstwy/kanały dogrywać na żądanie; przełącznik trybu w logice otwierania pliku.
- Korzyści: szybszy start pracy z plikiem, lepsza skalowalność na słabszych maszynach.
