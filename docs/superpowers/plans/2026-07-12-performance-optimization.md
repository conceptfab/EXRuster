# Performance, Threading & Dead-Code Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate UI-thread stalls (sliders, file open, layer clicks, export), fix correctness bugs found in the performance review, speed up the EXR load path, and remove confirmed dead code.

**Architecture:** The core change is a render-off-the-UI-thread pattern: `ImageCache.raw_pixels` becomes `Arc<Vec<f32>>` so a cheap `RenderSnapshot` can be handed to `rayon::spawn`; workers render into `SharedPixelBuffer` (which is `Send`) and the Slint event loop only calls `Image::from_rgba8` + property setters. A global generation counter drops stale frames. Everything else (exact layer matching, negative-result caching, bulk sample conversion, f32 TIFF, dead-code sweep) is independent and ordered so earlier tasks don't conflict with later ones.

**Tech Stack:** Rust (nightly, portable SIMD), Slint 1.12+, rayon, exr 1.73, image 0.25, tiff 0.10.

**Branch:** create `perf/optimization-pass` from `macOS` before starting:
```bash
git checkout macOS && git checkout -b perf/optimization-pass
```

**Verification commands used throughout:**
- `cargo check` — fast compile check
- `cargo test` — all tests
- `cargo clippy` — lint (baseline: 21 warnings; do not add new ones)

Key background facts the implementer must know:
- `slint::Weak<AppWindow>` **is** `Send`; `slint::Image` is **not** `Send`; `slint::SharedPixelBuffer` **is** `Send`. Render to `SharedPixelBuffer` in workers, convert to `Image` only inside `slint::invoke_from_event_loop`.
- `ConsoleModel` is `Rc<VecModel<SharedString>>` — **not** `Send`. Background completions must not capture it; append to `ui.get_console_text()` inside the event-loop closure instead (this pattern already exists in `src/ui/file_handlers.rs:199-207`).
- `UiProgress` (src/ui/progress.rs) holds a `Weak<AppWindow>` and is already used across threads via `Arc` (see `src/ui/file_handlers.rs:137`).
- Rayon pool is global, `num_cpus - 1` threads (src/main.rs:25-28).

---

### Task 1: Exact layer-name matching in `load_all_channels_for_layer_from_full`

The current matcher (src/io/image_cache.rs:481-490) treats a layer as matching if either name is a case-insensitive *substring* of the other, so requesting `"LightMix"` can return layer `"Light"`. The lazy path (src/io/lazy_exr_loader.rs:218) already uses exact `eq_ignore_ascii_case` — make the full path identical.

**Files:**
- Modify: `src/io/image_cache.rs:481-490`
- Test: same file, new `#[cfg(test)]` module at the end

- [ ] **Step 1: Write the failing test**

Append at the end of `src/io/image_cache.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::full_exr_cache::{FullExrCacheData, FullLayer};

    fn make_layer(name: &str, fill: f32) -> FullLayer {
        FullLayer {
            name: name.to_string(),
            width: 2,
            height: 1,
            channel_names: vec!["R".into(), "G".into(), "B".into()],
            channel_data: Arc::from(vec![fill; 6].into_boxed_slice()),
        }
    }

    #[test]
    fn load_layer_matches_exact_name_not_substring() {
        let full = Arc::new(FullExrCacheData {
            layers: vec![make_layer("Light", 1.0), make_layer("LightMix", 2.0)],
        });
        let lc = load_all_channels_for_layer_from_full(&full, "LightMix", None).unwrap();
        assert_eq!(lc.channel_data[0], 2.0, "must return LightMix, not Light");
    }

    #[test]
    fn load_layer_is_case_insensitive() {
        let full = Arc::new(FullExrCacheData {
            layers: vec![make_layer("Beauty", 3.0)],
        });
        let lc = load_all_channels_for_layer_from_full(&full, "beauty", None).unwrap();
        assert_eq!(lc.channel_data[0], 3.0);
    }
}
```

- [ ] **Step 2: Run tests to verify the first one fails**

Run: `cargo test --bin EXruster load_layer_matches -- --nocapture`
Expected: `load_layer_matches_exact_name_not_substring` FAILS (returns 1.0 — the "Light" layer matched first).

- [ ] **Step 3: Fix the matcher**

In `src/io/image_cache.rs`, replace the `matches` computation (lines 481-490):

```rust
    for layer in full.layers.iter() {
        let matches = layer.name.eq_ignore_ascii_case(layer_name);
        if matches {
```

