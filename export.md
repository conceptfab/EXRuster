Krok 1: Konfiguracja projektu i zależności
Na początek utwórz nowy projekt w Cargo i dodaj potrzebne zależności do pliku Cargo.toml.
code
Sh
cargo new exr_to_tiff_converter
cd exr_to_tiff_converter
Otwórz plik Cargo.toml i dodaj następujące linie w sekcji [dependencies]:
code
Toml
[dependencies]
exr = "1.9.1"         # Do odczytu plików EXR
tiff = "0.9.1"        # Do zapisu plików TIFF
anyhow = "1.0"        # Dla wygodnej obsługi błędów
Krok 2: Logika programu
Logika konwersji będzie wyglądać następująco:
Wczytaj argumenty: Pobierz ścieżkę do wejściowego pliku EXR i wyjściowego pliku TIFF z linii poleceń.
Otwórz plik EXR: Użyj biblioteki exr, aby otworzyć plik i odczytać jego metadane (w tym listę warstw i ich kanałów).
Utwórz plik TIFF: Przygotuj enkoder TIFF, który będzie zapisywał dane do pliku wyjściowego.
Iteruj po warstwach EXR: Dla każdej warstwy znalezionej w pliku EXR:
a. Wczytaj dane pikseli tylko dla tej konkretnej warstwy.
b. Sprawdź, jakie kanały są dostępne (np. R, G, B, A, czy może tylko jeden kanał).
c. Przygotuj bufor z danymi pikseli w formacie akceptowanym przez bibliotekę tiff (np. przeplatane [R, G, B, A, R, G, B, A, ...]).
d. Zapisz ten bufor jako nową stronę (obraz) w pliku TIFF.
e. Ważne: Zapisz nazwę warstwy EXR w metadanych (tagach) tej strony TIFF, aby można było je później zidentyfikować.
Zakończ: Zamknij plik TIFF.
Krok 3: Przykładowy kod
Oto kompletny kod, który możesz umieścić w src/main.rs. Kod jest dobrze skomentowany, aby wyjaśnić każdy krok.
code
Rust
use std::fs::File;
use std::path::Path;
use exr::prelude::*;
use tiff::encoder::{TiffEncoder, TiffValue, colortype};
use tiff::tags::Tag;
use anyhow::{Context, Result};

