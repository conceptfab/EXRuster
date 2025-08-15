    1 # Plan Optymalizacji Aplikacji EXRuster
    2
    3 Ten dokument opisuje proponowane zmiany w architekturze aplikacji w celu znacznego przyspieszenia czasu wczytywania folderów z plikami EXR oraz otwierania
      pojedynczych plików. Zmiany te zachowują istniejącą wizualizację postępu, dostosowując ją do nowej, bardziej wydajnej logiki.
    4
    5 ---
    6
    7 ## Optymalizacja 1: Strumieniowe Wczytywanie Miniaturek
    8
    9 **Problem:** Obecna implementacja w `src/thumbnails.rs` jest nieefektywna, ponieważ do wygenerowania pojedynczej miniaturki wczytuje z dysku wszystkie
      warstwy i kanały pliku EXR. Wersja z katalogu `stable` była pod tym względem znacznie szybsza, gdyż korzystała z dedykowanej funkcji strumieniowej.

10
11 **Rozwiązanie:** Zastąpimy logikę generowania miniaturek w `src/thumbnails.rs`, implementując wydajne wczytywanie strumieniowe, podobne do tego z wersji
`stable`. Pozwoli to na odczyt tylko niezbędnych danych (kanały RGBA), minimalizując operacje I/O. Zachowamy przy tym mechanizm `LruCache`, dzięki czemu
ponowne wczytanie katalogu będzie natychmiastowe.
12
13 ### Kroki implementacji
14
15 1. **Otwórz plik:** `src/thumbnails.rs`.
16 2. **Zastąp funkcję `generate_single_exr_thumbnail_work`:** Usuń całą obecną implementację tej funkcji i zastąp ją poniższym, zoptymalizowanym kodem. Nowa
wersja inteligentnie łączy szybkie wczytywanie strumieniowe z bardziej niezawodną ścieżką awaryjną (fallback) dla skomplikowanych plików EXR.
// Wklej ten kod do src/thumbnails.rs, zastępując istniejącą funkcję

      pub fn generate_single_exr_thumbnail_work(
          path: &Path,
          thumb_height: u32,
          exposure: f32,
          gamma: f32,
          tonemap_mode: i32,
      ) -> anyhow::Result<ExrThumbWork> {
          use std::convert::Infallible;
          use exr::math::Vec2;

          let path_buf = path.to_path_buf();

          // Krok 1: Szybkie pobranie metadanych (liczba warstw, macierz kolorów)
          let layers_info = extract_layers_info(&path_buf)
              .with_context(|| format!("Błąd odczytu meta EXR: {}", path.display()))?;
          let color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, "").ok();

          // Krok 2: Próba odczytu strumieniowego (najszybsza ścieżka)
          let dims = Rc::new(RefCell::new((0u32, 0u32, 0u32, 0u32))); // w, h, tw, th
          let out_pixels = Rc::new(RefCell::new(Vec::<u8>::new()));
          let write_count = Rc::new(RefCell::new(0usize));

          let stream_result = {
              let dims_c = dims.clone();
              let out_c1 = out_pixels.clone();
              let write_ctr_c = write_count.clone();

              exr::read_first_rgba_layer_from_file(
                  &path_buf,
                  move |resolution, _| -> Result<(), Infallible> {
                      let width = resolution.width() as u32;
                      let height = resolution.height() as u32;
                      if width == 0 || height == 0 { return Ok(()); }

                      let thumb_h = thumb_height.max(1);
                      let thumb_w = ((width as f32 * thumb_h as f32) / height as f32).max(1.0).round() as u32;

                      *dims_c.borrow_mut() = (width, height, thumb_w, thumb_h);
                      out_c1.borrow_mut().resize((thumb_w as usize) * (thumb_h as usize) * 4, 0u8);
                      Ok(())
                  },
                  move |_, position: Vec2<usize>, (r0, g0, b0, a0): (f32, f32, f32, f32)| {
                      let (width, height, thumb_w, thumb_h) = *dims.borrow();
                      if thumb_w == 0 || thumb_h == 0 { return; }

                      let sx = width as f32 / thumb_w as f32;
                      let sy = height as f32 / thumb_h as f32;

                      let x_out = ((position.x() as f32) / sx).floor() as u32;
                      let y_out = ((position.y() as f32) / sy).floor() as u32;

                      if x_out >= thumb_w || y_out >= thumb_h { return; }

                      let (mut r, mut g, mut b) = (r0, g0, b0);
                      if let Some(mat) = color_matrix_rgb_to_srgb {
                          let v = mat * Vec3::new(r, g, b);
                          r = v.x; g = v.y; b = v.z;
                      }
                      let px = process_pixel(r, g, b, a0, exposure, gamma, tonemap_mode);

                      let out_index = ((y_out as usize)  (thumb_w as usize) + (x_out as usize))  4;
                      let mut out_ref = out_pixels.borrow_mut();
                      if out_index + 3 < out_ref.len() {
                          out_ref[out_index + 0] = px.r;
                          out_ref[out_index + 1] = px.g;
                          out_ref[out_index + 2] = px.b;
                          out_ref[out_index + 3] = 255; // Wymuś pełną nieprzezroczystość
                          *write_ctr_c.borrow_mut() += 1;
                      }
                  },
              )
          };

          // Krok 3: Sprawdzenie, czy odczyt strumieniowy się powiódł i czy zapisał piksele
          let (thumb_w, thumb_h) = { let d = dims.borrow(); (d.2, d.3) };
          let expected_pixels = (thumb_w as usize) * (thumb_h as usize);
          let pixels_written = *write_count.borrow();

          if stream_result.is_ok() && pixels_written > 0 && pixels_written >= expected_pixels / 2 {
              // Sukces! Zwróć wynik ze strumienia
          } else {
              // Krok 4: Fallback do wolniejszej, ale bardziej niezawodnej metody
              let best_layer_name = find_best_layer(&layers_info);
              let (raw_pixels, width, height, _) = load_specific_layer(&path_buf, &best_layer_name, None)?;

              let scale = thumb_height as f32 / height as f32;
              let thumb_w_fb = ((width as f32) * scale).max(1.0).round() as u32;
              let thumb_h_fb = thumb_height.max(1);

              let mut pixels_fb: Vec<u8> = vec![0; (thumb_w_fb as usize) * (thumb_h_fb as usize) * 4];
              let m = color_matrix_rgb_to_srgb;

              pixels_fb.par_chunks_mut(4).enumerate().for_each(|(i, out)| {
                  let x = (i as u32) % thumb_w_fb;
                  let y = (i as u32) / thumb_h_fb;
                  let src_x = ((x as f32 / scale) as u32).min(width.saturating_sub(1));
                  let src_y = ((y as f32 / scale) as u32).min(height.saturating_sub(1));
                  let src_idx = (src_y as usize) * (width as usize) + (src_x as usize);

                  let (mut r, mut g, mut b, a) = raw_pixels[src_idx];
                  if let Some(mat) = m {
                      let v = mat * Vec3::new(r, g, b);
                      r = v.x; g = v.y; b = v.z;
                  }
                  let px = process_pixel(r, g, b, a, exposure, gamma, tonemap_mode);
                  out[0] = px.r; out[1] = px.g; out[2] = px.b; out[3] = px.a;
              });
              *out_pixels.borrow_mut() = pixels_fb;
          }

          let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
          let file_size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

          Ok(ExrThumbWork {
              path: path_buf,
              file_name,
              file_size_bytes,
              width: thumb_w,
              height: thumb_h,
              num_layers: layers_info.len(),
              pixels: out_pixels.borrow().clone(),
          })
      }

    1
    2 ---
    3
    4 ## Optymalizacja 2: Leniwe (lazy) Ładowanie Warstw Obrazu
    5
    6 **Problem:** Aplikacja przy otwieraniu pliku EXR wczytuje od razu wszystkie jego warstwy i kanały do pamięci (`FullExrCache`). To powoduje znaczne
      opóźnienie, zanim użytkownik zobaczy jakikolwiek obraz, szczególnie przy dużych plikach.
    7
    8 **Rozwiązanie:** Zmienimy strategię na "leniwe ładowanie" (lazy loading). Przy otwarciu pliku wczytana zostanie tylko domyślna, najważniejsza warstwa (np.
      "beauty"). Pozostałe warstwy zostaną doczytane z dysku dopiero wtedy, gdy użytkownik kliknie na nie w panelu warstw.
    9