(The empty/empty case is covered: `"".eq_ignore_ascii_case("")` is `true`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bin EXruster load_layer -- --nocapture`
Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add src/io/image_cache.rs
git commit -m "fix: exact layer-name matching in full-cache layer lookup"
```

---

### Task 2: Cache negative color-matrix results

Files without a `chromaticities` attribute (the common case) currently error out of `compute_rgb_to_srgb_matrix_from_file_for_layer` and are never cached, so **every layer/channel click re-reads the EXR header from disk** (src/processing/color_processing.rs:21, called under the app-state write lock). Cache `Option<Mat3>` instead.

**Files:**
- Modify: `src/processing/color_processing.rs`
- Modify: `src/io/image_cache.rs` (3 call sites: lines 103, 158, 266)

- [ ] **Step 1: Change the compute function to distinguish "no attribute" from I/O error**

In `src/processing/color_processing.rs`, change the signature and the primaries handling (lines 15-89):

```rust
pub fn compute_rgb_to_srgb_matrix_from_file_for_layer(
    path: &Path,
    layer_name: &str,
) -> anyhow::Result<Option<Mat3>> {
```

and replace the tail (lines 80-88):

```rust
    let Some((rx, ry, gx, gy, bx, by, wx, wy)) = primaries else {
        // No chromaticities attribute — a valid, cacheable outcome (assume sRGB).
        return Ok(None);
    };

    let m_src = rgb_to_xyz_from_primaries(rx, ry, gx, gy, bx, by, wx, wy);
    // Adaptacja Bradford do D65
    let m_adapt = bradford_adaptation_matrix((wx, wy), (0.3127, 0.3290));
    let m_xyz_to_srgb = xyz_to_srgb_matrix();
    Ok(Some(m_xyz_to_srgb * (m_adapt * m_src)))
}
```

- [ ] **Step 2: Change the cache to store `Option<Mat3>` and the cached fn to return `Option<Mat3>`**

Replace the cache declaration (lines 11-12) and the cached function (lines 166-200):

```rust
static COLOR_MATRIX_CACHE: LazyLock<RwLock<LruCache<(PathBuf, String), Option<Mat3>>>> =
    LazyLock::new(|| RwLock::new(LruCache::new(std::num::NonZeroUsize::new(100).unwrap())));
```

```rust
pub fn compute_rgb_to_srgb_matrix_from_file_for_layer_cached(
    path: &Path,
    layer_name: &str,
) -> Option<Mat3> {
    let key = (path.to_path_buf(), layer_name.to_string());

    if let Ok(cache) = COLOR_MATRIX_CACHE.read() {
        if let Some(&cached) = cache.peek(&key) {
            return cached;
        }
    }

    match compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name) {
        Ok(result) => {
            // Cache both Some(matrix) and None ("no chromaticities") outcomes.
            if let Ok(mut cache) = COLOR_MATRIX_CACHE.write() {
                cache.put(key, result);
            }
            result
        }
        // I/O errors are not cached — the file may appear or change later.
        Err(_) => None,
    }
}
```

Also update the test at line ~217 which inserts `Mat3::IDENTITY` into the cache: wrap the value as `Some(Mat3::IDENTITY)`.

- [ ] **Step 3: Update the 3 call sites in `src/io/image_cache.rs`**

Each currently ends with `.ok()`. Drop it:

```rust
// line 103 (new_with_full_cache) and line 158 (new_with_lazy_loader):
let color_matrix_rgb_to_srgb = crate::processing::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, &best_layer);
// line 266 (load_layer):
self.color_matrix_rgb_to_srgb = crate::processing::color_processing::compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path, layer_name);
```

The `if let Some(matrix) = ...` lines that follow each call site stay unchanged.

- [ ] **Step 4: Verify**

Run: `cargo test && cargo clippy 2>&1 | grep -c warning`
Expected: tests pass; warning count ≤ baseline (21).

- [ ] **Step 5: Commit**

```bash
git add src/processing/color_processing.rs src/io/image_cache.rs
git commit -m "perf: cache negative color-matrix lookups to stop per-click header re-reads"
```

---

### Task 3: Multipart EXR must not use the fast metadata parser

`FastEXRParser::parse_metadata` (src/io/fast_exr_metadata.rs:91-188) reads only the first header, so multipart files silently show 1 layer in the Meta tab. The multipart flag is parsed at line 105 and discarded.

**Files:**
- Modify: `src/io/fast_exr_metadata.rs:105`

- [ ] **Step 1: Bail on multipart so the standard parser takes over**

Replace line 105 (`let _is_multipart = ...`) with:

```rust
        let is_multipart = (version & 0x1000) != 0;
        if is_multipart {
            // Fast parser reads only the first header; multipart files must use
            // the standard exr-crate path (read_and_group_metadata falls back on Err).
            return Err("multipart EXR not supported by fast parser".into());
        }
```

The fallback already exists in `read_and_group_metadata` (src/io/exr_metadata.rs:35-49).

- [ ] **Step 2: Verify and commit**

Run: `cargo test`
Expected: PASS.

```bash
git add src/io/fast_exr_metadata.rs
git commit -m "fix: route multipart EXR metadata through the standard parser"
```

---

### Task 4: Render infrastructure — `Arc` pixels, `render_to_buffer`, `RenderSnapshot`

This is the enabler for Tasks 5-7. `ImageCache.raw_pixels` becomes `Arc<Vec<f32>>` so background renders can share pixels without copying; a free function renders into a `Send`-able `SharedPixelBuffer`.

**Files:**
- Modify: `src/io/image_cache.rs`

- [ ] **Step 1: Change `raw_pixels` to `Arc<Vec<f32>>` and add snapshot/render API**

In `src/io/image_cache.rs`:

1. Field (line 64): `pub raw_pixels: Arc<Vec<f32>>,`
2. In all three constructors (`new_with_full_cache`, `new_with_lazy_loader`, `new_from_hdr`) wrap the composed pixels: `raw_pixels: Arc::new(raw_pixels),`
3. In `load_layer` (line 260): `compose_composite_into_buffer(&layer_channels, Arc::make_mut(&mut self.raw_pixels));`
4. In `load_channel` (lines 670-680):

```rust
        // Reuse existing raw_pixels buffer - expand grayscale channel to RGBA.
        // make_mut copies only if a background render still holds the old Arc.
        let buffer_size = pixel_count * 4;
        let raw = Arc::make_mut(&mut self.raw_pixels);
        raw.resize(buffer_size, 0.0);

        raw.par_chunks_exact_mut(4).enumerate().for_each(|(i, chunk)| {
            let v = channel_slice[i];
            chunk[0] = v;
            chunk[1] = v;
            chunk[2] = v;
            chunk[3] = 1.0;
        });
```

(`update_histogram`, `process_depth_image_with_progress` and the `process_*` readers compile unchanged — `&Arc<Vec<f32>>` deref-coerces to `&[f32]`.)

5. Add after the `ImageCache` struct:

```rust
/// Everything a background thread needs to render a frame, detached from the cache.
#[derive(Clone)]
pub struct RenderSnapshot {
    pub pixels: Arc<Vec<f32>>,
    pub width: u32,
    pub height: u32,
    pub color_matrix: Option<Mat3>,
}

/// Render RGBA f32 pixels to an 8-bit buffer. Safe to call from any thread;
/// SharedPixelBuffer is Send — convert to slint::Image only on the event loop.
pub fn render_to_buffer(
    pixels: &[f32],
    width: u32,
    height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    color_matrix: Option<Mat3>,
) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    crate::processing::simd_processing::process_rgba_chunk_optimized(
        pixels,
        buffer.make_mut_slice(),
        exposure,
        gamma,
        tonemap_mode,
        color_matrix,
        false,
        true, // always parallel — callers are off the UI thread now
    );
    buffer
}
```

6. Add to `impl ImageCache`:

```rust
    pub fn snapshot(&self) -> RenderSnapshot {
        RenderSnapshot {
            pixels: Arc::clone(&self.raw_pixels),
            width: self.width,
            height: self.height,
            color_matrix: self.color_matrix_rgb_to_srgb,
        }
    }
```

7. Reimplement `process_to_image` on top of it (delete the old body incl. the two `log_info!` spam lines and `process_rgba_chunks_optimized`):

```rust
    pub fn process_to_image(&self, exposure: f32, gamma: f32, tonemap_mode: i32) -> Image {
        Image::from_rgba8(render_to_buffer(
            &self.raw_pixels,
            self.width,
            self.height,
            exposure,
            gamma,
            tonemap_mode,
            self.color_matrix_rgb_to_srgb,
        ))
    }
```

- [ ] **Step 2: Split depth rendering into a buffer-producing function**

In the second `impl ImageCache` block, rename `process_depth_image_with_progress` internals: the existing body becomes `process_depth_to_buffer` returning the buffer, and a thin wrapper keeps the old name:

```rust
    pub fn process_depth_to_buffer(
        &self,
        invert: bool,
        progress: Option<&dyn ProgressSink>,
    ) -> SharedPixelBuffer<Rgba8Pixel> {
        // ... entire existing body of process_depth_image_with_progress,
        // with BOTH `return Image::from_rgba8(buffer);` (empty-values early
        // return) and the final `Image::from_rgba8(buffer)` replaced by
        // `return buffer;` / `buffer`.
    }

    /// Specjalne renderowanie głębi: auto-normalizacja percentylowa + opcjonalne odwrócenie
    pub fn process_depth_image_with_progress(
        &self,
        invert: bool,
        progress: Option<&dyn ProgressSink>,
    ) -> Image {
        Image::from_rgba8(self.process_depth_to_buffer(invert, progress))
    }
```

- [ ] **Step 3: Verify**

Run: `cargo check && cargo test`
Expected: compiles, tests pass. If `compose_composite_from_channels` callers elsewhere complain (layer_export.rs uses it and expects `Vec<f32>` — it still returns `Vec<f32>`, only the struct field changed), fix only type mismatches at `raw_pixels` use sites.

- [ ] **Step 4: Commit**

```bash
git add src/io/image_cache.rs
git commit -m "refactor: Arc-shared pixels + thread-safe render_to_buffer/snapshot API"
```

---

### Task 5: Async preview rendering for sliders / tonemap / resize

`process_to_image` currently runs on the UI thread on every exposure/gamma throttle tick (src/ui/image_controls.rs:162), tonemap change (src/ui/setup.rs:40) and geometry debounce (src/ui/setup.rs:229). Move it to rayon with a generation counter and a fast decimated first pass.

**Files:**
- Modify: `src/ui/image_controls.rs` (replace `update_preview_image` + `handle_parameter_changed_throttled`)
- Modify: `src/ui/setup.rs:34-66` (tonemap), `:218-237` (geometry debounce)
- Modify: `src/ui/mod.rs:15`, `src/ui/ui_handlers.rs:52` (remove `update_preview_image` re-export)

- [ ] **Step 1: Add the async render entry point to `src/ui/image_controls.rs`**

Delete `update_preview_image` and `LAST_PREVIEW_LOG` (lines 9, 132-181). Add:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic render generation — the event loop only accepts the newest frame.
static RENDER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Longer side of the image as displayed (contain-fit, DPI-aware).
fn compute_display_target(ui: &AppWindow, img_w: u32, img_h: u32) -> u32 {
    let preview_w = ui.get_preview_area_width();
    let preview_h = ui.get_preview_area_height();
    let dpr = ui.window().scale_factor();
    let container_ratio = if preview_h > 0.0 { preview_w / preview_h } else { 1.0 };
    let image_ratio = if img_h > 0 { img_w as f32 / img_h as f32 } else { 1.0 };
    let display_long_side_logical = if container_ratio > image_ratio {
        preview_h * image_ratio
    } else {
        preview_w
    };
    (display_long_side_logical * dpr).round().max(1.0) as u32
}

/// Stride-decimate RGBA pixels so the interactive pass renders at ~display size.
fn decimate_for_display(
    pixels: &std::sync::Arc<Vec<f32>>,
    width: u32,
    height: u32,
    target_long_side: u32,
) -> (std::sync::Arc<Vec<f32>>, u32, u32) {
    let long_side = width.max(height);
    if target_long_side == 0 || long_side <= target_long_side.saturating_mul(3) / 2 {
        return (std::sync::Arc::clone(pixels), width, height);
    }
    let step = (long_side / target_long_side).max(1) as usize;
    let new_w = (width as usize).div_ceil(step) as u32;
    let new_h = (height as usize).div_ceil(step) as u32;
    let mut out = Vec::with_capacity((new_w as usize) * (new_h as usize) * 4);
    for y in (0..height as usize).step_by(step) {
        let row = y * width as usize;
        for x in (0..width as usize).step_by(step) {
            let i = (row + x) * 4;
            out.extend_from_slice(&pixels[i..i + 4]);
        }
    }
    (std::sync::Arc::new(out), new_w, new_h)
}

/// Kick off a preview render on the rayon pool. Two passes: a decimated frame
/// for instant feedback, then full resolution if no newer render superseded it.
pub fn spawn_preview_render(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) {
    let (snapshot, target) = {
        let Some(ui) = ui_handle.upgrade() else { return };
        let Ok(state) = app_state.read() else { return };
        let Some(cache) = state.image_cache.as_ref() else { return };
        (cache.snapshot(), compute_display_target(&ui, cache.width, cache.height))
    };

    let generation = RENDER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    rayon::spawn(move || {
        let post_frame = |buffer: slint::SharedPixelBuffer<slint::Rgba8Pixel>| {
            let ui_handle = ui_handle.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if RENDER_GENERATION.load(Ordering::SeqCst) != generation {
                    return; // a newer render was scheduled — drop this frame
                }
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_exr_image(slint::Image::from_rgba8(buffer));
                }
            });
        };

        let (small, sw, sh) =
            decimate_for_display(&snapshot.pixels, snapshot.width, snapshot.height, target);
        let decimated = sw != snapshot.width || sh != snapshot.height;
        post_frame(crate::io::image_cache::render_to_buffer(
            &small, sw, sh, exposure, gamma, tonemap_mode, snapshot.color_matrix,
        ));

        if decimated && RENDER_GENERATION.load(Ordering::SeqCst) == generation {
            post_frame(crate::io::image_cache::render_to_buffer(
                &snapshot.pixels,
                snapshot.width,
                snapshot.height,
                exposure,
                gamma,
                tonemap_mode,
                snapshot.color_matrix,
            ));
        }
    });
}
```

- [ ] **Step 2: Rewrite `handle_parameter_changed_throttled` to use it**

```rust
pub fn handle_parameter_changed_throttled(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    _console: ConsoleModel,
    exposure: Option<f32>,
    gamma: Option<f32>,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let final_exposure = exposure.unwrap_or_else(|| ui.get_exposure_value());
        let final_gamma = gamma.unwrap_or_else(|| ui.get_gamma_value());
        let tonemap_mode = ui.get_tonemap_mode();

        spawn_preview_render(
            ui.as_weak(),
            app_state,
            final_exposure,
            final_gamma,
            tonemap_mode,
        );

        if exposure.is_some() && gamma.is_some() {
            ui.set_status_text(
                format!("🔄 Exposure: {:.2} EV, Gamma: {:.2}", final_exposure, final_gamma).into(),
            );
        } else if exposure.is_some() {
            ui.set_status_text(format!("🔄 Exposure: {:.2} EV", final_exposure).into());
        } else if gamma.is_some() {
            ui.set_status_text(format!("🔄 Gamma: {:.2}", final_gamma).into());
        }
    }
}
```

Clean up now-unused imports in image_controls.rs (`ImageCache`, `push_console`, `Instant` if unreferenced).

- [ ] **Step 3: Switch setup.rs call sites**

In `src/ui/setup.rs`, `handle_tonemap_change` (lines 34-66) — replace the render block:

```rust
    fn handle_tonemap_change(&self, mode: i32) {
        if let Some(ui) = self.ui_weak.upgrade() {
            let exposure = ui.get_exposure_value();
            let gamma = ui.get_gamma_value();
            crate::ui::image_controls::spawn_preview_render(
                self.ui_weak.clone(),
                self.app_state.clone(),
                exposure,
                gamma,
                mode,
            );
            push_console(
                &ui,
                &self.console_model,
                format!("[preview] updated → tonemap mode: {}", mode),
            );
            ui.set_status_text(
                format!(
                    "Tonemap: {}",
                    match mode {
                        0 => "ACES",
                        1 => "Reinhard",
                        2 => "Linear",
                        3 => "Filmic",
                        4 => "Hable",
                        5 => "Local",
                        _ => "Unknown",
                    }
                ).into()
            );
        }
    }