fn main() -> Result<()> {
    // 1. Pobierz argumenty z linii poleceń
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        println!("Użycie: cargo run --release -- <plik_wejsciowy.exr> <plik_wyjsciowy.tiff>");
        return Ok(());
    }
    let input_path = Path::new(&args[1]);
    let output_path = Path::new(&args[2]);

    println!("Odczytuję plik EXR: {:?}", input_path);

    // 2. Odczytaj metadane z pliku EXR, aby poznać jego strukturę (warstwy, kanały, wymiary)
    let meta = read_first_meta_from_file(input_path)
        .context("Nie udało się odczytać metadanych z pliku EXR")?;

    let resolution = meta.resolution();
    let width = resolution.width() as u32;
    let height = resolution.height() as u32;

    println!("Rozdzielczość: {}x{}", width, height);
    println!("Znalezione warstwy:");
    for layer in &meta.layers {
        println!("  - {}", layer.name);
    }

    // 3. Przygotuj enkoder TIFF do zapisu pliku wyjściowego
    let mut output_file = File::create(output_path)
        .context("Nie udało się utworzyć pliku wyjściowego TIFF")?;
    let mut tiff_encoder = TiffEncoder::new(&mut output_file)
        .context("Nie udało się stworzyć enkodera TIFF")?;

    // 4. Iteruj po każdej warstwie z pliku EXR
    for layer_meta in &meta.layers {
        println!("\nPrzetwarzam warstwę: '{}'...", layer_meta.name);

        // a. Wczytaj dane pikseli tylko dla bieżącej warstwy
        //    Aby oszczędzać pamięć, wczytujemy tylko te kanały, które nas interesują (R, G, B, A).
        let read_options = ReadOptions::specific_layers_and_channels(
            &[&layer_meta.name],
            // Szukamy kanałów kończących się na .R, .G, .B, .A
            |layer, channel_name| ["R", "G", "B", "A"].contains(&channel_name.as_str()),
        );

        let image = read_image_from_file(input_path, read_options)
            .context(format!("Błąd podczas czytania warstwy '{}'", layer_meta.name))?;
        
        // Pobierz dane warstwy z wczytanego obrazu
        let layer_data = image.layers.get(0)
            .context("Oczekiwano przynajmniej jednej warstwy w odczytanym obrazie")?;

        // b. Znajdź kanały R, G, B, A
        let r_channel = layer_data.channels.get_named("R");
        let g_channel = layer_data.channels.get_named("G");
        let b_channel = layer_data.channels.get_named("B");
        let a_channel = layer_data.channels.get_named("A");

        // c. Przygotuj dane pikseli do zapisu w TIFF
        // EXR najczęściej używa 32-bitowych floatów. Zachowamy tę precyzję w TIFF.
        let pixel_count = (width * height) as usize;

        // Sprawdzamy, które kanały są dostępne i tworzymy odpowiedni bufor
        match (r_channel, g_channel, b_channel, a_channel) {
            // Przypadek RGBA
            (Some(r), Some(g), Some(b), Some(a)) => {
                println!("  > Konwertuję jako RGBA (32-bit float)");
                let mut pixel_buffer = vec![0.0f32; pixel_count * 4];
                let (r_f32, g_f32, b_f32, a_f32) = (r.sample_data.as_f32()?, g.sample_data.as_f32()?, b.sample_data.as_f32()?, a.sample_data.as_f32()?);
                
                for i in 0..pixel_count {
                    pixel_buffer[i * 4 + 0] = r_f32[i];
                    pixel_buffer[i * 4 + 1] = g_f32[i];
                    pixel_buffer[i * 4 + 2] = b_f32[i];
                    pixel_buffer[i * 4 + 3] = a_f32[i];
                }

                write_tiff_page(&mut tiff_encoder, width, height, &layer_meta.name, colortype::RGBAf32, &pixel_buffer)?;
            },
            // Przypadek RGB
            (Some(r), Some(g), Some(b), None) => {
                println!("  > Konwertuję jako RGB (32-bit float)");
                let mut pixel_buffer = vec![0.0f32; pixel_count * 3];
                let (r_f32, g_f32, b_f32) = (r.sample_data.as_f32()?, g.sample_data.as_f32()?, b.sample_data.as_f32()?);

                for i in 0..pixel_count {
                    pixel_buffer[i * 3 + 0] = r_f32[i];
                    pixel_buffer[i * 3 + 1] = g_f32[i];
                    pixel_buffer[i * 3 + 2] = b_f32[i];
                }

                write_tiff_page(&mut tiff_encoder, width, height, &layer_meta.name, colortype::RGBf32, &pixel_buffer)?;
            },
            // Przypadek jednokanałowy (np. Z-depth, maska)
            (Some(channel), None, None, None) | (None, Some(channel), None, None) | (None, None, Some(channel), None) => {
                 println!("  > Konwertuję jako Grayscale (32-bit float)");
                 let pixel_buffer = channel.sample_data.as_f32()?;
                 write_tiff_page(&mut tiff_encoder, width, height, &layer_meta.name, colortype::Grayf32, pixel_buffer)?;
            },
            _ => {
                println!("  > Pomijam warstwę '{}', ponieważ nie ma standardowego zestawu kanałów (RGB, RGBA, lub pojedynczy)", layer_meta.name);
            }
        }
    }

    println!("\nZakończono! Plik zapisany w: {:?}", output_path);

    Ok(())
}

