Opis znajduje się pod swoją miniaturką.
Wielkość (wysokość) miniaturek może być różna.
Odstęp między opisem jednego elementu a miniaturką następnego elementu musi być stały.
Odstęp między miniaturką a jej własnym opisem też musi być stały.
To jest klasyczny problem layoutu i w Slint rozwiązuje się go bardzo elegancko przez zagnieżdżanie i odpowiednie użycie właściwości spacing.
Schemat rozwiązania
Kluczem jest zagnieżdżenie. Każdy element galerii (obrazek + jego opis) to jedna, spójna grupa. Główny layout układa te grupy w stałych odstępach.
Oto prawidłowa struktura hierarchii:
code
Code
ScrollView
└── VerticalLayout (Główny kontener listy)
    │   spacing: 24px;  <-- TO JEST STAŁY ODSTĘP MIĘDZY ELEMENTAMI GALERII
    │
    ├── GalleryItem (Komponent dla elementu 1)
    │   └── VerticalLayout (Wewnętrzny layout)
    │       │   spacing: 8px;  <-- TO JEST STAŁY ODSTĘP OBRAZEK -> OPIS
    │       │
    │       ├── Image (Miniaturka 1)
    │       └── VerticalLayout (Blok opisu 1)
    │           ├── Text (Nazwa pliku)
    │           └── Text (Rozmiar, warstwy)
    │
    ├── GalleryItem (Komponent dla elementu 2)
    │   └── VerticalLayout (Wewnętrzny layout)
    │       │   spacing: 8px;
    │       │
    │       ├── Image (Miniaturka 2)
    │       └── VerticalLayout (Blok opisu 2)
    │           └── ...
    │
    └── ...
Dlaczego to działa?
Główny VerticalLayout widzi tylko komponenty GalleryItem. Nie obchodzi go, jak wysokie są w środku. On po prostu bierze pierwszy GalleryItem, stawia go na górze, zostawia 24px pustej przestrzeni (spacing) i stawia pod spodem drugi GalleryItem.
Dzięki temu, niezależnie od tego, czy Miniaturka 1 ma 200px wysokości, a Miniaturka 2 ma 400px, odstęp między końcem Opisu 1 a początkiem Miniaturki 2 będzie zawsze taki sam.
Przykładowy kod Slint (poprawiona wersja)
Ten kod implementuje powyższy schemat. Jest gotowy do wklejenia i uruchomienia.
code
Slint
import { ScrollView } from "std-widgets.slint";

// Struktura danych, bez zmian
struct ImageInfo {
    source: image,
    name: string,
    size: string,
    layers: string,
}

// =======================================================================
// KOMPONENT DLA JEDNEGO ELEMENTU GALERII (OBRAZEK + OPIS POD SPODEM)
// =======================================================================
component GalleryItem inherits VerticalLayout {
    in property <ImageInfo> item_data;

    // STAŁY ODSTĘP MIĘDZY OBRAZKIEM A JEGO OPISEM
    spacing: 8px;

    // 1. Obrazek
    Image {
        source: item_data.source;
        width: parent.width; // Rozciągnij na całą szerokość kontenera
    }

    // 2. Kontener na opisy (dla porządku i ewentualnego tła)
    VerticalLayout {
        spacing: 2px; // Mały odstęp między liniami tekstu
        
        Text {
            text: item_data.name;
            color: #ffffff;
            font-weight: 700;
            wrap: word-wrap; // Zawijaj tekst, jeśli jest za długi
        }
        Text {
            text: "\{item_data.size} / \{item_data.layers} layers";
            color: #dddddd;
            font-size: 13px;
        }
    }
}


// =======================================================================
// GŁÓWNE OKNO APLIKACJI
// =======================================================================
export component App inherits Window {
    width: 500px;
    height: 800px;
    background: #282c34; // Ciemne tło

    // Model danych, bez zmian
    in-out property <[ImageInfo]> images_data: [
        { source: @image-url("https://i.imgur.com/8aP16dC.jpeg"), name: "39_20_NA_Legia_FNR5_0001.EXR", size: "12.8 MB", layers: "1" },
        { source: @image-url("https://i.imgur.com/pTbcQeG.jpeg"), name: "39_20_NA_Legia_FNR5_0002.EXR", size: "12.5 MB", layers: "1" },
        { source: @image-url("https://i.imgur.com/kFLj7a4.jpeg"), name: "Render_Interior_With_A_Much_Longer_Name_That_Should_Wrap.PNG", size: "8.2 MB", layers: "1" },
    ];

    ScrollView {
        VerticalLayout {
            padding: 20px;
            
            // =======================================================================
            // KLUCZOWY ELEMENT: STAŁY ODSTĘP MIĘDZY CAŁYMI ELEMENTAMI GALERII
            // To jest odstęp, który będzie zawsze taki sam, niezależnie od wysokości
            // obrazków.
            // =======================================================================
            spacing: 24px;

            // Pętla tworząca elementy galerii z naszego modelu danych
            for item in images_data : GalleryItem {
                item_data: item;
            }
        }
    }
}
To rozwiązanie jest solidne, skalowalne i robi dokładnie to, czego potrzebujesz.