```

(The `if let Ok(state) = self.app_state.read()` / `if let Some(ref cache)` wrapper disappears — `spawn_preview_render` no-ops when no image is loaded.)

Geometry debounce (lines 218-237) — replace the closure body:

```rust
        let debounced = crate::ui::image_controls::DebouncedGeometry::new(move || {
            if let Some(ui) = ui_handle.upgrade() {
                let exposure = ui.get_exposure_value();
                let gamma = ui.get_gamma_value();
                let mode = ui.get_tonemap_mode();
                crate::ui::image_controls::spawn_preview_render(
                    ui.as_weak(),
                    app_state.clone(),
                    exposure,
                    gamma,
                    mode,
                );
            }
        });
```

Remove the now-unused `console` capture of that closure if clippy flags it.

- [ ] **Step 4: Fix re-exports**

Remove `update_preview_image` from the re-export lists at `src/ui/mod.rs:15` and `src/ui/ui_handlers.rs:52`.

- [ ] **Step 5: Verify**

Run: `cargo check && cargo test && cargo clippy 2>&1 | tail -3`
Expected: compiles; no new warnings. Note: the two `Arc<Mutex<ThrottledUpdate>>` clippy warnings at setup.rs:191/236 are pre-existing (Timer is UI-thread-only; `Rc<RefCell<...>>` would silence them — optional, do it if trivial).

- [ ] **Step 6: Manual smoke test**

Run: `cargo run --bin EXruster -- <some .exr file>` (ask the user for a test file if none is at hand; `EXRUSTER_LAZY_OPEN=1` also worth one pass).
Expected: dragging exposure/gamma sliders updates the preview without freezing the window; final image sharpens to full res after the drag stops.

- [ ] **Step 7: Commit**

```bash
git add src/ui/image_controls.rs src/ui/setup.rs src/ui/mod.rs src/ui/ui_handlers.rs
git commit -m "perf: render previews off the UI thread with stale-frame dropping and decimated first pass"
```

---

### Task 6: File-open — move render + histogram out of the event loop

All three open paths (lazy src/ui/file_handlers.rs:150-211, full :273-348, HDR :445-488) run `process_to_image` + `apply_histogram_to_ui` **inside** `invoke_from_event_loop` while holding the write lock. The cache is constructed on the rayon thread — do the heavy work there, before handing off.

**Files:**
- Modify: `src/ui/file_handlers.rs`

- [ ] **Step 1: Add a histogram-apply helper that takes precomputed data**

In `src/ui/file_handlers.rs`, below `apply_histogram_to_ui`:

```rust
/// Apply an already-computed histogram to the UI (no locking, no recompute).
pub fn apply_histogram_data_to_ui(
    ui: &AppWindow,
    hist_data: &crate::processing::histogram::HistogramData,
) {
    hist_data.apply_to_ui(ui);
    ui.set_histogram_total_pixels(hist_data.total_pixels as i32);
    let lum = crate::processing::histogram::HistogramChannel::Luminance;
    ui.set_histogram_p1(hist_data.get_percentile(lum, 0.01));
    ui.set_histogram_p50(hist_data.get_percentile(lum, 0.50));
    ui.set_histogram_p99(hist_data.get_percentile(lum, 0.99));
}
```

and shrink `apply_histogram_to_ui` to reuse it:

```rust
pub fn apply_histogram_to_ui(ui: &AppWindow, app_state: &SharedAppState) {
    let hist = {
        let Ok(mut state) = app_state.write() else { return };
        let Some(cache) = state.image_cache.as_mut() else { return };
        if cache.update_histogram().is_err() {
            return;
        }
        cache.get_histogram_data()
    };
    if let Some(hist_data) = hist {
        apply_histogram_data_to_ui(ui, &hist_data);
    }
}
```

- [ ] **Step 2: Restructure the FULL path success arm (lines 272-349)**

Replace `Ok(cache) => { ... }` with (pattern shown in full; apply mechanically):

```rust
                                    Ok(mut cache) => {
                                        // Heavy work on the rayon thread: render + histogram.
                                        let snap = cache.snapshot();
                                        let buffer = crate::io::image_cache::render_to_buffer(
                                            &snap.pixels, snap.width, snap.height,
                                            exposure0, gamma0, tonemap_mode0, snap.color_matrix,
                                        );
                                        let _ = cache.update_histogram();
                                        let hist = cache.get_histogram_data();

                                        let _ = invoke_from_event_loop(move || {
                                            if let Some(ui2) = ui_weak.upgrade() {
                                                let layers_info_vec = {
                                                    if let Ok(mut state) = app_state_c.write() {
                                                        state.full_exr_cache = Some(full.clone());
                                                        state.image_cache = Some(cache);
                                                        state
                                                            .image_cache
                                                            .as_ref()
                                                            .map(|c| c.layers_info.clone())
                                                            .unwrap_or_default()
                                                    } else {
                                                        Vec::new()
                                                    }
                                                };
                                                ui2.set_exr_image(slint::Image::from_rgba8(buffer));
                                                if let Some(h) = hist {
                                                    apply_histogram_data_to_ui(&ui2, &h);
                                                }
                                                // ... keep the existing layers-model +
                                                // console/status/progress code unchanged,
                                                // using layers_info_vec.len() where
                                                // layers_info_len was used before ...
                                            }
                                        });
                                    }
