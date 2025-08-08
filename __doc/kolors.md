// KROK 1: DEFINICJA KOLORÓW
// Tworzysz globalny obiekt (pojemnik) o nazwie 'MojeKolory'.
export global MojeKolory {
    property <color> tlo_aplikacji: #2c3e50;  // Ciemny niebieski
    property <color> tekst_glowny: #ffffff;   // Biały
    property <color> akcent: #e74c3c;         // Czerwony
}


// KROK 2: UŻYCIE KOLORÓW
// Główny komponent, np. okno.
Window {
    // Używasz zdefiniowanego koloru tła
    background: MojeKolory.tlo_aplikacji;

    Rectangle {
        // Używasz zdefiniowanego koloru akcentu
        background: MojeKolory.akcent;
        width: 100px;
        height: 50px;

        Text {
            text: "Tekst";
            // Używasz zdefiniowanego koloru tekstu
            color: MojeKolory.tekst_glowny;
        }
    }
}