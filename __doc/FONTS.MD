Metoda 1: Używanie czcionek systemowych
To domyślne i najprostsze podejście. Slint, podobnie jak przeglądarki internetowe, potrafi korzystać z czcionek, które są dostępne w systemie operacyjnym użytkownika (Windows, macOS, Linux).
Jak to działa?
Po prostu podajesz nazwę czcionki we właściwości font-family. Slint poprosi system operacyjny o znalezienie i załadowanie tej czcionki.
Obsługiwane formaty:
Slint korzysta z systemowych bibliotek do renderowania czcionek, więc obsługuje najpopularniejsze formaty, takie jak TrueType (.ttf) i OpenType (.otf).
Jak używać?
Wystarczy przypisać nazwę czcionki do właściwości font-family na elemencie tekstowym.
code
Slint
Text {
    text: "Tekst renderowany czcionką Arial";
    font-size: 20px;
    font-family: "Arial"; // Nazwa czcionki musi być w cudzysłowie
}
Dobra praktyka: Czcionki zapasowe (fallback)
Ponieważ nie masz pewności, czy dana czcionka jest zainstalowana na komputerze użytkownika, warto podać listę czcionek. Slint spróbuje użyć pierwszej, a jeśli jej nie znajdzie, przejdzie do następnej.
code
Slint
Text {
    // Slint spróbuje użyć "Segoe UI", potem "Roboto", a na końcu dowolnej systemowej czcionki bezszeryfowej.
    font-family: "Segoe UI", "Roboto", sans-serif;
}
sans-serif: ogólna nazwa dla czcionki bezszeryfowej (jak Arial, Helvetica).
serif: ogólna nazwa dla czcionki szeryfowej (jak Times New Roman).
monospace: ogólna nazwa dla czcionki o stałej szerokości znaków (jak Courier New).
Zalety:
Nie zwiększa rozmiaru Twojej aplikacji.
Aplikacja może wyglądać bardziej "natywnie", używając domyślnych czcionek systemowych.
Wady:
Brak gwarancji, że aplikacja będzie wyglądać tak samo na każdym systemie.
Jeśli kluczowa dla projektu czcionka nie jest dostępna, wygląd interfejsu może się "rozjechać".
Metoda 2: Osadzanie czcionek (kompilowanie z kodem)
To podejście gwarantuje, że Twoja aplikacja będzie wyglądać identycznie na każdym urządzeniu, niezależnie od zainstalowanych w systemie czcionek.
Jak to działa?
Plik czcionki (np. .ttf lub .otf) jest dołączany do pliku wykonywalnego Twojej aplikacji. Podczas uruchamiania Slint ładuje czcionkę bezpośrednio z pamięci.
Jak używać?
Proces składa się z dwóch kroków:
Zadeklaruj i osadź czcionkę: W pliku .slint użyj globalnego obiektu Global i funkcji embed(), aby wczytać plik czcionki. Ścieżka do pliku jest względna w stosunku do lokalizacji pliku .slint.
Użyj osadzonej czcionki: Odwołaj się do zadeklarowanej zmiennej w właściwości font-family.
Przykład:
Załóżmy, że struktura Twojego projektu wygląda tak:
code
Code
twoj-projekt/
├── fonts/
│   └── MojSuperFont-Regular.ttf
└── ui/
    └── app.slint
W pliku app.slint osadzasz i używasz czcionki w ten sposób:
code
Slint
// Plik: ui/app.slint

// Krok 1: Osadzenie czcionki w globalnym obiekcie
// Ścieżka jest względna, więc wychodzimy z katalogu 'ui' do głównego folderu projektu
global AppFonts {
    // Deklarujemy właściwość, która przechowuje naszą czcionkę
    property <font> SuperFont: embed("../fonts/MojSuperFont-Regular.ttf");
}

export component App inherits Window {
    // ...
    VerticalLayout {
        // Krok 2: Użycie osadzonej czcionki
        Text {
            text: "Ten tekst używa mojej własnej czcionki!";
            font-size: 24px;
            // Odwołujemy się do niej przez jej zadeklarowaną nazwę i właściwość .family
            font-family: AppFonts.SuperFont.family;
        }

        Text {
            text: "A ten tekst używa czcionki systemowej.";
            font-family: "Verdana", sans-serif;
        }
    }
}
Zalety:
Pełna kontrola nad wyglądem: Aplikacja wygląda identycznie na każdym systemie.
Brak zależności: Użytkownik nie musi niczego instalować. Aplikacja jest samowystarczalna.
Wady:
Większy rozmiar pliku wykonywalnego: Każdy osadzony plik czcionki zwiększa rozmiar aplikacji o kilkadziesiąt do kilkuset kilobajtów.
Podsumowanie
Cecha	Czcionki systemowe	Czcionki osadzone (kompilowane z kodem)
Sposób użycia	font-family: "Nazwa Czcionki";	global G { property <font> F: embed("plik.ttf"); } <br> font-family: G.F.family;
Zależności	Aplikacja zależy od czcionek zainstalowanych na komputerze użytkownika.	Brak zewnętrznych zależności. Czcionka jest częścią aplikacji.
Rozmiar aplikacji	Minimalny, bez zmian.	Zwiększony o rozmiar plików czcionek.
Spójność wyglądu	Niska. Wygląd może się różnić między systemami.	Wysoka. Gwarantowany, identyczny wygląd wszędzie.
Kiedy używać?	Proste aplikacje, prototypy, narzędzia deweloperskie, gdy wygląd nie jest krytyczny.	Aplikacje komercyjne, z mocno zdefiniowanym brandingiem, gdy spójny wygląd jest kluczowy.