```

- [ ] **Step 3: Apply the same restructure to the LAZY path (lines 149-212) and HDR path (lines 443-489)**

Identical pattern: `Ok(mut cache)` → snapshot → `render_to_buffer` → `update_histogram` → `get_histogram_data` on the rayon thread; inside the event loop only: state assignment (`state.full_exr_cache = None;` for lazy/HDR), `set_exr_image(Image::from_rgba8(buffer))`, `apply_histogram_data_to_ui`, layers model, console/status/progress — all existing code kept.

- [ ] **Step 4: Verify**

Run: `cargo check && cargo test`
Then: `cargo run --bin EXruster -- <test.exr>` — file opens, image + histogram + layer tree appear, no long white-window freeze at the end of loading.

- [ ] **Step 5: Commit**

```bash
git add src/ui/file_handlers.rs
git commit -m "perf: compute first render and histogram on the loader thread, not the event loop"
```

---

### Task 7: Layer/channel clicks — load + render in the background

`handle_layer_tree_click` (src/ui/layers.rs) does disk I/O (lazy mode), a full composite copy and a **sequential** full-image tone-map on the UI thread inside the write lock. Move the work to rayon; render in parallel.

**Files:**
- Modify: `src/ui/layers.rs` (kind==2 and kind==3 branches)

- [ ] **Step 1: Rewrite the layer branch (kind == 2, lines 69-160)**

```rust
    // WARSTWA — kind 2
    else if kind == 2 {
        if let Some(ui) = ui_handle.upgrade() {
            let layer_name = trimmed.to_string();

            let real_layer_name = {
                let map = lock_or_recover(&crate::ui::file_handlers::DISPLAY_TO_REAL_LAYER);
                map.get(&layer_name)
                    .cloned()
                    .unwrap_or_else(|| layer_name.clone())
            };

            push_console(
                &ui,
                &console,
                format!("[layer] clicked: {} (real='{}')", layer_name, real_layer_name),
            );
            ui.set_status_text(format!("Loading layer: {}", layer_name).into());

            let exposure = ui.get_exposure_value();
            let gamma = ui.get_gamma_value();
            let tonemap_mode = ui.get_tonemap_mode();
            let progress = std::sync::Arc::new(crate::ui::progress::UiProgress::new(ui.as_weak()));
            progress.start_indeterminate(Some("Loading layer"));
            let ui_weak = ui.as_weak();
            let app_state = app_state.clone();

            rayon::spawn(move || {
                // Load + render off the UI thread; the write lock never blocks the event loop.
                let result = (|| -> anyhow::Result<_> {
                    let mut state = app_state
                        .write()
                        .map_err(|_| anyhow::anyhow!("app state lock poisoned"))?;
                    let path = state
                        .current_file_path
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("No file loaded"))?;
                    let cache = state
                        .image_cache
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("No image cache loaded"))?;
                    cache.load_layer(&path, &real_layer_name, Some(progress.as_ref()))?;
                    let channels = cache
                        .layers_info
                        .iter()
                        .find(|l| l.name == real_layer_name)
                        .map(|l| {
                            l.channels
                                .iter()
                                .map(|c| c.name.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|| "?".into());
                    Ok((cache.snapshot(), channels))
                })();

                match result {
                    Ok((snap, channels)) => {
                        let buffer = crate::io::image_cache::render_to_buffer(
                            &snap.pixels, snap.width, snap.height,
                            exposure, gamma, tonemap_mode, snap.color_matrix,
                        );
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_exr_image(slint::Image::from_rgba8(buffer));
                                ui.set_status_text(
                                    format!(
                                        "Layer: {} | mode: RGB | channels: {}",
                                        real_layer_name, channels
                                    )
                                    .into(),
                                );
                                ui.set_selected_layer_item(layer_name.into());
                                progress.finish(None);
                            }
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status_text(
                                    format!("Error loading layer {}: {}", real_layer_name, e).into(),
                                );
                                progress.reset();
                            }
                        });
                    }
                }
            });
        }
    }
```

Note: console pushes from the completion are dropped (ConsoleModel is not `Send`); status text carries the same information. `patterns::processing` / `ScopedProgress` is replaced by explicit `UiProgress` start/finish because the scoped guard cannot cross threads.

- [ ] **Step 2: Rewrite the channel branch (kind == 3, lines 162-312)**

Keep the existing parsing of `(active_layer, channel_short)` on the UI thread (it needs `cache.current_layer_name` for the fallback — replace that one lookup with a read lock):

```rust
        if let Some(ui) = ui_handle.upgrade() {
            let current_layer_fallback = app_state
                .read()
                .ok()
                .and_then(|s| s.image_cache.as_ref().map(|c| c.current_layer_name.clone()))
                .unwrap_or_default();

            let (active_layer, channel_short) = {
                // ... existing parsing block, verbatim, with the single change:
                // `cache.current_layer_name.clone()` → `current_layer_fallback.clone()`
            };
            let channel_short = normalize_channel_name(&channel_short);

            let exposure_unused = ();
            let _ = exposure_unused; // (delete the old _exposure/_gamma reads)

            let progress = std::sync::Arc::new(crate::ui::progress::UiProgress::new(ui.as_weak()));
            progress.start_indeterminate(Some("Loading channel"));
            let ui_weak = ui.as_weak();
            let app_state = app_state.clone();
            let clicked_item_owned = clicked_item.clone();

            rayon::spawn(move || {
                let upper = channel_short.to_ascii_uppercase();
                let is_depth = upper == "Z" || upper.contains("DEPTH");

                let result = (|| -> anyhow::Result<_> {
                    let mut state = app_state
                        .write()
                        .map_err(|_| anyhow::anyhow!("app state lock poisoned"))?;
                    let path = state
                        .current_file_path
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("No file loaded"))?;
                    let cache = state
                        .image_cache
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("No image cache loaded"))?;
                    cache.load_channel(&path, &active_layer, &channel_short, Some(progress.as_ref()))?;
                    // invert only for depth; both modes use percentile normalization
                    Ok(cache.process_depth_to_buffer(is_depth, Some(progress.as_ref())))
                })();

                match result {
                    Ok(buffer) => {
                        let mode_label = if is_depth {
                            "Depth (auto-normalized, inverted)"
                        } else {
                            "Grayscale (auto-normalized)"
                        };
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_exr_image(slint::Image::from_rgba8(buffer));
                                ui.set_status_text(
                                    format!(
                                        "Layer: {} | Channel: {} | mode: {}",
                                        active_layer, channel_short, mode_label
                                    )
                                    .into(),
                                );
                                ui.set_selected_layer_item(clicked_item_owned.into());
                                progress.finish(None);
                            }
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status_text(
                                    format!(
                                        "Error loading channel {}: {}",
                                        channel_short, e
                                    )
                                    .into(),
                                );
                                progress.reset();
                            }
                        });
                    }
                }
            });
        }
```

Delete the now-unused `status_msg`/`console_buffer` builders and unused imports (`std::fmt::Write` if orphaned, `UiErrorReporter` if orphaned).

- [ ] **Step 3: Delete `process_to_composite` and its helper**

With the layer click now using `render_to_buffer`, `process_to_composite` + `process_rgba_chunks_composite_optimized` (src/io/image_cache.rs:343-391) have no callers. Verify with `grep -rn "process_to_composite" src/` (expect no hits outside image_cache.rs), then delete both.

- [ ] **Step 4: Verify**

Run: `cargo check && cargo test`
Then `cargo run --bin EXruster -- <multi-layer.exr>`: clicking layers and channels updates the preview; the window stays responsive during the switch; group expand/collapse still works (kind 0/1 branch untouched).

- [ ] **Step 5: Commit**

```bash
git add src/ui/layers.rs src/io/image_cache.rs
git commit -m "perf: load and render layer/channel switches on the rayon pool"
```

---

### Task 8: True 32-bit float TIFF export

`save_tiff32_float` (src/processing/layer_export.rs:442-481) currently quantizes to u16 and converts back — the output has 16-bit precision, no HDR range. Make the f32 path carry raw linear data end to end (a float export is the "give me the real data" path — no tone mapping, no quantization).

**Files:**
- Modify: `src/processing/layer_export.rs`
- Test: same file, new `#[cfg(test)]` module

- [ ] **Step 1: Write the failing test**

