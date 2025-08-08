Lista TODO dla \_\_rustEXR wg. priorytetów
🎯 Priorytet 1: Krytyczne (Użyteczność i Stabilność)
Te zadania są kluczowe, aby biblioteka była użyteczna dla innych programistów i godna zaufania.
✍️ Stworzenie dokumentacji i przykładów użycia:
Zadanie: Dodać komentarze dokumentacyjne (///) do wszystkich publicznych struktur, funkcji i modułów (Image, Layer, read::from_file, write::to_file itd.).
Cel: Umożliwienie użytkownikom zrozumienia API bez potrzeby czytania kodu źródłowego. cargo doc powinno generować kompletną i użyteczną dokumentację.
Kluczowe: Uzupełnić README.md o proste, działające przykłady odczytu i zapisu pliku. To pierwsza rzecz, na którą patrzy potencjalny użytkownik.
🧊 Ustabilizowanie publicznego API dla obrazów scanline:
Zadanie: Zdecydować o ostatecznym kształcie API dla aktualnie wspieranych funkcji. Nazwa gałęzi (refactor) sugeruje, że API może się jeszcze zmieniać. Należy je zamrozić dla wersji 1.0.
Cel: Zapewnienie stabilności. Użytkownicy nie lubią, gdy API zmienia się z każdą aktualizacją.
🐛 Rozszerzenie pokrycia testami i naprawa błędów:
Zadanie: Dodać więcej plików .exr do zestawu testowego, obejmując przypadki brzegowe (obrazy 1x1, różne typy danych kanałów, niestandardowe nazwy kanałów, pliki bez kompresji).
Cel: Zwiększenie niezawodności i wyłapanie ukrytych błędów w logice parsowania i zapisu.
⭐ Priorytet 2: Ważne (Rozszerzenie kluczowych funkcjonalności)
Po ustabilizowaniu podstaw, te zadania znacząco zwiększą możliwości i atrakcyjność biblioteki.
🖼️ Implementacja wsparcia dla obrazów kafelkowych (Tiled Images):
Zadanie: Dodać logikę do odczytu i zapisu obrazów z atrybutem lineOrder ustawionym na TILED. Wymaga to obsługi atrybutów tile_description i innej organizacji danych w pliku.
Cel: Wsparcie dla bardzo dużych rozdzielczości i standardu używanego w wielu profesjonalnych potokach produkcyjnych. To największy brakujący element standardu EXR.
🧩 Uzupełnienie obsługi wszystkich metod kompresji:
Zadanie: Zaimplementować brakujące warianty Compression::NotImplemented (np. PXR24, B44, B44A, DWAA, DWAB).
Cel: Pełna zgodność ze standardem i możliwość odczytu dowolnego pliku EXR napotkanego "na dziko".
🚀 Optymalizacja wydajności przez zrównoleglenie:
Zadanie: Wykorzystać bibliotekę rayon do zrównoleglenia operacji dekompresji i kompresji. Dekompresja poszczególnych linii skanowania lub kafelków to zadanie, które idealnie nadaje się do przetwarzania równoległego.
Cel: Znaczące przyspieszenie operacji na obrazach o wysokiej rozdzielczości, co jest kluczowe w zastosowaniach profesjonalnych.
🛠️ Wprowadzenie wzorca Builder do tworzenia obrazów:
Zadanie: Stworzyć ImageBuilder, który ułatwi programistyczne tworzenie nowych obrazów od zera. Np. Image::builder().with_resolution(1920, 1080).add_rgba_layer("beauty").build().
Cel: Poprawa ergonomii API. Uczynienie biblioteki bardziej przyjazną i "idiomatyczną" w użyciu.
👍 Priorytet 3: Dobre do posiadania (Kompletność i ułatwienia)
Te zadania sprawią, że biblioteka będzie bardziej kompletna i dopracowana.
🌊 Implementacja wsparcia dla Deep Data:
Zadanie: Dodać możliwość odczytu i zapisu "głębokich" pikseli, które przechowują wiele próbek na piksel.
Cel: Wsparcie dla zaawansowanych technik compositingu w VFX. Jest to niszowa, ale bardzo ważna funkcja dla profesjonalistów.
📦 Implementacja wsparcia dla plików wieloczęściowych (Multi-part):
Zadanie: Umożliwić odczyt i zapis plików .exr, które zawierają wiele niezależnych obrazów (części) w jednym kontenerze (standard EXR 2.0).
Cel: Pełna zgodność z nowoczesnym standardem OpenEXR.
👓 Rozbudowa przeglądarki exr_viewer:
Zadanie: Dodać do przeglądarki proste funkcje, takie jak wybór warstwy/kanału do wyświetlenia, kontrola ekspozycji, wyświetlanie metadanych.
Cel: Stworzenie bardziej użytecznego narzędzia do debugowania i demonstracji możliwości biblioteki.
🔄 Zadania ciągłe (Utrzymanie projektu)
Te zadania powinny być wykonywane regularnie w trakcie rozwoju.
Aktualizacja zależności: Regularne uruchamianie cargo update i testowanie z nowymi wersjami bibliotek.
Refaktoryzacja: Utrzymywanie wysokiej jakości kodu, usuwanie długu technicznego, upraszczanie wewnętrznej logiki.
Rozbudowa CI: Dodanie clippy z bardziej rygorystycznymi ustawieniami do GitHub Actions, aby automatycznie wyłapywać potencjalne problemy i antywzorce.