/// Funkcja pomocnicza do zapisu pojedynczej strony w pliku TIFF
fn write_tiff_page<C>(
    encoder: &mut TiffEncoder<File>,
    width: u32,
    height: u32,
    layer_name: &str,
    color_type: C,
    data: &[C::Inner],
) -> Result<()>
where
    C: tiff::encoder::ColorType,
{
    let mut image_writer = encoder.new_image(color_type, width, height)
        .context("Nie udało się stworzyć nowego obrazu w TIFF")?;
    
    // d. Zapisz nazwę warstwy jako tag "ImageDescription"
    image_writer.write_tag(Tag::ImageDescription, TiffValue::Ascii(layer_name.to_string()))?;

    // Zapisz dane pikseli
    image_writer.write_image(data)
        .context(format!("Nie udało się zapisać danych obrazu dla warstwy '{}'", layer_name))?;

    Ok(())
}
Krok 4: Kompilacja i uruchomienie
Upewnij się, że masz jakiś plik .exr z warstwami. Jeśli nie, możesz znaleźć przykładowe pliki w Internecie (np. na stronie openexr.com).
Skompiluj i uruchom program, podając ścieżki do plików. Użyj --release dla lepszej wydajności.
code
Sh
# Przykład użycia
cargo run --release -- assets/multilayer.exr output/result.tiff
Program powinien wyświetlić listę znalezionych warstw, a następnie przetworzyć każdą z nich i zapisać do pliku TIFF.
Co robi ten kod i dlaczego?
Oszczędność Pamięci: Zamiast wczytywać cały, potencjalnie ogromny plik EXR do pamięci, najpierw czytamy tylko metadane (read_first_meta_from_file). Następnie, w pętli, wczytujemy dane pikseli tylko dla jednej warstwy na raz (ReadOptions::specific_layers_and_channels).
Zachowanie Precyzji: EXR przechowuje dane w formacie zmiennoprzecinkowym (float), co jest kluczowe dla szerokiego zakresu dynamiki (HDR). Kod konwertuje dane do f32 w TIFF (RGBAf32, RGBf32, Grayf32), dzięki czemu nie tracisz tej informacji.
Identyfikacja Warstw: Zapisanie nazwy warstwy EXR w tagu ImageDescription w pliku TIFF jest kluczowe. Programy takie jak Adobe Photoshop czy GIMP odczytają ten tag i wyświetlą go jako nazwę warstwy, co ułatwia pracę z plikiem wynikowym.
Obsługa Różnych Układów Kanałów: Kod potrafi obsłużyć warstwy RGBA, RGB oraz jednokanałowe (traktowane jako skala szarości). Jeśli warstwa ma niestandardowy układ (np. tylko kanał R i B), zostanie pominięta z odpowiednim komunikatem.
Solidna Obsługa Błędów: Użycie anyhow::Result i metody .context() sprawia, że komunikaty o błędach są czytelne i wskazują, w którym miejscu operacja zawiodła (np. "Nie udało się odczytać pliku EXR").
Możliwe modyfikacje i ulepszenia
Konwersja do 8-bit/16-bit: Jeśli plik docelowy ma być mniejszy i nie wymaga precyzji float, możesz przekonwertować dane. Na przykład, aby zapisać jako 8-bitowe RGBA, musisz "przyciąć" wartości float do zakresu [0.0, 1.0], a następnie przeskalować je do [0, 255].
code
Rust
// Przykład konwersji f32 na u8
let pixel_f32 = 0.75;
let pixel_u8 = (pixel_f32.clamp(0.0, 1.0) * 255.0).round() as u8;
Obsługa niestandardowych kanałów: Można rozbudować logikę, aby obsługiwała inne nazwy kanałów (np. diffuse.R, specular.G) lub zapisywała je jako osobne obrazy w skali szarości.
Wsparcie dla kompresji TIFF: Biblioteka tiff pozwala na ustawienie kompresji (np. LZW, Deflate), co może znacznie zmniejszyć rozmiar pliku wyjściowego. Można to ustawić podczas tworzenia enkodera.