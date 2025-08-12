# Raport z analizy kodu UI (Slint)

Poniższy dokument zawiera analizę i rekomendacje dotyczące kodu interfejsu użytkownika napisanego w języku Slint. Celem jest poprawa jego czytelności, reużywalności, spójności oraz uproszczenie logiki, co przełoży się na łatwiejsze utrzymanie i rozwój aplikacji.

## 1. Komponentyzacja i Redukcja Duplikacji Kodu

Zidentyfikowano wiele miejsc, w których te same lub bardzo podobne wzorce UI są powielane, zamiast być wydzielone do reużywalnych komponentów.

1.  **Problem:** Definicje przycisków w oknach dialogowych (`console_window.slint`, `meta_window.slint`) oraz w dolnym panelu `appwindow.slint` są tworzone w miejscu przez kompozycję `Rectangle`, `Text` i `TouchArea`. Jest to powielanie tego samego kodu.

2.  **Rozwiązanie:** Należy stworzyć jeden, generyczny komponent przycisku, który będzie można wykorzystać we wszystkich tych miejscach. Komponent `PrzyciskAkcji` jest dobrym punktem wyjścia, ale można stworzyć bardziej ogólny `StyledButton`, który akceptowałby tekst, ikonę i callback jako parametry.

3.  **Zadania dla AI:**
    *   Utwórz nowy komponent w `ui/components/`, np. `DialogButton.slint`.
    *   Zastąp ręcznie tworzone przyciski "Clear", "Close" w `console_window.slint` i `meta_window.slint` nowym komponentem.
    *   Zastąp przyciski "Hide/Show Panel", "Select working folder" w `appwindow.slint` tym samym komponentem, przekazując odpowiedni tekst i callback.

## 2. Uproszczenie Układu (Layout) i Logiki

Główny plik `appwindow.slint` zawiera bardzo skomplikowaną i trudną w utrzymaniu logikę do zarządzania układem kolumn.

1.  **Problem:** W `appwindow.slint` znajduje się blok kilkunastu właściwości (`_menu_col1_eff`, `_n1`, `_n2`, `_right_panel_width` itd.), które ręcznie obliczają procentowe szerokości kolumn i pozycje elementów. Jest to niepotrzebnie skomplikowane, podatne na błędy i trudne do zrozumienia.

2.  **Rozwiązanie:** Należy całkowicie usunąć te ręczne obliczenia i wykorzystać wbudowane w Slint mechanizmy układu. Główny `HorizontalBox` zawierający trzy kolumny może zarządzać szerokościami automatycznie.

3.  **Zadania dla AI:**
    *   Usuń wszystkie pomocnicze właściwości (`_menu_*`, `_col*`, `_n*`, `_sum_eff`) z `appwindow.slint`.
    *   W głównym `HorizontalBox` (trzy kolumny) użyj właściwości `preferred-width` z wartościami procentowymi (np. `parent.width * root.column1-percent`) dla bocznych paneli.
    *   Dla środkowej kolumny użyj właściwości `horizontal-stretch: 1`, aby automatycznie wypełniła pozostałą przestrzeń. To wyeliminuje potrzebę ręcznego sumowania i normalizacji szerokości.

## 3. Wykorzystanie Standardowych Komponentów Slint

W niektórych miejscach funkcjonalność, dla której istnieją standardowe komponenty Slint, została zaimplementowana ręcznie.

1.  **Problem:** W `meta_window.slint` widok tabelaryczny metadanych jest zaimplementowany za pomocą pętli `for` generującej komponenty `Rectangle` i `Text` wewnątrz `ScrollView`. Jest to nieefektywne dla długich list i bardziej skomplikowane niż to konieczne.

2.  **Rozwiązanie:** Należy użyć wbudowanego komponentu `ListView` z `std-widgets.slint`, który jest zoptymalizowany pod kątem wyświetlania list (wirtualizacja wierszy) i znacznie upraszcza kod.

3.  **Zadania dla AI:**
    *   W `meta_window.slint` zastąp `ScrollView` i pętlę `for` komponentem `ListView`.
    *   Przekaż modele `meta-table-keys` i `meta-table-values` do `ListView`.
    *   Zdefiniuj wygląd pojedynczego wiersza (delegata) wewnątrz `ListView`, który będzie renderował klucz i wartość. To uprości logikę i poprawi wydajność.

## 4. Poprawa Spójności i Doświadczenia Użytkownika (UX)

Można wprowadzić drobne zmiany, które poprawią spójność wizualną i uczynią interfejs bardziej intuicyjnym.

1.  **Problem:** Komponent `ParameterSlider` ma na stałe zakodowane kolory gałki suwaka (`Kolory.ekspozycja_galka*`), co utrudnia jego ponowne użycie dla innych parametrów (np. Gamma) z inną kolorystyką.

2.  **Rozwiązanie:** Komponenty powinny być jak najbardziej generyczne. Kolory i inne aspekty stylu powinny być przekazywane jako właściwości.

3.  **Zadania dla AI:**
    *   Do `ParameterSlider.slint` dodaj nowe właściwości, np. `in property <color> handle-color;` oraz `in property <color> handle-hover-color;`.
    *   Użyj tych właściwości do ustawienia tła gałki suwaka, zamiast odwoływać się bezpośrednio do globalnych kolorów.
    *   W `appwindow.slint`, podczas używania `ParameterSlider` dla ekspozycji i gammy, przekaż odpowiednie kolory.

4.  **Problem:** Separatory do zmiany rozmiaru kolumn są funkcjonalne, ale niewidoczne. Użytkownik może nie wiedzieć, że może zmienić szerokość kolumn.

5.  **Rozwiązanie:** Dodać wizualną informację zwrotną, gdy kursor myszy znajduje się nad separatorem.

6.  **Zadania dla AI:**
    *   W `appwindow.slint`, dla `Rectangle` pełniącego rolę separatora, dodaj logikę, która zmienia jego tło (np. na `Kolory.hover`), gdy `TouchArea` wewnątrz niego ma stan `has-hover`.

## Podsumowanie

Sugerowane zmiany, zwłaszcza refaktoryzacja logiki układu w `appwindow.slint` i większa komponentyzacja, znacząco poprawią jakość i łatwość utrzymania kodu UI. Wprowadzenie `ListView` i sparametryzowanie komponentów uczyni kod bardziej wydajnym i elastycznym.