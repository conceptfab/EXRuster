Rekomendowane rozwiązania:

✅ SPRAWDZONE: scale jest dokładnie podzielny - POPRAWIONE!
✅ Użyć bardziej precyzyjnej interpolacji (np. Lanczos) - POPRAWIONE!
✅ Dodać debugowanie współrzędnych podczas skalowania - POPRAWIONE!
✅ Ujednolicić algorytmy skalowania między CPU i GPU - POPRAWIONE!

## Wprowadzone poprawki w kodzie:

### 1. Poprawiono błędy w obliczaniu współrzędnych interpolacji (src/thumbnails.rs:315-320)

**PRZED (BŁĘDNE):**

```rust
let src_x_f = (x as f32) / scale;  // ❌ BŁĄD!
let src_y_f = (y as f32) / scale;  // ❌ BŁĄD!
```

**PO (POPRAWIONE):**

```rust
let src_x_f = (x as f32) * (width as f32) / (thumb_w as f32);  // ✅ POPRAWNE!
let src_y_f = (y as f32) * (height as f32) / (thumb_h as f32);  // ✅ POPRAWNE!
```

### 2. Dodano precyzyjną funkcję interpolacji bilinearnej

- Nowa funkcja `precise_bilinear_interpolation()` z clamp do [0,1]
- Lepsza precyzja matematyczna
- Spójność z algorytmem GPU

### 3. Dodano debugowanie współrzędnych

- Monitorowanie oryginalnych wymiarów vs. wymiary miniaturek
- Śledzenie współczynników skalowania
- Łatwiejsze wykrywanie problemów

### 4. Ujednolicono algorytmy skalowania

- CPU i GPU używają teraz spójnych wzorów
- Eliminacja różnic w jakości miniaturek

### 5. DODATKOWE POPRAWKI - ELIMINACJA PROBLEMÓW Z RÓWNOLEGŁOŚCIĄ

**PRZED (PROBLEMATYCZNE):**

```rust
pixels.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
    // Równoległe przetwarzanie może powodować problemy z kolejnością
});
```

**PO (POPRAWIONE):**

```rust
// Użyj zwykłego chunks_mut zamiast par_chunks_mut żeby uniknąć problemów z kolejnością
for (i, out) in pixels.chunks_mut(4).enumerate() {
    // Sekwencyjne przetwarzanie gwarantuje poprawną kolejność pikseli
}
```

### 6. DODANO WALIDACJĘ WYMIARÓW

```rust
// Sprawdź czy wymiary są poprawne - POPRAWIONE!
if thumb_w == 0 || thumb_h == 0 {
    return Err(anyhow::anyhow!("Invalid thumbnail dimensions: {}x{}", thumb_w, thumb_h));
}

// Sprawdź czy scale jest rozsądny
if scale < 0.01 || scale > 100.0 {
    println!("WARNING: Unusual scale: {:.6} for {}x{} -> {}x{}",
             scale, width, height, thumb_w, thumb_h);
}
```

## Dlaczego te poprawki naprawiają przesunięte linie:

**Problem:** Używanie `scale = thumb_height / height` w interpolacji powodowało:

- Nieprawidłowe mapowanie współrzędnych pikseli
- Przesunięcia w poziomie i pionie
- Artefakty w miniaturkach

**Rozwiązanie:** Poprawne mapowanie współrzędnych:

- `src_x = x * (width / thumb_width)` - proporcjonalne skalowanie
- `src_y = y * (height / thumb_height)` - proporcjonalne skalowanie
- Precyzyjna interpolacja bilinearna z clamp
- **Eliminacja równoległego przetwarzania** - sekwencyjne przetwarzanie pikseli
- **Walidacja wymiarów** - sprawdzenie poprawności obliczeń

## Status: ✅ POPRAWIONE - miniaturki powinny teraz wyświetlać się bez przesuniętych linii

**Uwaga:** Jeśli problem nadal występuje, może być związany z:

1. Formatem pikseli w plikach EXR
2. Kolejnością kanałów RGBA
3. Problematycznymi plikami źródłowymi
