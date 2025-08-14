💡 Konkretne rozwiązania:
1. Dodanie GPU path do miniaturek
2. GPU compute shader dla miniaturek
Wykorzystanie istniejącego shadera z image_cache.rs
Batch processing wszystkich pikseli miniaturek
Hardware texture sampling dla skalowania
3. Hybrydowe podejście
GPU dla dużych plików (>100MB)
CPU dla małych plików (<100MB)
Automatyczne przełączanie w zależności od dostępności GPU
�� Oczekiwane przyspieszenie:
Małe pliki (1-10MB): 2-5x szybsze
Średnie pliki (10-100MB): 5-10x szybsze
Duże pliki (100MB+): 10-20x szybsze
Czy chcesz, żebym zaimplementował GPU acceleration dla miniaturek? To może znacząco przyspieszyć proces generowania miniaturek, szczególnie dla dużych plików EXR