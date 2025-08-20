

  ŚREDNIE (Code Quality):

  5. Modułowa reorganizacja - łatwiejsze utrzymanie
                          Faza 1: Hierarchiczna struktura katalogów
      - Utworzenie podkatalogów: processing/, io/, ui/, platform/, utils/
      - Przeniesienie modułów do odpowiednich katalogów
      - Dodanie plików mod.rs z publicznymi interfejsami
     
     Faza 2: Dekompozycja dużych modułów
     - Podział ui_handlers.rs na mniejsze, specjalizowane handlery
     - Wyodrębnienie logiki biznesowej z main.rs
     - Stworzenie czytelnych API między modułami
     
     Faza 3: Definicja interfejsów
     - Jasne publiczne API dla każdej grupy modułów
     - Redukcja bezpośrednich zależności między modułami
     - Implementacja dependency injection gdzie to możliwe
     
     To ukończy reorganizację modułową zgodnie z założeniami.
  6. Error handling - lepsze debugging i recovery
  7. Code deduplication - mniej duplikacji logiki

  NISKIE (Long-term):

  8. Async migration - lepsze UI responsiveness
  9. Configuration system - większa elastyczność
  10. Testing infrastructure - reliability