Append at the end of `src/processing/layer_export.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::full_exr_cache::{FullExrCacheData, FullLayer};

    fn test_exporter() -> LayerExporter {
        let layer = FullLayer {
            name: "Beauty".into(),
            width: 2,
            height: 1,
            channel_names: vec!["R".into(), "G".into(), "B".into()],
            // planar: R=[5.0,5.0] G=[0.5,0.5] B=[0.25,0.25]
            channel_data: std::sync::Arc::from(
                vec![5.0f32, 5.0, 0.5, 0.5, 0.25, 0.25].into_boxed_slice(),
            ),
        };
        let cache = std::sync::Arc::new(FullExrCacheData { layers: vec![layer] });
        let layers_info = cache.to_layers_info();
        LayerExporter::new(cache, layers_info)
    }

    #[test]
    fn tiff32_export_preserves_hdr_values() {
        let exporter = test_exporter();
        let lc = exporter.load_layer_channels("Beauty").unwrap();
        let processed = exporter
            .process_layer_to_pixels(&lc, &ExportFormat::Tiff32Float)
            .unwrap();
        let PixelData::F32(data) = processed.data else {
            panic!("expected f32 pixel data for Tiff32Float");
        };
        assert_eq!(processed.channels, 3);
        assert_eq!(data[0], 5.0, "HDR value above 1.0 must survive f32 export");
    }

    #[test]
    fn png16_export_stays_u16() {
        let exporter = test_exporter();
        let lc = exporter.load_layer_channels("Beauty").unwrap();
        let processed = exporter
            .process_layer_to_pixels(&lc, &ExportFormat::Png16)
            .unwrap();
        assert!(matches!(processed.data, PixelData::U16(_)));
    }
}
```

- [ ] **Step 2: Run tests — expect a compile error (`PixelData` doesn't exist, `process_layer_to_pixels` takes no format)**

Run: `cargo test --bin EXruster tiff32`
Expected: compile FAIL.

- [ ] **Step 3: Introduce `PixelData` and thread the format through**

1. Replace the container (lines 484-490):

```rust
/// Processed pixel data container
enum PixelData {
    U16(Vec<u16>),
    F32(Vec<f32>),
}

struct ProcessedPixels {
    data: PixelData,
    width: u32,
    height: u32,
    channels: u8,
}
```

2. `export_single_layer` (line 167): pass the format:

```rust
        let processed_pixels = self.process_layer_to_pixels(&layer_channels, format)?;
```

3. `process_layer_to_pixels` (lines 185-220):

```rust
    fn process_layer_to_pixels(
        &self,
        layer_channels: &LayerChannels,
        format: &ExportFormat,
    ) -> Result<ProcessedPixels> {
        let pixel_count = (layer_channels.width * layer_channels.height) as usize;
        let rgb_pixels = crate::io::image_cache::compose_composite_from_channels(layer_channels);

        let has_alpha = layer_channels.channel_names.iter().any(|name| {
            let n = name.to_ascii_uppercase();
            n == "A" || n.starts_with("ALPHA")
        });
        let channels: u8 = if has_alpha { 4 } else { 3 };

        let data = match format {
            // 32-bit float export keeps linear HDR data — no tone mapping, no quantization.
            ExportFormat::Tiff32Float => {
                let mut out = Vec::with_capacity(pixel_count * channels as usize);
                for chunk in rgb_pixels.chunks_exact(4) {
                    out.extend_from_slice(&chunk[..channels as usize]);
                }
                PixelData::F32(out)
            }
            ExportFormat::Png16 => PixelData::U16(match layer_channels.channel_names.len() {
                1 => self.process_grayscale_pixels(&rgb_pixels, pixel_count)?,
                _ => self.process_color_pixels(&rgb_pixels, pixel_count, has_alpha)?,
            }),
        };

        Ok(ProcessedPixels {
            data,
            width: layer_channels.width,
            height: layer_channels.height,
            channels,
        })
    }
```

4. `save_png16` (lines 415-439): destructure at the top:

```rust
        let PixelData::U16(ref data) = pixels.data else {
            anyhow::bail!("PNG16 export requires 16-bit data");
        };
```
and use `data.clone()` where `pixels.data.clone()` was.

5. `save_tiff32_float` (lines 442-481): drop the u16→f32 conversion:

```rust
        let PixelData::F32(ref float_data) = pixels.data else {
            anyhow::bail!("TIFF float export requires f32 data");
        };
```
keep the `4 =>` / `3 =>` `write_image` arms with `float_data`; delete the unreachable `1 =>` arm (channels is always 3 or 4) and keep the `_ => bail` arm.

- [ ] **Step 4: Run tests**

Run: `cargo test --bin EXruster layer_export`
Expected: both new tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/processing/layer_export.rs
git commit -m "fix: Tiff32Float exports true linear f32 data instead of re-expanded u16"
```

---

### Task 9: Export works in lazy mode

Export requires `state.full_exr_cache` (src/ui/export_handlers.rs:84-88, :285-288), which is `None` for files >700 MB. Route the exporter through `ExrDataSource` so both modes work.

**Files:**
- Modify: `src/io/image_cache.rs` (Clone for `ExrDataSource`, accessor)
- Modify: `src/processing/layer_export.rs` (`LayerExporter` source)
- Modify: `src/ui/export_handlers.rs` (both `*_impl` fns)

- [ ] **Step 1: Make `ExrDataSource` cloneable and expose it**

In `src/io/image_cache.rs`:

```rust
impl Clone for ExrDataSource {
    fn clone(&self) -> Self {
        match self {
            ExrDataSource::Full(c) => ExrDataSource::Full(Arc::clone(c)),
            ExrDataSource::Lazy(l) => ExrDataSource::Lazy(Arc::clone(l)),
            ExrDataSource::Hdr => ExrDataSource::Hdr,
        }
    }
}

impl ImageCache {
    pub fn data_source(&self) -> ExrDataSource {
        self.data_source.clone()
    }
}
```

(Place the `impl ImageCache` addition inside the existing first `impl` block.)

- [ ] **Step 2: Switch `LayerExporter` to `ExrDataSource`**

In `src/processing/layer_export.rs`:

```rust
use crate::io::image_cache::{ExrDataSource, LayerChannels, LayerInfo};

pub struct LayerExporter {
    source: ExrDataSource,
    layers_info: Vec<LayerInfo>,
    export_params: ExportParams,
}

impl LayerExporter {
    pub fn new(source: ExrDataSource, layers_info: Vec<LayerInfo>) -> Self {
        Self {
            source,
            layers_info,
            export_params: ExportParams::default(),
        }
    }
    // ...
    fn load_layer_channels(&self, layer_name: &str) -> Result<LayerChannels> {
        match &self.source {
            ExrDataSource::Full(full) => {
                crate::io::image_cache::load_all_channels_for_layer_from_full(full, layer_name, None)
            }
            ExrDataSource::Lazy(lazy) => Ok(lazy.get_layer_data(layer_name, None)?.to_layer_channels()),
            ExrDataSource::Hdr => anyhow::bail!("Export is not supported for HDR files"),
        }
    }
}
```

Update the test helper from Task 8:

```rust
        LayerExporter::new(ExrDataSource::Full(cache.clone()), layers_info)
```

Drop the now-unused `use crate::io::full_exr_cache::FullExrCacheData;` from the non-test imports if orphaned.

- [ ] **Step 3: Update export_handlers to build from `image_cache`**

In `src/ui/export_handlers.rs`, in BOTH `export_base_layer_impl` (lines 73-104) and `export_layer_group_impl` (lines 274-297), replace the cache extraction:

```rust
    let state = app_state
        .read()
        .map_err(|_| anyhow::anyhow!("Failed to read app state"))?;

    let cache = state
        .image_cache
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No image loaded"))?;
    let source = cache.data_source();
    let layers_info = cache.layers_info.clone();

    let export_params = create_export_params(ui, &export_config)?;
    let exporter = LayerExporter::new(source, layers_info).with_params(export_params);
```

Delete the `_file_path` validation lines (the `image_cache` presence check subsumes them) and the `full_exr_cache`/`to_layers_info` lines.

- [ ] **Step 4: Verify**

Run: `cargo test`
Then: `EXRUSTER_LAZY_OPEN=1 cargo run --bin EXruster -- <test.exr>` → Export beauty → a PNG appears next to the source file (previously: "No EXR cache available").

- [ ] **Step 5: Commit**

```bash
git add src/io/image_cache.rs src/processing/layer_export.rs src/ui/export_handlers.rs
git commit -m "feat: export works in lazy mode via ExrDataSource"
```

---

### Task 10: Export runs on the rayon pool

`handle_export` (src/ui/export_handlers.rs:203-265) does compose + tone-map + encode + disk writes synchronously in the callback. Freeze for seconds on "export all". Gather UI state first, then spawn.

**Files:**
- Modify: `src/ui/export_handlers.rs`

- [ ] **Step 1: Restructure `handle_export`**

Replace `handle_export` and fold both `*_impl` functions into one background-safe `run_export` (they no longer need `&ui` — `ExportParams` is created up front):

```rust
/// Generic export handler for all export types
pub fn handle_export(
    export_type: ExportType,
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let export_config =
            match create_export_config_from_ui(ui_handle.clone(), &app_state, console.clone()) {
                Some(config) => config,
                None => return,
            };
        let export_params = match create_export_params(&ui, &export_config) {
            Ok(p) => p,
            Err(e) => {
                push_console(&ui, &console, format!("[export] config error: {}", e));
                return;
            }
        };

        let export_name = match export_type {
            ExportType::Beauty => "beauty",
            ExportType::All => "all layers",
            ExportType::Scene => "scene layers",
            ExportType::Objects => "object layers",
            ExportType::Cryptomatte => "cryptomatte layers",
            ExportType::Lights => "light layers",
        }
        .to_string();

        push_console(&ui, &console, format!("[export] {} started", export_name));
        let progress = std::sync::Arc::new(crate::ui::progress::UiProgress::new(ui.as_weak()));
        progress.start_indeterminate(Some(&format!("Exporting {}", export_name)));

        let ui_weak = ui.as_weak();
        let app_state = app_state.clone();

        rayon::spawn(move || {
            let result = run_export(&app_state, &export_config, export_params, &export_type);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    match result {
                        Ok(paths) => {
                            let mut log = ui.get_console_text().to_string();
                            for p in &paths {
                                if !log.is_empty() {
                                    log.push('\n');
                                }
                                log.push_str(&format!("[export] -> {}", p.display()));
                            }
                            ui.set_console_text(log.into());
                            ui.set_status_text(
                                format!("{} export completed ({} files)", export_name, paths.len())
                                    .into(),
                            );
                        }
                        Err(e) => {
                            ui.set_status_text(
                                format!("{} export failed: {}", export_name, e).into(),
                            );
                        }
                    }
                    progress.finish(None);
                }
            });
        });
    }
}