10 ### Kroki implementacji
11
12 #### Etap 1: Ustawienie "lekkiego" wczytywania jako domyślnego
13
14 1. **Otwórz plik:** `src/ui_handlers.rs`.
15 2. **Znajdź funkcję:** `handle_open_exr_from_path`.
16 3. **Uprość logikę:** Wewnątrz funkcji znajduje się rozgałęzienie `if use_light ... else ...`. Usuń całe to rozgałęzienie, pozostawiając **jedynie ciało
bloku `if use_light`**. To sprawi, że szybsza, "lekka" metoda wczytywania będzie używana dla wszystkich plików, a nie tylko tych powyżej 700MB.
17
18 **Kod do usunięcia:**
// Usuń ten fragment z handle_open_exr_from_path
let file_size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
let force_light = std::env::var("EXRUSTER_LIGHT_OPEN").ok().as_deref() == Some("1");
let use_light = force_light || file_size_bytes > 700 1024 1024; // >700MB ⇒ light

      // ... oraz cały blok 'else' z logiką dla 'FULL ścieżka'

1 Po tej zmianie kod w `rayon::spawn` powinien zaczynać się bezpośrednio od `let t_start = Instant::now(); let light_res = ...`.
2
3 #### Etap 2: Implementacja doładowywania warstw na żądanie
4
5 1. **Otwórz plik:** `src/image_cache.rs`.
6 2. **Dodaj nową funkcję pomocniczą:** Ta funkcja będzie odpowiedzialna za wczytanie pojedynczej warstwy z dysku i dodanie jej do istniejącego w pamięci
`FullExrCache`. Wklej ją wewnątrz bloku `impl ImageCache`.
// Wklej wewnątrz impl ImageCache w src/image_cache.rs
fn ensure_layer_is_loaded(&mut self, path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
let layer_exists_in_cache = self.full_cache.layers.iter().any(|l| l.name == layer_name);

          if layer_exists_in_cache {
              return Ok(()); // Warstwa już jest w cache, nic nie rób
          }

          if let Some(p) = progress {
              p.start_indeterminate(Some(&format!("Lazy loading layer: {}...", layer_name)));
          }

          // Wczytaj brakującą warstwę z dysku
          let layer_channels = load_all_channels_for_layer(path, layer_name, progress)?;

          // Stwórz nową FullLayer
          let new_full_layer = crate::full_exr_cache::FullLayer {
              name: layer_channels.layer_name,
              width: layer_channels.width,
              height: layer_channels.height,
              channel_names: layer_channels.channel_names,
              channel_data: layer_channels.channel_data.to_vec(), // Konwersja z Arc<[f32]>
          };

          // Dodaj ją do istniejącego cache
          // Potrzebujemy mutowalnego dostępu, więc musimy obejść Arc
          if let Some(mut_cache) = Arc::get_mut(&mut self.full_cache) {
               mut_cache.layers.push(new_full_layer);
          } else {
              // Jeśli Arc jest współdzielony, musimy go sklonować i zastąpić
              let mut new_cache_data = (*self.full_cache).clone();
              new_cache_data.layers.push(new_full_layer);
              self.full_cache = Arc::new(new_cache_data);
          }

          if let Some(p) = progress {
              p.finish(Some("Layer loaded"));
          }

          Ok(())
      }

