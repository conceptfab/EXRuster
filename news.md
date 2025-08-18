# EXRuster - Nowe funkcje FAZY 3

### FAZA 3 – Zaawansowane funkcje GPU (nowości)

#### Zaawansowany tone mapping

- Dodane tryby:
  - Filmic (tryb 3)
  - Hable (tryb 4)
  - Local Adaptation (tryb 5, z parametrem `local_adaptation_radius`)
- Zmiany:
  - `src/shaders/image_processing.wgsl`: nowe funkcje `filmic_tonemap`, `hable_tonemap`, lokalna adaptacja (lokalne uśrednianie 3×3), obsługa nowych trybów.
  - `src/image_cache.rs`: rozszerzenie `ParamsStd140` o `local_adaptation_radius` i przekazywanie do shadera.
  - UI: nowe przyciski trybów w `ui/appwindow.slint`, zaktualizowany callback w `src/main.rs` (status pokazuje nazwy trybów).

#### Filtry w czasie rzeczywistym (GPU)

- Nowe compute shadery:
  - `src/shaders/blur.wgsl`: Gaussian, Box, Motion (parametry: typ, promień, siła, kierunek).
  - `src/shaders/sharpen.wgsl`: Unsharp Mask, High-Pass, Edge Enhancement.
  - `src/shaders/histogram.wgsl`: histogram RGB/Luminance (atomiki).
- Integracja:
  - `src/gpu_context.rs`: cache shaderów/pipeline'ów i układów BGL dla blur/sharpen/histogram + metody get\_\*.
  - UI: przyciski „GPU Blur", „GPU Sharpen", „GPU Histogram" w `ui/appwindow.slint`.
  - `src/main.rs`: callbacki `on_apply_gpu_blur`, `on_apply_gpu_sharpen`, `on_compute_gpu_histogram`.
  - `src/ui_handlers.rs`: funkcje `apply_gpu_blur`, `apply_gpu_sharpen`, `compute_gpu_histogram` (logują i ustawiają status – implementacja GPU I/O w toku).

#### GPU‑accelerated Export (szkielet)

- Struktury i przepływ:
  - `ExportTask`, `ExportFormat` (PNG16, TIFF16, JPEG, EXR).
  - `handle_async_export` (wątek+timer), `perform_export` z fallbackiem CPU, stub `perform_gpu_export`.
  - Handlery: `handle_export_convert_gpu`, `handle_export_beauty_gpu`, `handle_export_channels_gpu`.
- UI:
  - Przyciski „Export All16 GPU", „Export default GPU", „Export prefix GPU".
  - `src/main.rs`: przypięte callbacki UI.

#### Zmiany w rdzeniu GPU

- `src/gpu_context.rs`:
  - Rozszerzony `GpuPipelineCache` o blur/sharpen/histogram (shader modules, BGL, layouts, pipelines).
  - Publiczne gettery w `GpuContext` do nowych pipeline'ów/BGL.
- `src/shaders/image_processing.wgsl`:
  - Dodane Filmic/Hable/LocalAdaptation i ścieżka lokalnego uśredniania.
- `src/image_cache.rs`:
  - Parametr `local_adaptation_radius` w uniformach do shadera.

#### UX i obsługa

- Więcej trybów tone mapping (przyciski).
- Sekcja „GPU Filters" w panelu.
- Sekcja eksportu z wariantami GPU.
- Status bar pokazuje nazwę bieżącego trybu tone mapping.

#### Jak używać (skrót)

- Tone mapping: wybierz Linear/ACES/Reinhard/Filmic/Hable/Local.
- GPU Filters: użyj przycisków GPU Blur/Sharpen/Histogram (na razie log i status).
- Export (GPU): użyj przycisków w sekcji Export (implementacja I/O w toku).

#### Ograniczenia / TODO

- Implementacje I/O dla filtrów i eksportu GPU są szkieletem (logowanie + status).
- Local Adaptation używa uproszczonego okna 3×3 (do ewentualnej optymalizacji i uogólnienia na promień).