/// Runs on the rayon pool — must not touch UI objects.
fn run_export(
    app_state: &SharedAppState,
    config: &UiExportConfig,
    params: ExportParams,
    export_type: &ExportType,
) -> Result<Vec<PathBuf>> {
    let (source, layers_info) = {
        let state = app_state
            .read()
            .map_err(|_| anyhow::anyhow!("Failed to read app state"))?;
        let cache = state
            .image_cache
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No image loaded"))?;
        (cache.data_source(), cache.layers_info.clone())
    };

    let exporter = LayerExporter::new(source, layers_info).with_params(params);

    match export_type {
        ExportType::Beauty => exporter
            .export_base_layer(config.format.clone(), &config.output_directory, &config.base_filename)
            .map(|p| vec![p]),
        ExportType::All => exporter.export_all_layers(
            config.format.clone(), &config.output_directory, &config.base_filename,
        ),
        ExportType::Scene => exporter.export_layer_group(
            "scene", config.format.clone(), &config.output_directory, &config.base_filename,
        ),
        ExportType::Objects => exporter.export_layer_group(
            "objects", config.format.clone(), &config.output_directory, &config.base_filename,
        ),
        ExportType::Cryptomatte => exporter.export_layer_group(
            "cryptomatte", config.format.clone(), &config.output_directory, &config.base_filename,
        ),
        ExportType::Lights => exporter.export_layer_group(
            "lights", config.format.clone(), &config.output_directory, &config.base_filename,
        ),
    }
}
```

Delete `export_base_layer_impl`, `export_layer_group_impl` and the `patterns::processing` import. Note `create_export_params` gets a small change: `LayerExporter` no longer needs `ui`, but `create_export_params` itself still reads UI values — it stays on the UI thread, called before the spawn (as shown).

- [ ] **Step 2: Verify**

Run: `cargo check && cargo test`
Then export a multi-layer EXR ("export all") and confirm the window stays responsive and the progress spinner actually animates during the export.

- [ ] **Step 3: Commit**

```bash
git add src/ui/export_handlers.rs
git commit -m "perf: run exports on the rayon pool instead of the UI thread"
```

---

### Task 11: Bulk sample conversion + parallel channels in the load path

`build_full_exr_cache` (src/io/full_exr_cache.rs:80-86) and both branches of `LazyExrLoader::load_layer_from_disk` (src/io/lazy_exr_loader.rs:244-247, 286-289) call `value_by_flat_index(i).to_f32()` per sample — an enum dispatch per sample, billions of them for large files, single-threaded.

**Files:**
- Modify: `src/io/full_exr_cache.rs`
- Modify: `src/io/lazy_exr_loader.rs`

- [ ] **Step 1: Add the bulk converter helper**

In `src/io/full_exr_cache.rs` (uses `exr::prelude as exr`; if `FlatSamples` is not in the prelude, import `::exr::image::FlatSamples` directly):

```rust
/// Bulk-convert a channel's samples with one dispatch per channel instead of
/// a per-sample enum match through value_by_flat_index().
pub(crate) fn flat_samples_to_f32(samples: &exr::FlatSamples) -> Vec<f32> {
    match samples {
        exr::FlatSamples::F16(v) => v.iter().map(|x| x.to_f32()).collect(),
        exr::FlatSamples::F32(v) => v.clone(),
        exr::FlatSamples::U32(v) => v.iter().map(|&x| x as f32).collect(),
    }
}
```

- [ ] **Step 2: Rewrite the aggregation loop in `build_full_exr_cache` (lines 60-88)**

```rust
    use rayon::prelude::*;

    struct ConvertedChannel {
        layer_name: String,
        short: String,
        width: u32,
        height: u32,
        data: Vec<f32>,
    }

    // Flatten (layer, channel) pairs, then convert all channels in parallel.
    let channel_refs: Vec<_> = any_image
        .layer_data
        .iter()
        .flat_map(|layer| {
            let width = layer.size.width() as u32;
            let height = layer.size.height() as u32;
            let base_attr: Option<String> =
                layer.attributes.layer_name.as_ref().map(|s| s.to_string());
            layer
                .channel_data
                .list
                .iter()
                .map(move |ch| (width, height, base_attr.clone(), ch))
        })
        .collect();

    let converted: Vec<ConvertedChannel> = channel_refs
        .into_par_iter()
        .map(|(width, height, base_attr, ch)| {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());
            ConvertedChannel {
                layer_name: lname,
                short,
                width,
                height,
                data: flat_samples_to_f32(&ch.sample_data),
            }
        })
        .collect();

    // Aggregate sequentially, preserving first-occurrence layer order.
    let mut layer_map: HashMap<String, (u32, u32, Vec<String>, Vec<f32>)> = HashMap::new();
    let mut layer_order: Vec<String> = Vec::with_capacity(any_image.layer_data.len());

    for c in converted {
        let entry = layer_map.entry(c.layer_name.clone()).or_insert_with(|| {
            layer_order.push(c.layer_name.clone());
            (c.width, c.height, Vec::new(), Vec::new())
        });
        // Jeśli rozmiary różnią się (rzadkie), preferuj pierwszy i pomiń konfliktujące kanały
        if entry.0 != c.width || entry.1 != c.height {
            continue;
        }
        entry.2.push(c.short);
        entry.3.extend_from_slice(&c.data);
    }
```

(The `out_layers` tail from line 90 stays unchanged.)

- [ ] **Step 3: Use the bulk converter in the lazy loader**

In `src/io/lazy_exr_loader.rs`, selective-read branch (lines 244-247):

```rust
                let converted = crate::io::full_exr_cache::flat_samples_to_f32(&ch.sample_data);
                data.extend_from_slice(&converted[..pixel_count.min(converted.len())]);
```

and fallback branch (lines 286-289):

```rust
                    let converted = crate::io::full_exr_cache::flat_samples_to_f32(&ch.sample_data);
                    channel_data.extend_from_slice(&converted[..pixel_count.min(converted.len())]);
```

- [ ] **Step 4: Verify**

Run: `cargo test`
Then open a large multi-layer EXR and compare the "[cache] cache created (N ms)" console line before/after (expect a noticeable drop on multi-channel files).

- [ ] **Step 5: Commit**

```bash
git add src/io/full_exr_cache.rs src/io/lazy_exr_loader.rs
git commit -m "perf: bulk per-channel sample conversion, parallel across channels"
```

---

### Task 12: Reuse in-RAM metadata instead of re-reading the file header

`new_with_full_cache` (src/io/image_cache.rs:92) calls `extract_layers_info(path)` — a disk header parse — although `FullExrCacheData::to_layers_info()` produces the same structure from RAM (already used by the exporter).

**Files:**
- Modify: `src/io/image_cache.rs`

- [ ] **Step 1: Replace the call and delete the dead function**

Line 92:

```rust
        let layers_info = full_cache.to_layers_info();
