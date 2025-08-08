Oczywiście. Dodanie ikony do aplikacji Rust dla systemu Windows jest standardowym procesem, który polega na osadzeniu jej w zasobach pliku wykonywalnego (.exe). Robi się to za pomocą skryptu budowania (build.rs).
Jaki format pliku jest obsługiwany?
Kluczowym i jedynym natywnie wspieranym formatem dla ikony aplikacji w systemie Windows jest .ico.
Plik .ico to specjalny format kontenera, który może zawierać wiele obrazów o różnych rozmiarach i głębiach kolorów w jednym pliku. Dzięki temu system Windows może wybrać najlepszą wersję ikony do wyświetlenia w zależności od kontekstu (np. mała ikona na pasku zadań, duża na pulpicie, bardzo duża w widoku "Szczegóły").
Zalecane rozmiary do umieszczenia w pliku .ico:
16x16
32x32
48x48
256x256
Większość nowoczesnych narzędzi do tworzenia ikon (lub konwerterów online) tworzy plik .ico zawierający wszystkie te rozmiary z jednego obrazu źródłowego (np. PNG o wymiarach 256x256 lub większym).
Jak dodać ikonę do aplikacji - Krok po Kroku
Użyjemy do tego popularnej i bardzo prostej w użyciu skrzynki (crate) o nazwie winres.
Krok 1: Przygotuj plik ikony
Stwórz lub pobierz swoją ikonę w formacie .png lub innym.
Użyj konwertera online (np. icoconverter.com) lub programu graficznego (np. GIMP, IrfanView), aby przekonwertować swój obraz na plik .ico.
Zapisz plik, np. pod nazwą icon.ico, i umieść go w głównym katalogu swojego projektu Rust.
Struktura projektu powinna wyglądać mniej więcej tak:
code
Code
twoj-projekt/
├── src/
│   └── main.rs
├── Cargo.toml
└── icon.ico        <-- Twoja ikona
Krok 2: Zaktualizuj Cargo.toml
Musimy dodać winres jako zależność budowania (build-dependency). Oznacza to, że crate będzie używany tylko podczas kompilacji, a nie dołączany do finalnej aplikacji.
Otwórz plik Cargo.toml i dodaj sekcję [build-dependencies]:
code
Toml
[package]
name = "moja-aplikacja-z-ikona"
version = "0.1.0"
edition = "2021"

[dependencies]
# Tutaj są normalne zależności twojej aplikacji
# np. slint = "1.2"

# Dodaj tę sekcję
[build-dependencies]
winres = "0.1"
Krok 3: Stwórz skrypt budowania (build.rs)
W głównym katalogu projektu (obok Cargo.toml) stwórz nowy plik o nazwie build.rs.
Wklej do niego poniższy kod:
code
Rust
// Plik: build.rs

// Sprawdzamy, czy kompilujemy na platformę Windows.
// To zapobiega błędom podczas kompilacji na Linuksie czy macOS.
#[cfg(target_os = "windows")]
fn main() {
    // Używamy skrzynki winres, aby utworzyć nowy obiekt zasobów Windows.
    let mut res = winres::WindowsResource::new();

    // Ustawiamy ikonę aplikacji. Ścieżka jest względna do głównego katalogu projektu.
    res.set_icon("icon.ico");

    // Kompilujemy zasoby. To wygeneruje plik .rc i skompiluje go,
    // a następnie poinstruuje Cargo, aby dołączył go do finalnego .exe.
    if let Err(e) = res.compile() {
        eprintln!("Błąd kompilacji zasobów Windows: {}", e);
        // Zakończ proces budowania z błędem
        std::process::exit(1);
    }
}

// Jeśli nie kompilujemy na Windows, skrypt nie powinien nic robić.
#[cfg(not(target_os = "windows"))]
fn main() {}
Wyjaśnienie kodu:
#[cfg(target_os = "windows")]: Ten atrybut sprawia, że kod wewnątrz funkcji main zostanie wykonany tylko wtedy, gdy kompilujesz swoją aplikację na systemie Windows.
winres::WindowsResource::new(): Tworzy obiekt, który będzie zarządzał zasobami Windows.
res.set_icon("icon.ico"): To jest kluczowa linia. Mówi ona winres, aby użył pliku icon.ico jako głównej ikony aplikacji.
res.compile(): Uruchamia proces kompilacji zasobów. W tle winres używa kompilatora zasobów (rc.exe), który jest częścią narzędzi budowania Windows (Build Tools for Visual Studio).
Krok 4: Skompiluj aplikację
Teraz wystarczy uruchomić standardowe polecenie budowania:
code
Bash
cargo build
lub dla wersji zoptymalizowanej:
code
Bash
cargo build --release
Cargo automatycznie wykryje plik build.rs, uruchomi go, a winres zajmie się resztą.
Krok 5: Sprawdź rezultat
Przejdź do katalogu target/debug lub target/release. Znajdź swój plik .exe – powinien teraz mieć przypisaną ikonę, widoczną w Eksploratorze Plików Windows!
Podsumowanie
Pytanie	Odpowiedź
Jaki format pliku?	.ico (zawierający wiele rozmiarów, np. 16, 32, 48, 256 px)
Jak to zrobić?	Użyć skryptu budowania (build.rs) ze skrzynką winres.
Kluczowe kroki	1. Przygotuj icon.ico. <br> 2. Dodaj winres do [build-dependencies] w Cargo.toml. <br> 3. Stwórz build.rs z kodem ustawiającym ikonę. <br> 4. Uruchom cargo build.
Dodatkowa wskazówka: Skrzynka winres pozwala również na ustawienie innych metadanych pliku .exe, takich jak wersja, nazwa produktu, opis pliku itp. Można to zrobić za pomocą metod takich jak set_version_info, set_manifest i innych.