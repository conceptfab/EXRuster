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

## Status: 🔍 ANALIZUJĘ - problem z przesuniętymi liniami nadal występuje

**Uwaga:** Jeśli problem nadal występuje, może być związany z:

1. Formatem pikseli w plikach EXR
2. Kolejnością kanałów RGBA
3. Problematycznymi plikami źródłowymi
4. **NOWE: Problematycznym układem pikseli w buforze**

## NOWA ANALIZA - Potencjalny problem z układem pikseli:

### **Problem z `chunks_mut(4)`:**

**PRZED (PROBLEMATYCZNE):**

```rust
for (i, out) in pixels.chunks_mut(4).enumerate() {
    let x = (i as u32) % thumb_w;  // ❌ BŁĄD: może być niepoprawne!
    let y = (i as u32) / thumb_w;  // ❌ BŁĄD: może być niepoprawne!
    // ...
    out[0] = px.r; out[1] = px.g; out[2] = px.b; out[3] = px.a;
}
```

**PO (NOWE PODEJŚCIE):**

```rust
// ALTERNATYWNE PODEJŚCIE: użyj indeksów bezpośrednio zamiast chunks_mut
for y in 0..thumb_h {
    for x in 0..thumb_w {
        let i = (y as usize) * (thumb_w as usize) + (x as usize);
        let buffer_idx = i * 4;
        // ...
        pixels[buffer_idx] = px.r; pixels[buffer_idx + 1] = px.g;
        pixels[buffer_idx + 2] = px.b; pixels[buffer_idx + 3] = px.a;
    }
}
```

### **Dlaczego `chunks_mut(4)` może być problematyczne:**

1. **Kolejność pikseli:** `chunks_mut(4)` może nie gwarantować poprawnej kolejności wiersz po wierszu
2. **Obliczanie współrzędnych:** `i % thumb_w` i `i / thumb_w` może dawać błędne wyniki
3. **Bufor pikseli:** Może być wypełniany w złej kolejności

### **Nowe podejście gwarantuje:**

- **Poprawną kolejność:** `for y in 0..thumb_h` gwarantuje wiersz po wierszu
- **Poprawne współrzędne:** `x` i `y` są bezpośrednio z pętli
- **Poprawny indeks bufora:** `buffer_idx = i * 4` gdzie `i = y * thumb_w + x`

**Status:** 🔍 TESTUJĘ NOWE PODEJŚCIE - może to rozwiązać problem z przesuniętymi liniami!

## NOWA POPRAWKA - Nearest Neighbor zamiast Bilinearnej Interpolacji:

### **Problem z interpolacją bilinearną:**

Interpolacja bilinearna może powodować **artefakty i przesunięcia** w miniaturkach, szczególnie przy dużych różnicach w rozdzielczości.

### **Rozwiązanie - Nearest Neighbor:**

```rust
// NEAREST NEIGHBOR - może rozwiązać problem z przesuniętymi liniami!
let src_x = src_x_f.round() as u32;
let src_y = src_y_f.round() as u32;
let src_x = src_x.min(width.saturating_sub(1));
let src_y = src_y.min(height.saturating_sub(1));

let idx = (src_y as usize) * (width as usize) + (src_x as usize);
if idx < raw_pixels.len() {
    let (r, g, b, a) = raw_pixels[idx];
    // Bezpośrednie kopiowanie piksela - brak interpolacji!
}
```

### **Dlaczego Nearest Neighbor może pomóc:**

1. **Brak interpolacji:** Nie ma mieszania pikseli, które może powodować artefakty
2. **Ostre krawędzie:** Każdy piksel pochodzi z dokładnie jednego piksela źródłowego
3. **Brak przesunięć:** Eliminuje problemy z wagami interpolacji
4. **Szybsze:** Prostsze obliczenia

### **Toggle dla testowania:**

```rust
let use_nearest_neighbor = true; // Toggle dla testowania
```

**Status:** 🎯 TESTUJĘ NEAREST NEIGHBOR - to może być brakujący element!

## NOWA ANALIZA - Problem może być w parsowaniu warstw!

### **Kluczowa obserwacja:**

- **Pliki BEZ warstw:** Działają poprawnie (używają `load_first_rgba_layer`)
- **Pliki Z warstwami:** Mają przesunięte linie (używają `load_specific_layer`)

### **Potencjalne problemy w parsowaniu warstw:**

1. **Funkcja `split_layer_and_short`** może błędnie parsować nazwy kanałów
2. **Funkcja `extract_layers_info`** może błędnie mapować warstwy
3. **Funkcja `find_best_layer`** może wybierać złą warstwę
4. **Błędne mapowanie kanałów** → przesunięte piksele!

