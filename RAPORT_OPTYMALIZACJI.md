# Raport optymalizacji kodu `EXRuster`

Poniższy raport przedstawia zidentyfikowane obszary do optymalizacji w kodzie aplikacji w celu maksymalnego przyspieszenia jej działania, z zachowaniem zasady unikania nadmiernej inżynierii (over-engineering). Sugerowane zmiany inspirowane są standardowymi technikami stosowanymi w wysokowydajnych aplikacjach graficznych, w tym w oprogramowaniu pisanym w C++.

## Etap 1: Optymalizacja operacji I/O i parsowania EXR

Największym zidentyfikowanym wąskim gardłem jest wielokrotne odczytywanie i parsowanie tego samego pliku EXR z dysku podczas różnych operacji. Każda zmiana warstwy czy eksport powoduje ponowne, kosztowne operacje I/O.

1.  **Problem:** Funkcje takie jak `ImageCache::new`, `ImageCache::load_layer`, `handle_export_convert` i `handle_export_channels` niezależnie wywołują funkcje z biblioteki `exr` (np. `read_all_flat_layers_from_file`), które za każdym razem czytają plik od nowa.
2.  **Rozwiązanie:** Wprowadzić centralny, jednorazowy mechanizm cache'owania dla całego wczytanego pliku EXR. Po otwarciu pliku przez użytkownika, cała jego zawartość (wszystkie warstwy i piksele) powinna zostać wczytana do pamięci RAM i przechowywana w dedykowanej strukturze.
3.  **Zadania dla AI:**
    *   W module `ui_handlers.rs` lub `main.rs` zdefiniuj nowy, globalnie dostępny cache, np. `type FullExrFileCache = Arc<Mutex<Option<exr::image::AnyImage>>>`.
    *   Zmodyfikuj `handle_open_exr_from_path` w `ui_handlers.rs`: po wybraniu pliku, wczytaj go w całości za pomocą `exr::read_all_flat_layers_from_file` i umieść wynik w nowym, globalnym cache'u.
    *   Przebuduj `ImageCache` oraz funkcje `load_layer`, `load_channel` i `handle_export_*` tak, aby zamiast czytać plik z dysku, pobierały dane (piksele, metadane warstw) z obiektu `AnyImage` przechowywanego w globalnym cache'u. Spowoduje to, że zmiana warstwy lub eksport będą operacjami wykonywanymi wyłącznie w pamięci, co drastycznie przyspieszy działanie.

## Etap 2: Przyspieszenie przetwarzania obrazu i generowania miniatur

Przetwarzanie pikseli i tworzenie miniatur odbywa się równolegle, co jest bardzo dobrym rozwiązaniem. Można je jednak dodatkowo zoptymalizować.

1.  **Problem:** Pętle `par_iter` w `image_cache.rs` i `thumbnails.rs` przetwarzają piksele pojedynczo. Nowoczesne procesory oferują instrukcje SIMD (Single Instruction, Multiple Data), które potrafią przetwarzać wiele danych (np. 4 lub 8 pikseli) w jednym cyklu zegara.
2.  **Rozwiązanie:** Zastosować jawne operacje SIMD do przetwarzania bloków pikseli.
3.  **Zadania dla AI:**
    *   W plikach `image_cache.rs` i `image_processing.rs` zmodyfikuj pętle przetwarzające piksele (np. w `process_to_image`). Zamiast `par_iter().for_each()`, użyj `par_chunks_mut()`, aby operować na fragmentach bufora.
    *   Wewnątrz pętli, dla każdego fragmentu, użyj typów z modułu `std::simd` (np. `f32x4`) do jednoczesnego wykonania operacji matematycznych (mnożenie przez ekspozycję, tone mapping, korekcja gamma) na 4 pikselach naraz. Wymaga to refaktoryzacji funkcji `tone_map_and_gamma`.

4.  **Problem:** Funkcja `generate_single_exr_thumbnail_work` w `thumbnails.rs` posiada ścieżkę rezerwową (fallback), która wczytuje warstwę w pełnej rozdzielczości, aby ją potem przeskalować w dół do rozmiaru miniatury. Jest to bardzo nieefektywne dla dużych obrazów.
5.  **Rozwiązanie:** Wykorzystać fakt, że pliki EXR mogą zawierać pre-generowane obrazy o niższej rozdzielczości (mip-mapy).
6.  **Zadania dla AI:**
    *   W `generate_single_exr_thumbnail_work`, przed wykonaniem kosztownej ścieżki rezerwowej, sprawdź, czy wczytany plik EXR posiada mip-mapy dla "najlepszej" warstwy.
    *   Jeśli mip-mapy są dostępne, wczytaj najmniejszy poziom, który jest większy lub równy docelowemu rozmiarowi miniatury. Biblioteka `exr` udostępnia funkcjonalność do odczytu konkretnych poziomów mip-map, co pozwoli uniknąć wczytywania i skalowania obrazu o pełnej rozdzielczości.

## Etap 3: Usprawnienia w zarządzaniu pamięcią

Sposób przechowywania danych kanałów można zoptymalizować, aby zmniejszyć liczbę alokacji i poprawić lokalność danych w pamięci.

1.  **Problem:** Struktura `LayerChannels` w `image_cache.rs` używa `HashMap<String, Vec<f32>>`. Przechowywanie każdego kanału w osobnym wektorze prowadzi do fragmentacji pamięci i może być mniej wydajne przy dostępie do danych piksela (skakanie po różnych obszarach pamięci dla kanałów R, G i B).
2.  **Rozwiązanie:** Przechowywać wszystkie dane pikseli dla warstwy w jednym, ciągłym buforze pamięci.
3.  **Zadania dla AI:**
    *   Zmodyfikuj logikę wczytywania warstwy (`load_all_channels_for_layer`). Zamiast tworzyć mapę wektorów, od razu buduj finalny, przeplatany (interleaved) bufor `Vec<(f32, f32, f32, f32)>` dla kompozytu RGBA.
    *   Jeśli potrzebny jest dostęp do pojedynczych kanałów, można je wyodrębnić z tego przeplatanego bufora lub przechowywać wszystkie kanały w jednym dużym `Vec<f32>` i operować na jego fragmentach (`slice`). To drugie podejście eliminuje narzut związany z `HashMap` i wieloma osobnymi alokacjami `Vec`.

## Podsumowanie

Kluczową i najbardziej wpływową zmianą jest wprowadzenie globalnego cache'a na cały plik EXR (Etap 1). Już samo to powinno przynieść ogromny wzrost responsywności aplikacji. Pozostałe etapy, zwłaszcza wykorzystanie SIMD, stanowią dalsze, bardziej zaawansowane kroki w kierunku maksymalizacji wydajności, które warto wdrożyć po zrealizowaniu pierwszego etapu.
