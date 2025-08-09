Główna zasada
Definiujesz właściwość stanu (State Property): Tworzysz w swoim komponencie UI właściwość (najczęściej typu bool), która przechowuje informację o tym, czy panel jest aktualnie widoczny, czy schowany. Np. property <bool> panel-widoczny: false;.
Tworzysz element wyzwalający (Trigger): Dodajesz element, który będzie zmieniał ten stan, np. Button. W jego akcji clicked (lub toggled dla CheckBox) zmieniasz wartość właściwości stanu (np. z false na true i odwrotnie).
Wiążesz wygląd panelu ze stanem: Właściwości wizualne panelu (takie jak jego height, width lub visible) są "związane" z właściwością stanu. Oznacza to, że gdy stan się zmienia, wygląd panelu automatycznie się aktualizuje.
(Opcjonalnie, ale zalecane) Dodajesz animację: Aby przejście było płynne, a nie natychmiastowe, dodajesz animację do zmienianej właściwości (np. height). Slint czyni to niezwykle prostym.
Praktyczny przykład
Poniżej znajduje się kompletny, minimalny przykład, który pokazuje, jak stworzyć zwijany panel za pomocą przycisku.
Załóżmy, że tworzymy plik app.slint.
code
Slint
// Plik: app.slint
import { Button, VerticalLayout } from "std-widgets.slint";

export component MainWindow inherits Window {
    width: 300px;
    height: 350px;
    title: "Chowany Panel w Slint";

    // Krok 1: Właściwość przechowująca stan panelu (widoczny/ukryty)
    in-out property <bool> panel-widoczny: false;

    VerticalLayout {
        spacing: 10px;
        padding: 10px;

        // Krok 2: Przycisk, który zmienia stan
        Button {
            text: root.panel-widoczny ? "Ukryj Panel" : "Pokaż Panel";
            clicked => {
                root.panel-widoczny = !root.panel-widoczny;
            }
        }

        // Krok 3: Panel, którego wygląd jest związany ze stanem
        Rectangle {
            id: chowany-panel;
            background: #3498db;
            border-radius: 5px;

            // Ustawiamy wysokość na 0, gdy jest schowany, i na 150px, gdy widoczny
            height: root.panel-widoczny ? 150px : 0px;

            // WAŻNE: `clip` zapobiega "wylewaniu się" zawartości poza prostokąt,
            // gdy jego wysokość wynosi 0.
            clip: true;

            // Krok 4: Płynna animacja zmiany wysokości
            animate height {
                duration: 300ms;
                easing: ease-in-out;
            }

            // Zawartość panelu
            Text {
                text: "Witaj! To jest zawartość\nchowanego panelu.";
                color: white;
                horizontal-alignment: center;
                vertical-alignment: center;
            }
        }

        // Reszta interfejsu
        Text {
            text: "To jest stała część interfejsu, \nktóra jest zawsze widoczna.";
        }
    }
}
Wyjaśnienie kodu .slint:
in-out property <bool> panel-widoczny: false;
Definiujemy właściwość panel-widoczny, która jest dostępna wewnątrz (in) i na zewnątrz (out) komponentu. Domyślnie ustawiamy ją na false (panel jest schowany).
Button { ... }
Tekst przycisku dynamicznie zmienia się w zależności od stanu (? : to operator trójargumentowy).
clicked => { root.panel-widoczny = !root.panel-widoczny; } to kluczowa linia. Po kliknięciu odwracamy wartość logiczną stanu. Slint automatycznie wykryje tę zmianę.
Rectangle { ... }
height: root.panel-widoczny ? 150px : 0px; – to jest serce mechanizmu. Jeśli panel-widoczny jest true, wysokość to 150px. Jeśli false, wysokość to 0px.
animate height { ... } – ta deklaracja sprawia, że każda zmiana właściwości height nie będzie natychmiastowa, ale potrwa 300ms z płynnym przyspieszaniem i zwalnianiem (ease-in-out).
clip: true; – to bardzo ważna właściwość. Bez niej, nawet gdy wysokość prostokąta (Rectangle) wynosiłaby 0, jego zawartość (tekst) mogłaby być wciąż renderowana. clip "przytnie" całą zawartość do granic elementu.
Kod w Rust do uruchomienia UI
Aby to uruchomić, potrzebujesz prostego pliku main.rs.
code
Rust
// Plik: main.rs
slint::include_modules!(); // Wczytuje definicję z pliku .slint

fn main() -> Result<(), slint::PlatformError> {
    let main_window = MainWindow::new()?;

    main_window.run()
}
I oczywiście Cargo.toml:
code
Toml
# Plik: Cargo.toml
[package]
name = "slint-collapsible-panel"
version = "0.1.0"
edition = "2021"

[dependencies]
slint = "1.5"

[build-dependencies]
slint-build = "1.5"
Oraz prosty build.rs:
code
Rust
// Plik: build.rs
fn main() {
    slint_build::compile("ui/app.slint").unwrap();
}
(Pamiętaj, aby umieścić plik app.slint w folderze ui).
Podsumowanie
Zasada jest prosta i niezwykle potężna:
Stan napędza UI.
Zamiast ręcznie zmieniać wysokość i widoczność w kodzie imperatywnym (np. w Rust), deklarujesz w Slint, jak UI ma wyglądać w zależności od stanu. Resztą zajmuje się sam Slint, włącznie z wydajnym przerysowywaniem i animacjami.