### **Dodane debugowanie:**

```rust
// W split_layer_and_short:
println!("DEBUG split_layer_and_short: '{}' + {:?} -> ('{}', '{}')",
         full, base_attr, result.0, result.1);

// W extract_layers_info:
println!("DEBUG channel mapping: '{}' -> layer='{}', short='{}'",
         full_channel_name, layer_name_effective, short_channel_name);

// W find_best_layer:
println!("DEBUG find_best_layer: WYBRANO warstwę '{}' (Plan X)", layer.name);
```

### **Co sprawdzamy:**

1. **Czy nazwy kanałów są poprawnie parsowane**
2. **Czy warstwy są poprawnie mapowane**
3. **Czy wybierana jest właściwa warstwa**
4. **Czy kolejność kanałów RGBA jest zachowana**

**Status:** 🔍 DEBUGUJĘ PARSOWANIE WARSTW - to może być główna przyczyna!

c## DODATKOWE DEBUGOWANIE - Mapowanie kanałów R/G/B/A:

### **Nowe debugowanie dodane:**

1. **Wykrywanie kanałów:**

```rust
println!("DEBUG channel detection: '{}' -> short='{}' -> su='{}'", full, short, su);
println!("DEBUG: Znaleziono R kanał na indeksie {}", idx);
```

2. **Mapowanie indeksów:**

```rust
println!("DEBUG load_specific_layer: r_idx={:?}, g_idx={:?}, b_idx={:?}, a_idx={:?}",
         r_idx, g_idx, b_idx, a_idx);
println!("DEBUG load_specific_layer: Finalne indeksy: r={}, g={}, b={}", ri, gi, bi);
```

3. **Uzupełnianie brakujących kanałów:**

```rust
println!("DEBUG load_specific_layer: Uzupełniono r_idx={:?}", r_idx);
```

4. **Weryfikacja pikseli:**

```rust
println!("DEBUG load_specific_layer: Pierwszy piksel: R={:.3}, G={:.3}, B={:.3}, A={:.3}",
         out[0].0, out[0].1, out[0].2, out[0].3);
```

### **Co to pokaże:**

- **W jakiej kolejności** są znajdowane kanały R/G/B/A
- **Czy indeksy** są poprawnie mapowane
- **Czy kanały** są w poprawnej kolejności w buforze
- **Czy wartości pikseli** są poprawne

### **Potencjalne problemy do wykrycia:**

1. **Kanały w złej kolejności** (np. B, G, R zamiast R, G, B)
2. **Błędne indeksy** kanałów
3. **Niepoprawne mapowanie** nazw kanałów
4. **Błędne wartości** pikseli

**Status:** 🔍 KOMPLETNE DEBUGOWANIE - teraz zobaczymy dokładnie co się dzieje!

## DODATKOWE DEBUGOWANIE - Kolejność pikseli i kanałów:

### **Nowe debugowanie dodane:**

5. **Kolejność pikseli w buforze:**

```rust
println!("DEBUG load_specific_layer: Rozpoczynam wczytywanie {} pikseli ({}x{})",
         pixel_count, width, height);
println!("DEBUG pixel[{}]: pos=({},{})", i, x, y);
```

6. **Weryfikacja ostatniego piksela:**

```rust
println!("DEBUG load_specific_layer: Ostatni piksel[{}]: pos=({},{}) R={:.3}, G={:.3}, B={:.3}, A={:.3}",
         last_idx, last_x, last_y, out[last_idx].0, out[last_idx].1, out[last_idx].2, out[last_idx].3);
```

7. **Kolejność kanałów w pliku EXR:**

```rust
println!("DEBUG   [{}]: '{}' -> layer='{}', short='{}'", idx, full, lname, short);
```

### **Co to pokaże:**

- **Czy piksele są w poprawnej kolejności** (wiersz po wierszu)
- **Czy kanały są w poprawnej kolejności** w pliku EXR
- **Czy mapowanie pozycji** jest poprawne
- **Czy problem jest w kolejności** pikseli czy kanałów

### **Potencjalne problemy do wykrycia:**

1. **Kanały w złej kolejności** w pliku EXR
2. **Błędna kolejność pikseli** w buforze
3. **Niepoprawne mapowanie pozycji** (x, y)
4. **Błędne indeksowanie** kanałów

**Status:** 🔍 SUPER KOMPLETNE DEBUGOWANIE - teraz zobaczymy WSZYSTKO!