1
2 3. **Zmodyfikuj `ImageCache::load_layer`:** Zaktualizuj tę funkcję, aby korzystała z nowej metody `ensure_layer_is_loaded` przed próbą załadowania warstwy.
// W src/image_cache.rs, zmodyfikuj funkcję `load_layer`

      pub fn load_layer(&mut self, path: &PathBuf, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<()> {
          // Krok 1: Upewnij się, że dane warstwy są w pamięci (doładuj jeśli trzeba)
          self.ensure_layer_is_loaded(path, layer_name, progress)?;

          // Krok 2: Kontynuuj jak wcześniej, wczytując dane z full_cache
          let layer_channels = load_all_channels_for_layer_from_full(&self.full_cache, layer_name, progress)?;

          self.width = layer_channels.width;
          self.height = layer_channels.height;
          self.current_layer_name = layer_channels.layer_name.clone();
          self.raw_pixels = compose_composite_from_channels(&layer_channels);
          self.current_layer_channels = Some(layer_channels);
          self.color_matrix_rgb_to_srgb = compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name).ok();
          self.mip_levels = build_mip_chain(&self.raw_pixels, self.width, self.height, 4);

          Ok(())
      }

1
2 #### Etap 3: Wizualizacja postępu
3
4 Logika paska postępu (`UiProgress`) będzie działać automatycznie.
5 _ Przy pierwszym otwarciu pliku pasek postępu pokaże szybkie wczytanie domyślnej warstwy.
6 _ Gdy użytkownik kliknie na inną, jeszcze niezaładowaną warstwę, funkcja `handle_layer_tree_click` w `ui_handlers.rs` wywoła `cache.load_layer`. Dzięki
naszym zmianom, `load_layer` uruchomi `UiProgress` na czas doładowywania danych z dysku, dając użytkownikowi czytelną informację zwrotną, że aplikacja
pracuje. Nie są tu potrzebne żadne dodatkowe zmiany w `ui_handlers.rs`.
7
8 Po wprowadzeniu tych zmian aplikacja powinna być znacznie bardziej responsywna przy pierwszym kontakcie z plikami, zachowując jednocześnie wysoką wydajność
podczas dalszej pracy.
