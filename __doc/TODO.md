1. Wyswietlanie pełnej nazwy po najechaniu na nazwę
2. filtr wyświetlania w panelu


Chce dodac integracje z windows. Jak przesunę plik na exr na ikonę programu to program ma ten plik otworzyć. Dadatkowo ma sprawdzić czy w folderze skad jest plik exr nie ma więcej plików - jeśli są to ma je wczytać i pokazac panel z miniaturkami. Ten program ma być domyślnym do otwierania EXR w windows - i musi to obsługiwać



Dodaj w prawym panelu etykietę Export oraz 3 przyciski jeden pod drugim. Po dodaniu przycisków zbuduj odpowiednie funkcje i podłącz je do UI:
Oto przyciski:
Convert: Zapisuje danych plik exr jako tiff - zachowując strukturę i głebie kolorów i wszystkie inne istotne informacje
Export Beauty - eksportuje tylko warstwę Beauty jako 16 bit png
Export Channels - eksportuje wszystkie kanały jako osobne pliki png



strzałki do wczytywania!