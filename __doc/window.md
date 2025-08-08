1. Domyślne zachowanie (z belką systemową)
Jeśli po prostu stworzysz okno w Slint, domyślnie będzie ono używać natywnej ramki i belki systemu operacyjnego (Windows, macOS, Linux).
code
Slint
// main.slint
export component App inherits Window {
    Text {
        text: "To okno ma standardową belkę Windowsa.";
    }
}
Zalety:
Prostota: Nie musisz nic robić, to działa "samo z siebie".
Spójność: Aplikacja wygląda jak inne natywne programy w danym systemie.
Funkcjonalność: Przesuwanie okna, zmiana rozmiaru, minimalizacja – wszystko działa tak, jak użytkownik jest do tego przyzwyczajony.
2. Okno bez belki systemowej (Frameless Window)
Możesz całkowicie usunąć ramkę i belkę systemową. Daje Ci to "czyste płótno", na którym możesz narysować własny interfejs, włączając w to własną belkę tytułową. Robi się to za pomocą właściwości window-frame.
code
Slint
// main.slint
export component App inherits Window {
    // Ta właściwość usuwa standardową ramkę i belkę okna
    window-frame: "none";
    
    // Musimy sami stworzyć interfejs do zarządzania oknem
    VerticalLayout {
        // 1. Nasza własna belka tytułowa
        Rectangle {
            height: 30px;
            background: #333;

            HorizontalLayout {
                padding: 5px;
                spacing: 10px;
                
                // Obszar do przesuwania okna
                TouchArea {
                    // Ważne: to pole pozwala przesuwać okno myszką
                    moved => { root.window-position = root.window-position + self.mouse-x - self.pressed-mouse-x; }
                }
                
                // Nasz własny przycisk zamykania
                Button {
                    text: "X";
                    clicked => { root.hide(); } // root.hide() zamyka okno
                }
            }
        }
        
        // 2. Reszta aplikacji
        Text {
            text: "To jest okno bez belki systemowej.\nPrzesuwaj je, łapiąc za górny, ciemny pasek.";
        }
    }
}
Zalety:
Pełna kontrola nad wyglądem: Możesz stworzyć unikalny design, idealnie dopasowany do Twojej aplikacji (jak np. w Spotify, Discord czy VS Code).
Niestandardowe funkcje: Możesz dodać do belki własne przyciski, logo, menu czy inne elementy.
Wady:
Więcej pracy: Musisz samodzielnie zaimplementować logikę przesuwania okna, zamykania, minimalizacji itd.
Niespójność: Aplikacja może wyglądać i zachowywać się inaczej niż reszta programów w systemie, co może być mylące dla niektórych użytkowników.
Podsumowanie
Belka systemowa (domyślnie)	Własna belka (window-frame: "none")
Kiedy używać?	Gdy chcesz szybko stworzyć aplikację, która wygląda "normalnie" i natywnie.	Gdy wygląd i unikalny branding są kluczowe, a Ty jesteś gotów na dodatkową pracę.
Wygląd	Standardowy dla Windows/macOS/Linux.	Dowolny, zdefiniowany przez Ciebie.
Wysiłek	Minimalny.	Znaczny (trzeba samemu oprogramować zachowanie okna).
Więc odpowiadając bezpośrednio na Twoje pytanie: okno w Slint nie musi mieć belki górnej z Windows. Możesz ją łatwo wyłączyć i stworzyć własną.