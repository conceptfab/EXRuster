Zmiany w pliku appwindow.slint
W prawej kolumnie (Column 3) - sekcja VerticalBox:
Zmiana 1: Ujednolicenie struktury dla wszystkich kontrolek
python// Column 3
Rectangle {
    width: (root.width - _non_column_width) * _n3;
    visible: show-right-panel;
    clip: true;
    background: Kolory.panel_tlo;
    border-color: Kolory.linia_podzialu;
    border-width: 1px;
    
    // Prawa kolumna: funkcje Exposure na górze
    VerticalBox {
        padding: 1px;
        spacing: 1px;
        alignment: start;
        property <length> label_left_margin: 12px;    // Stały odstęp dla etykiet
        property <length> controls_margin: 6px;       // Stały margines dla kontrolek

        // Exposure Slider
        VerticalBox {
            width: parent.width;
            spacing: 2px;
            // Etykieta
            Text {
                text: "Exposure:";
                color: Kolory.tekst;
                font-size: 10px;
                font-family: "Geist";
                font-weight: 700;
                horizontal-alignment: left;
                x: label_left_margin;
            }
            // Kontrolka na środku
            HorizontalBox {
                width: parent.width;
                padding-left: controls_margin;
                padding-right: controls_margin;
                ParameterSlider {
                    value: root.exposure-value;
                    min-value: -5.0;
                    max-value: 5.0;
                    slider-width: parent.width - controls_margin * 2;
                    value-changed(new-value) => {
                        root.exposure-value = new-value;
                        root.exposure-changed(new-value);
                    }
                }
            }
        }
        
        // Gamma Slider
        VerticalBox {
            width: parent.width;
            spacing: 2px;
            // Etykieta
            Text {
                text: "Gamma:";
                color: Kolory.tekst;
                font-size: 10px;
                font-family: "Geist";
                font-weight: 700;
                horizontal-alignment: left;
                x: label_left_margin;
            }
            // Kontrolka na środku
            HorizontalBox {
                width: parent.width;
                padding-left: controls_margin;
                padding-right: controls_margin;
                ParameterSlider {
                    value: root.gamma-value;
                    min-value: 0.5;
                    max-value: 4.5;
                    slider-width: parent.width - controls_margin * 2;
                    value-changed(new-value) => {
                        root.gamma-value = new-value;
                        root.gamma-changed(new-value);
                    }
                }
            }
        }

        // Tonemap selector
        VerticalBox {
            width: parent.width;
            spacing: 2px;
            // Etykieta
            Text {
                text: "Tonemap:";
                color: Kolory.tekst;
                font-size: 10px;
                font-family: "Geist";
                font-weight: 700;
                horizontal-alignment: left;
                x: label_left_margin;
            }
            // Przyciski na środku
            HorizontalBox {
                width: parent.width;
                height: 25px;
                padding-left: controls_margin;
                padding-right: controls_margin;
                spacing: 6px;
                // ACES
                PrzyciskAkcji {
                    text: "ACES";
                    height: 25px;
                    width: (parent.width - controls_margin * 2 - parent.spacing * 2) / 3;
                    highlighted: root.tonemap-mode == 0;
                    clicked => { root.tonemap-mode = 0; root.tonemap-mode-changed(0); }
                }
                // Reinhard
                PrzyciskAkcji {
                    text: "Reinhard";
                    height: 25px;
                    width: (parent.width - controls_margin * 2 - parent.spacing * 2) / 3;
                    highlighted: root.tonemap-mode == 1;
                    clicked => { root.tonemap-mode = 1; root.tonemap-mode-changed(1); }
                }
                // Linear
                PrzyciskAkcji {
                    text: "Linear";
                    height: 25px;
                    width: (parent.width - controls_margin * 2 - parent.spacing * 2) / 3;
                    highlighted: root.tonemap-mode == 2;
                    clicked => { root.tonemap-mode = 2; root.tonemap-mode-changed(2); }
                }
            }
        }
        
        // Reset Button
        HorizontalBox {
            width: parent.width;
            height: 25px;
            padding-left: controls_margin;
            padding-right: controls_margin;
            PrzyciskAkcji {
                text: "Reset";
                height: 25px;
                width: parent.width - controls_margin * 2;
                clicked => {
                    exposure-value = 0.0;
                    gamma-value = 2.2;
                    root.tonemap-mode = 0;
                    root.tonemap-mode-changed(0);
                    exposure-changed(exposure-value);
                    gamma-changed(gamma-value);
                }
            }
        }
        
        // Spacer
        Rectangle {
            height: 10px;
        }

        // Export section
        VerticalBox {
            width: parent.width;
            spacing: 2px;
            // Etykieta Export
            Text {
                text: "Export";
                color: Kolory.tekst;
                font-size: 10px;
                font-family: "Geist";
                font-weight: 700;
                horizontal-alignment: left;
                x: label_left_margin;
            }

            // Convert Button
            HorizontalBox {
                width: parent.width;
                height: 25px;
                padding-left: controls_margin;
                padding-right: controls_margin;
                PrzyciskAkcji {
                    text: "Convert (TIFF)";
                    height: 25px;
                    width: parent.width - controls_margin * 2;
                    clicked => { export-convert(); }
                }
            }

            // Export Beauty Button
            HorizontalBox {
                width: parent.width;
                height: 25px;
                padding-left: controls_margin;
                padding-right: controls_margin;
                PrzyciskAkcji {
                    text: "Export Beauty (PNG16)";
                    height: 25px;
                    width: parent.width - controls_margin * 2;
                    clicked => { export-beauty(); }
                }
            }

            // Export Channels Button
            HorizontalBox {
                width: parent.width;
                height: 25px;
                padding-left: controls_margin;
                padding-right: controls_margin;
                PrzyciskAkcji {
                    text: "Export Channels (PNG16)";
                    height: 25px;
                    width: parent.width - controls_margin * 2;
                    clicked => { export-channels(); }
                }
            }
        }
    }

    // Nakładka maskująca prawą krawędź
    Rectangle {
        x: parent.width - 1px;
        width: 1px;
        height: parent.height;
        background: Kolory.panel_tlo;
    }
}
Zmiana 2: Aktualizacja komponentu ParameterSlider
W pliku ParameterSlider.slint trzeba usunąć wewnętrzną etykietę, ponieważ teraz etykiety są zarządzane zewnętrznie:
pythonexport component ParameterSlider inherits Rectangle {
    in property <float> value: 0.0;
    in property <float> min-value: 0.0;
    in property <float> max-value: 1.0;
    in property <length> slider-width: 100px;
    
    callback value-changed(float);
    
    width: slider-width;
    height: 20px;
    
    // Tylko suwak, bez etykiety
    Rectangle {
        width: parent.width;
        height: 12px;
        y: (parent.height - self.height) / 2;
        background: Kolory.suwak_tlo;
        border-radius: 6px;
        
        // Slider handle
        Rectangle {
            property <float> progress: (value - min-value) / max(0.001, max-value - min-value);
            x: progress * (parent.width - self.width);
            y: (parent.height - self.height) / 2;
            width: 16px;
            height: 16px;
            background: Kolory.suwak_tor;
            border-radius: 8px;
            
            TouchArea {
                width: parent.width;
                height: parent.height;
                moved => {
                    if (self.pressed) {
                        value = min-value + (max-value - min-value) * max(0.0, min(1.0, self.mouse-x / (parent.parent.width - parent.width)));
                        value-changed(value);
                    }
                }
            }
        }
    }
}
Podsumowanie zmian:

Ujednolicone odstępy: Wszystkie etykiety mają stały odstęp label_left_margin: 12px od lewej krawędzi
Wycentrowane kontrolki: Wszystkie przyciski i suwaki są wycentrowane z marginesem controls_margin: 6px z każdej strony
Spójna struktura: Każda sekcja (Exposure, Gamma, Tonemap, Export) używa tej samej struktury VerticalBox z etykietą na górze i kontrolkami poniżej
Usunięte duplikowanie: Etykiety są teraz zarządzane na poziomie głównego layoutu, nie w komponentach

Te zmiany sprawią, że interfejs będzie bardziej uporządkowany i wizualnie spójny.