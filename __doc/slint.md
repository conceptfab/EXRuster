Schemat Pozycjonowania w Slint
1. Pozycjonowanie Poziome (Horyzontalne)
Używamy HorizontalLayout, który układa elementy od lewej do prawej.
Główne wartości dla alignment:
start: Przykleja do lewej krawędzi (domyślne).
end: Przykleja do prawej krawędzi.
center: Wyśrodkowuje w dostępnej przestrzeni.
stretch: Rozciąga element, aby wypełnił całą dostępną przestrzeń.
Schemat wizualny:
code
Code
// Używamy HorizontalLayout, aby ułożyć elementy obok siebie.
HorizontalLayout {
    // Domyślnie element jest przyklejony do lewej (alignment: start)
    // [ Tekst |                         ]
    Text { text: "Do lewej"; }

    // Wyśrodkowanie w poziomie
    // [          | Tekst |          ]
    Text { text: "Środek"; alignment: center; }

    // Przyklejenie do prawej
    // [                         | Tekst ]
    Text { text: "Do prawej"; alignment: end; }
}
Sztuczka ze "spacerem": Jak umieścić jeden element po lewej, a drugi po prawej?
Najprostszym sposobem jest wstawienie pomiędzy nimi "pustego" elementu, który rozciągnie się i zajmie całą wolną przestrzeń.
code
Code
HorizontalLayout {
    // [ Lewy tekst | <---- pustka ----> | Prawy tekst ]

    Text { text: "Po lewej"; }

    // "Spacer" - pusty prostokąt, który rozepcha elementy
    Rectangle { }

    Text { text: "Po prawej"; }
}
W HorizontalLayout pusty element bez zdefiniowanej szerokości zachowuje się jak sprężyna i automatycznie wypełnia dostępną przestrzegnie, rozpychając sąsiadów.
2. Pozycjonowanie Pionowe (Wertykalne)
Używamy VerticalLayout, który układa elementy od góry do dołu. Zasada działania alignment jest identyczna.
Główne wartości dla alignment:
start: Przykleja do górnej krawędzi (domyślne).
end: Przykleja do dolnej krawędzi.
center: Wyśrodkowuje w dostępnej przestrzeni.
stretch: Rozciąga element, aby wypełnił całą dostępną przestrzeń w pionie.
Schemat wizualny:
code
Code
// Używamy VerticalLayout, aby ułożyć elementy jeden pod drugim.
VerticalLayout {
    // [ Tekst     ]  (na górze)
    // [           ]
    // [           ]
    Text { text: "Do góry"; } // Domyślnie alignment: start

    // [           ]
    // [ Tekst     ]  (na środku)
    // [           ]
    Text { text: "Środek"; alignment: center; }

    // [           ]
    // [           ]
    // [ Tekst     ]  (na dole)
    Text { text: "Do dołu"; alignment: end; }
}
3. Łączenie pozycjonowania - Przyklejanie do rogów i środka
Aby pozycjonować element jednocześnie w pionie i w poziomie (np. w prawym dolnym rogu), najczęściej umieszczamy go w kontenerze (np. Rectangle lub GridLayout) i używamy właściwości horizontal-alignment oraz vertical-alignment.
Schemat dla pojedynczego elementu w kontenerze:
code
Code
Rectangle { // Ten prostokąt jest naszym kontenerem
    background: #eee;
    width: 200px;
    height: 100px;

    // Przykłady pozycjonowania tekstu wewnątrz tego prostokąta

    // 1. Lewy górny róg (domyślnie)
    Text { text: "L-G"; }

    // 2. Prawy górny róg
    Text { text: "P-G"; horizontal-alignment: end; }

    // 3. Środek
    Text { text: "ŚRODEK"; horizontal-alignment: center; vertical-alignment: center; }

    // 4. Lewy dolny róg
    Text { text: "L-D"; vertical-alignment: end; }

    // 5. Prawy dolny róg
    Text { text: "P-D"; horizontal-alignment: end; vertical-alignment: end; }
}
Kompletny, działający przykład
Poniższy kod tworzy okno, które wizualnie demonstruje wszystkie omówione koncepty. Możesz go zapisać jako plik przyklad.slint i uruchomić.
code
Slint
// Zapisz jako przyklad.slint i uruchom w Live-Preview

export component App inherits Window {
    title: "Schemat Pozycjonowania w Slint";
    width: 600px;
    height: 450px;

    // Główny layout, układający przykłady jeden pod drugim
    VerticalLayout {
        spacing: 10px;
        padding: 10px;

        // --- Przykład 1: Pozycjonowanie poziome ---
        Rectangle {
            background: #d0d0ff;
            height: 40px;
            HorizontalLayout {
                padding: 5px;
                spacing: 10px;
                Text { text: "Lewo (start)"; }
                Rectangle {} // "Spacer" rozpychający elementy
                Text { text: "Prawo (end)"; }
            }
        }

        // --- Przykład 2: Pozycjonowanie pionowe ---
        Rectangle {
            background: #d0ffd0;
            height: 120px;
            VerticalLayout {
                padding: 5px;
                spacing: 5px;
                Text { text: "Góra (start)"; }
                Text { text: "Środek (center)"; alignment: center; }
                Text { text: "Dół (end)"; alignment: end; }
            }
        }

        // --- Przykład 3: Pozycjonowanie w kontenerze (rogi i środek) ---
        Rectangle {
            background: #ffd0d0;
            // Ten prostokąt zajmie resztę dostępnej przestrzeni
            Text {
                text: "Prawy dolny róg";
                horizontal-alignment: end;
                vertical-alignment: end;
                padding: 10px;
                font-size: 16px;
                color: blue;
            }
            Text {
                text: "Środek";
                horizontal-alignment: center;
                vertical-alignment: center;
                font-size: 24px;
                font-weight: 700;
            }
        }
    }
}
Podsumowanie:
Chcesz ułożyć elementy obok siebie? Użyj HorizontalLayout.
Chcesz ułożyć elementy jeden pod drugim? Użyj VerticalLayout.
Chcesz przykleić element do lewej/prawej/góry/dołu? Ustaw alignment na tym elemencie wewnątrz Layoutu.
Chcesz przykleić coś do lewej i prawej jednocześnie? Użyj "sztuczki ze spacerem" (Rectangle {}).
Chcesz pozycjonować coś w rogu lub na środku kontenera? Umieść element bezpośrednio w kontenerze (np. Rectangle) i użyj na nim właściwości horizontal-alignment i vertical-alignment.