```

`extract_layers_info` (lines 398-440) now has no callers (verify: `grep -rn "extract_layers_info" src/` → only the definition and a comment in full_exr_cache.rs). Delete the function; update the comment at full_exr_cache.rs:54 to reference `to_layers_info`. Remove orphaned imports (`HashMap` stays — used elsewhere in the file; check `split_layer_and_short`).

- [ ] **Step 2: Verify and commit**

Run: `cargo test`

```bash
git add src/io/image_cache.rs src/io/full_exr_cache.rs
git commit -m "perf: build layer info from the in-RAM cache instead of re-reading headers"
```

---

### Task 13: Thumbnail pipeline — resize before tone mapping, zero-copy handoff

Thumbnails currently tone-map (3× `powf`) every full-res pixel before resizing to ~390 px (src/io/thumbnails.rs:224-237), the LRU clones full pixel vecs per hit (:431, :457), the UI handoff copies pixels twice (src/ui/thumbnails.rs:119-158), and `main.rs:111` uses height 130 while the canonical height is 390 — so app-start thumbnails are generated twice.

**Files:**
- Modify: `src/main.rs:111`
- Modify: `src/io/thumbnails.rs`
- Modify: `src/ui/thumbnails.rs`

- [ ] **Step 1: Canonical height in main.rs**

Line 107-112: replace `130` with `crate::ui::browser_handlers::CANONICAL_THUMB_HEIGHT`.

- [ ] **Step 2: EXR thumbnails — read linear f32, resize, then tone-map at thumb size**

Replace the body of `generate_single_exr_thumbnail_work_new` (src/io/thumbnails.rs:204-290):

```rust
pub fn generate_single_exr_thumbnail_work_new(
    exr_path: &Path,
    thumb_height: u32,
    color_config: &ColorConfig,
    timing_stats: &TimingStats,
) -> anyhow::Result<ExrThumbWork> {
    let load_start = Instant::now();

    // Read linear f32 pixels — tone mapping happens AFTER downscaling,
    // so the expensive per-pixel math runs at thumb resolution, not full res.
    let reader = exr::read_first_rgba_layer_from_file(
        exr_path,
        |resolution, _| exr::pixel_vec::PixelVec {
            resolution,
            pixels: vec![[0.0f32; 4]; resolution.width() * resolution.height()],
        },
        |pixel_vec, position, (r, g, b, a): (f32, f32, f32, f32)| {
            let index = position.y() * pixel_vec.resolution.width() + position.x();
            pixel_vec.pixels[index] = [r, g, b, a];
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to read EXR: {}", e))?;

    // Count layers from file metadata headers (fast header-only read, no pixel data)
    let num_layers = ::exr::meta::MetaData::read_from_file(exr_path, false)
        .map(|meta| meta.headers.len())
        .unwrap_or(1);

    let image_data = reader.layer_data.channel_data.pixels;
    let (width, height) = (
        image_data.resolution.width() as u32,
        image_data.resolution.height() as u32,
    );
    let thumb_width = (width as f32 / height as f32 * thumb_height as f32) as u32;

    let raw: Vec<f32> = image_data.pixels.into_iter().flatten().collect();
    let img = image::ImageBuffer::<image::Rgba<f32>, _>::from_raw(width, height, raw)
        .ok_or_else(|| anyhow::anyhow!("Could not create image buffer"))?;
    let small = image::imageops::resize(
        &img,
        thumb_width,
        thumb_height,
        image::imageops::FilterType::Triangle,
    );

    let pixels = tonemap_thumb_rgba(small.as_raw(), color_config);

    timing_stats.add_load_time(load_start.elapsed());

    let raw_name = exr_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string());
    let file_name = crate::utils::normalize_display_name(&raw_name);
    let file_size_bytes = fs::metadata(exr_path).map(|m| m.len()).unwrap_or(0);

    Ok(ExrThumbWork {
        path: exr_path.to_path_buf(),
        file_name,
        file_size_bytes,
        width: thumb_width,
        height: thumb_height,
        num_layers,
        pixels,
    })
}

/// Tone-map interleaved linear RGBA f32 to RGBA8 using the shared pipeline
/// (sRGB OETF for gamma≈2.2 instead of raw powf).
fn tonemap_thumb_rgba(rgba: &[f32], color_config: &ColorConfig) -> Arc<[u8]> {
    let params = crate::processing::tone_mapping::ToneMapParams::new(
        color_config.exposure,
        color_config.gamma,
        color_config.tonemap_mode as i32,
    );
    let mut out = Vec::with_capacity(rgba.len());
    for chunk in rgba.chunks_exact(4) {
        let (r, g, b) = crate::processing::tone_mapping::tone_map_and_gamma(
            chunk[0],
            chunk[1],
            chunk[2],
            params.exposure_multiplier,
            params.gamma_inv,
            params.use_srgb,
            params.mode,
        );
        out.push((r.clamp(0.0, 1.0) * 255.0) as u8);
        out.push((g.clamp(0.0, 1.0) * 255.0) as u8);
        out.push((b.clamp(0.0, 1.0) * 255.0) as u8);
        out.push((chunk[3].clamp(0.0, 1.0) * 255.0) as u8);
    }
    Arc::from(out.into_boxed_slice())
}
```

Add `use std::sync::Arc;` to the imports. Check the exact signature of `ToneMapParams::new` and `tone_map_and_gamma` in `src/processing/tone_mapping.rs` before writing (they are used with this exact shape in `src/processing/layer_export.rs:226-242`); if `ToneMapParams::new` takes `ToneMapMode` rather than `i32`, drop the `as i32` cast.

- [ ] **Step 3: Same restructure for HDR thumbnails**

Replace the pixel loop in `generate_single_hdr_thumbnail_work` (lines 293-350): build `ImageBuffer::<image::Rgba<f32>, _>::from_raw(width, height, hdr.rgba)`, resize to thumb size, then `let pixels = tonemap_thumb_rgba(small.as_raw(), color_config);`. Delete the manual `powf` loop.

- [ ] **Step 4: `Arc<[u8]>` in the work item and LRU**

1. `ExrThumbWork.pixels: Vec<u8>` → `pub pixels: Arc<[u8]>,` (line 200)
2. `ThumbValue.pixels: Vec<u8>` → `pixels: Arc<[u8]>,` (line 378)
3. `c_get` (line 431): `pixels: v.pixels.clone()` is now a cheap Arc clone — no code change needed, just no longer copies.
4. `put_thumb_cache` (line 457): same — `pixels: work.pixels.clone()`.

- [ ] **Step 5: Build `SharedPixelBuffer` off-thread in `src/ui/thumbnails.rs`**

Replace the `prepared` mapping (lines 117-142) with:

```rust
            // Build SharedPixelBuffers in the background (Send) — the event loop
            // then only wraps them in slint::Image, no pixel copying on the UI thread.
            prog.set(0.92, Some("🎨 Preparing pixel data..."));
            let prepared: Vec<_> = sorted_works
                .into_iter()
                .map(|w| {
                    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w.width, w.height);
                    buffer.make_mut_bytes().copy_from_slice(&w.pixels);
                    (
                        buffer,
                        w.file_name,
                        w.file_size_bytes,
                        w.num_layers,
                        w.path,
                        w.width,
                        w.height,
                    )
                })
                .collect();
```

and in the event-loop closure (lines 153-171):

```rust
                    let items: Vec<ThumbItem> = prepared
                        .into_iter()
                        .map(|(buffer, file_name, file_size_bytes, num_layers, path, width, height)| {
                            ThumbItem {
                                img: Image::from_rgba8(buffer),
                                name: file_name.into(),
                                size: human_size(file_size_bytes).into(),
                                layers: format!("{} layers", num_layers).into(),
                                path: path.display().to_string().into(),
                                width: width as i32,
                                height: height as i32,
                            }
                        })
                        .collect();
```

- [ ] **Step 6: Throttle per-file progress messages**

In `src/io/thumbnails.rs::generate_thumbnails_cpu_raw`, messages force an unthrottled event-loop dispatch per file. In both `p.set(frac, Some(&format!(...)))` calls (lines ~103-112 and ~137-147), only attach the message every 16th file:

```rust
                let msg;
                let msg_ref = if n % 16 == 0 || n == total_files {
                    msg = format!(
                        "Processed: {}/{} {}",
                        n,
                        total_files,
                        path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                    );
                    Some(msg.as_str())
                } else {
                    None
                };
                p.set(frac, msg_ref);
```

(analogous for the "Cached:" branch).

- [ ] **Step 7: Verify**

Run: `cargo test`
Then: `cargo run --bin EXruster -- <file-in-a-folder-with-many-exrs>` — thumbnails appear, then click a folder in the tree: the same thumbnails must load instantly from cache (no 130-vs-390 regeneration).

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/io/thumbnails.rs src/ui/thumbnails.rs
git commit -m "perf: tone-map thumbnails after downscale, Arc-backed cache, off-thread pixel buffers"
```

---

### Task 14: File copy off the UI thread

`handle_copy_current_file_to` (src/ui/browser_handlers.rs:97) copies a potentially multi-GB file with `std::fs::copy` in the callback.

**Files:**
- Modify: `src/ui/browser_handlers.rs:96-110`

- [ ] **Step 1: Spawn the copy**

Replace the final `match std::fs::copy(...)` block:

```rust
    ui.set_status_text(format!("Copying {} ...", src.display()).into());
    let ui_weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = std::fs::copy(&src, &dest);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            match result {
                Ok(bytes) => {
                    ui.set_status_text(
                        format!("Copied {} bytes to {}", bytes, dest.display()).into(),
                    );
                }
                Err(e) => {
                    log_error!("Copy failed: {}", e);
                    ui.set_status_text(format!("Copy failed: {}", e).into());
                }
            }
        });
    });
```

(The `push_console` success line is dropped — `ConsoleModel` is not `Send`; status text carries the result.)

- [ ] **Step 2: Verify and commit**

Run: `cargo check`

```bash
git add src/ui/browser_handlers.rs
git commit -m "perf: copy file in a background thread"
```

---

### Task 15: Dead-code sweep

All items below were verified unreferenced by grep during planning. Re-verify each with grep before deleting (a later task may have changed things).

**Files:**
- Modify: `Cargo.toml`, `src/processing/simd_processing.rs`, `src/processing/layer_export.rs`, `src/processing/channel_classification.rs`, `src/utils/error_handling.rs`, `src/io/fast_exr_metadata.rs`, `src/io/thumbnails.rs`, `src/io/file_operations.rs`, `CLAUDE.md`
- Delete: `ui/ui_showcase.slint`

- [ ] **Step 1: Remove the `unified_simd` feature and its divergent second implementation**

The feature is never enabled anywhere, and its `ScalarProcessor` Filmic/tonemap math diverges from the real pipeline — a latent bug if ever turned on.

1. `Cargo.toml`: delete the `[features]` section (lines 34-35).
2. `src/processing/simd_processing.rs`: delete lines 8-9 (`#[cfg(feature = "unified_simd")] use ...ToneMapMode;`), the whole `mod unified_processing` (lines 14-148), and the `#[cfg(feature = "unified_simd")]` block inside `process_scalar_pixels` (lines 307-351, keeping the "Original implementation (fallback)" body as the only body).
3. `src/processing/layer_export.rs`: delete `process_color_pixels_simd_optimized` (lines 302-366) and the `#[cfg(feature = "unified_simd")]` / `#[cfg(not(feature = "unified_simd"))]` wrappers in `process_color_pixels` (keep the scalar body unconditionally). (If Task 8 already restructured this fn, just remove the cfg blocks that remain.)

- [ ] **Step 2: Remove the unreachable grayscale kernel path**

`process_rgba_chunk_optimized`'s `grayscale` parameter is now always `false` (the only remaining callers are `render_to_buffer` and `process_scalar_pixels` recursion). Verify: `grep -rn "process_rgba_chunk_optimized\|process_scalar_pixels" src/` — confirm no caller passes `true`. Then:

1. Remove the `grayscale: bool` parameter from `process_rgba_chunk_optimized` and `process_scalar_pixels`, and the `if grayscale` branches inside both.
2. Delete `process_simd_chunk_grayscale` (lines 192-225) and `store_grayscale_simd` (lines 279-293).
3. Update the call in `render_to_buffer` (src/io/image_cache.rs) to drop the `false,` grayscale argument.
4. `LuminanceWeights` import in simd_processing.rs becomes unused — remove it.

- [ ] **Step 3: Remove smaller confirmed-dead items**

For each, `grep -rn "<symbol>" src/` first; delete only on zero non-definition hits:

1. `src/processing/channel_classification.rs`: `determine_channel_group_ultra_fast` and `CHANNEL_PREFIX_MAP` (lines 34-59, 166-194) — production code always uses the config-based path. Port any tests that only exercised the ultra-fast fn to `determine_channel_group_with_config`.
2. `src/utils/error_handling.rs`: `handle_ui_error!` and `handle_ui_option!` macros (lines 53-113) — zero invocations.
3. `src/io/fast_exr_metadata.rs`: `impl From<FastEXRMetadata> for ExrMetadata` (lines 334-383) — zero call sites.
4. `src/io/thumbnails.rs`: `impl Clone for TimingStats` (lines 27-34), `total_save_time` field, `get_save_time`, `get_total_time`; change the summary log (line ~163) to log only the load time.
5. Delete `ui/ui_showcase.slint` (only compiled via `slint-viewer`, never imported by `appwindow.slint`; verify: `grep -rn "ui_showcase" ui/ build.rs`).

- [ ] **Step 4: Trim `image` crate features and fix the open-dialog filter**

1. `Cargo.toml:23`: the app decodes only HDR via `image` and saves PNG (TIFF goes through the `tiff` crate, EXR through the `exr` crate):

```toml
image = { version = "0.25", default-features = false, features = ["png", "hdr"] }
```

2. `src/io/file_operations.rs:10`: the dialog offers png/jpg/jpeg/gif but every non-`.hdr` selection is parsed as EXR and fails. Restrict the filter to `&["exr", "hdr"]`.

Run `cargo check` — if any `image::` API used in the codebase requires a removed feature (e.g. `image::imageops::resize` needs none), fix by re-adding only the specific feature that's actually required.

- [ ] **Step 5: Refresh CLAUDE.md**

Update the stale sections (they mislead future agents):
- Remove references to `src/processing/pipeline.rs`, `src/utils/buffer_pool.rs`, `src/utils/cache.rs`, `src/utils/progress.rs`, `tokio` (none exist).
- Remove the `unified_simd` feature instructions (`cargo check --features unified_simd`, the SIMD-feature paragraph) — the SIMD pipeline in `simd_processing.rs` is always on.
- Update the module lists to the actual tree (`src/io/`: add `fast_exr_metadata.rs`, `folder_tree.rs`, `hdr_loader.rs`, `lazy_exr_loader.rs`, `progress_reader.rs`, `selective_layer_reader.rs`; `src/ui/`: add `browser_handlers.rs`, `export_handlers.rs`, `file_handlers.rs`, `thumbnails.rs`; `src/utils/`: actual files are `channel_config.rs`, `error_handling.rs`, `logging.rs`, `utils.rs`).
- Key technologies: replace `tokio` with `memmap2` (mmap for >500 MB files), add `lru`, `dashmap`.
- Add one line documenting the threading rule established by this plan: *"All image processing and file I/O runs on the rayon pool; the Slint event loop only assigns `slint::Image`s and properties. Renders go through `image_cache::render_to_buffer` + `RenderSnapshot`."*

- [ ] **Step 6: Verify everything**

Run: `cargo fmt && cargo clippy --all-targets && cargo test && cargo build --release`
Expected: fmt clean, clippy warning count ≤ baseline, all tests pass, release build succeeds.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: remove dead code (unified_simd, grayscale kernels, unused macros/impls) and refresh docs"
```

---

### Task 16: Final verification pass

- [ ] **Step 1: Full check suite**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test && cargo build --release
```
Expected: all green.

- [ ] **Step 2: Manual smoke test (needs a real EXR; ask the user for test files if unavailable)**

1. `cargo run --release --bin EXruster -- <big-multilayer.exr>` — opens without end-of-load freeze; histogram populated.
2. Drag exposure and gamma sliders — preview follows fluidly, sharpens at rest.
3. Change tonemap mode, resize the window — preview updates.
4. Click several layers and channels (incl. a depth/Z channel) — window stays responsive.
5. Export "all layers" as PNG16 and as 32-bit TIFF — UI responsive during export; open the TIFF in an app that reads float TIFFs and confirm values >1.0 survive (or `tiffinfo` shows 32-bit float).
6. `EXRUSTER_LAZY_OPEN=1` — repeat steps 4-5 (lazy-mode layer clicks and export).
7. Open a folder with many EXRs — thumbnails load; re-entering the folder hits the cache instantly.

- [ ] **Step 3: Wrap up**

Use superpowers:finishing-a-development-branch to decide merge/PR.

---

## Self-review notes

- **Spec coverage:** optimization (Tasks 2, 5, 6, 11, 12, 13), multithreading (Tasks 4-7, 10, 14), bottleneck removal (Tasks 5-7, 10, 11, 13 — the top-5 ranked hot paths from the review are all addressed), dead code (Task 15). Correctness bugs found during analysis (Tasks 1, 3, 8, 9) are included because they sit directly on the reworked paths.
- **Type consistency:** `render_to_buffer` and `RenderSnapshot` are defined in Task 4 and consumed in Tasks 5, 6, 7; `PixelData` defined in Task 8, consumed in 8 only; `ExrDataSource::data_source()` defined in Task 9, consumed in 9, 10; `flat_samples_to_f32` defined in Task 11 for both consumers; `tonemap_thumb_rgba` defined and consumed in Task 13.
- **Known API-check points for the implementer:** `exr::prelude::FlatSamples` re-export (Task 11), `ToneMapParams::new` mode argument type (Task 13), `SharedPixelBuffer::make_mut_bytes` (Task 13) — each has a stated fallback.
