# EXRuster Optimization Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove dead code, eliminate duplication, and fix performance hot-path inefficiencies identified by an audit of the EXRuster codebase — without changing observable application behavior.

**Architecture:** Five sequential phases ordered by risk (lowest first): (1) mechanical clippy auto-fixes, (2) dead-code deletion, (3) hot-path performance fixes, (4) de-duplication refactors, (5) code-quality cleanups. Each phase is independently shippable; every task is a single commit with `cargo build` + `cargo test` + manual smoke test as acceptance gates.

**Tech Stack:** Rust (nightly, portable_simd), Slint 1.12, exr 1.73, rayon 1.11, memmap2 0.9, dashmap 6.1, lru 0.16, glam 0.30.

---

## Findings Summary (from audit — 2026-04-17)

- `cargo clippy` reports 41 warnings, 23 auto-fixable.
- `src/utils/buffer_pool.rs` (199 lines) is entirely dead — file starts with `#![allow(dead_code)]`, only referenced by its own tests.
- `src/processing/tone_mapping.rs`: `ToneMapModeId` wrapper and `tone_map_and_gamma_*_safe` variants are dead.
- `src/io/thumbnails.rs`: `generate_thumbnails_gpu_raw` is a thin wrapper that just delegates to the CPU path.
- `src/io/image_cache.rs:478`: `Arc::from(layer.channel_data.as_slice())` clones the entire f32 buffer instead of reusing the source `Arc`.
- `src/processing/tone_mapping.rs`: `exposure_multiplier` and `use_srgb` are recomputed per SIMD chunk (frame-constant values).
- `src/processing/histogram.rs`: two parallel passes over the pixels (min/max, then binning) can be fused.
- `src/io/lazy_exr_loader.rs:183-188`: fake LRU eviction using `cache.keys().next()` — comment admits it.
- `src/processing/layer_export.rs:222-301` vs `src/io/image_cache.rs`: `compose_rgb_from_channels` duplicates composite logic.
- `src/processing/channel_classification.rs:226-269`: hand-rolled SSE2 prefix-match for short strings — `str::starts_with` is fast enough.
- `src/main.rs` uses `println!` for progress logging; many call sites use `eprintln!` inconsistently.

---

## File Structure

Files touched by this plan (and their post-plan responsibility):

- **Delete:** `src/utils/buffer_pool.rs` (entirely dead).
- **Modify:** `src/utils/mod.rs` — remove `pub mod buffer_pool;`.
- **Modify:** `src/io/thumbnails.rs` — collapse GPU-alias wrapper, reduce pixel copy.
- **Modify:** `src/io/image_cache.rs` — fix Arc clone, remove `find_best_layer` indirection, de-duplicate RGB resolution.
- **Modify:** `src/io/lazy_exr_loader.rs` — fix fake LRU, remove `Arc::make_mut` growth hot spot.
- **Modify:** `src/processing/tone_mapping.rs` — hoist frame-constants, delete dead safe-wrappers, delete `ToneMapModeId`.
- **Modify:** `src/processing/simd_processing.rs` — share helpers with unified path, clippy cleanups.
- **Modify:** `src/processing/histogram.rs` — fuse min/max + binning passes, remove per-chunk allocations.
- **Modify:** `src/processing/channel_classification.rs` — remove SSE2 prefix matcher.
- **Modify:** `src/processing/layer_export.rs` — reuse `image_cache::compose_*` helpers; remove duplicated `compose_rgb_from_channels`.
- **Modify:** `src/io/metadata_traits.rs` — inline single-use `UnifiedLayerInfo` or delete file.
- **Modify:** `src/main.rs`, `src/ui/setup.rs`, `src/ui/file_handlers.rs` — replace ad-hoc `println!` with a single `log_info!`/`log_warn!` macro in `src/utils/logging.rs`.
- **No changes:** `ui/*.slint`, `Cargo.toml` (profiles already tuned), Windows icon code.

---

# Phase 1 — Clippy Auto-Fixes (Safe Baseline)

Intent: establish a clippy-clean baseline before any structural change, so later diffs stay focused.

### Task 1.1: Create baseline test run

**Files:**
- No code changes — commit only marks starting point.

- [ ] **Step 1: Confirm the workspace builds and tests pass**

Run:
```bash
cargo build
cargo test
```
Expected: both succeed. Record any pre-existing failures and treat them as out-of-scope.

- [ ] **Step 2: Capture clippy baseline**

Run:
```bash
cargo clippy --all-targets -- -D warnings 2>&1 | tee target/clippy-baseline.txt || true
```
Expected: ~41 warnings listed. File stored for diff.

- [ ] **Step 3: Commit baseline artefact**

```bash
git add target/clippy-baseline.txt 2>/dev/null || true
git commit --allow-empty -m "chore: baseline clippy + test state before optimization audit"
```

---

### Task 1.2: Apply safe clippy auto-fixes

**Files:**
- Modify: any file touched by `cargo clippy --fix` (expect 23 automated edits across `src/**/*.rs`).

- [ ] **Step 1: Apply auto-fixes on a clean tree**

Run:
```bash
cargo clippy --fix --allow-dirty --allow-staged --all-targets
```
Expected: "Fixed N warnings" summary. No compile errors.

- [ ] **Step 2: Re-run clippy and tests**

```bash
cargo clippy --all-targets
cargo test
```
Expected: remaining warnings (~18) are the non-auto-fixable subset. Tests pass.

- [ ] **Step 3: Smoke test the UI**

Run `cargo run --bin EXruster -- <path to a small .exr>`; open a file, change exposure, switch layers, close. No panics.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "refactor: apply cargo clippy --fix auto-suggestions (23 edits)"
```

---

### Task 1.3: Clean remaining clippy warnings manually

**Files:**
- Modify: `src/io/image_cache.rs` (line 306 and similar — 8-arg functions, `&PathBuf` → `&Path`).
- Modify: `src/ui/setup.rs:190, 235` (non-Send Arc usage notes).
- Modify: `src/processing/simd_processing.rs:385` (8-arg function).

- [ ] **Step 1: Change `&PathBuf` to `&Path` in image_cache.rs:334**

```rust
// BEFORE
pub fn extract_layers_info(path: &PathBuf) -> Result<Vec<LayerInfo>, ...>

// AFTER
pub fn extract_layers_info(path: &Path) -> Result<Vec<LayerInfo>, ...>
```
Update all callers accordingly (`Grep` for `extract_layers_info(`).

- [ ] **Step 2: Reduce 8-arg functions to struct arguments**

For each flagged function (`src/io/image_cache.rs:306`, `src/processing/simd_processing.rs:385`), introduce a local parameter struct:

```rust
struct ProcessRgbaArgs<'a> {
    rgba: &'a mut [f32],
    exposure: f32,
    gamma: f32,
    contrast: f32,
    saturation: f32,
    brightness: f32,
    tone_map_id: u8,
    use_srgb: bool,
}

fn process_rgba_chunk_optimized(args: ProcessRgbaArgs<'_>) { /* body unchanged */ }
```
Update call sites to build the struct inline.

- [ ] **Step 3: Run clippy + test**

```bash
cargo clippy --all-targets
cargo test
```
Expected: warning count drops to zero or to a documented residual list.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "refactor: resolve remaining clippy warnings (Path vs PathBuf, arg-count)"
```

---

# Phase 2 — Dead Code Removal

Intent: delete code not referenced by the binary. Each task is independently revertible.

### Task 2.1: Delete `src/utils/buffer_pool.rs`

**Files:**
- Delete: `src/utils/buffer_pool.rs`
- Modify: `src/utils/mod.rs`

- [ ] **Step 1: Confirm no production references**

Run:
```bash
grep -rn "buffer_pool\|BufferPool\|PooledBuffer" src/ --include='*.rs'
```
Expected: matches only inside `src/utils/buffer_pool.rs` itself and (possibly) `src/utils/mod.rs`.

- [ ] **Step 2: Remove module declaration**

Edit `src/utils/mod.rs`:
```rust
// BEFORE
pub mod buffer_pool;

// AFTER
// (line deleted)
```

- [ ] **Step 3: Delete the file**

```bash
git rm src/utils/buffer_pool.rs
```

- [ ] **Step 4: Build and test**

```bash
cargo build
cargo test
```
Expected: success. Unused-import warnings elsewhere would indicate a missed reference.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "chore: remove unused BufferPool module (dead code, tests-only refs)"
```

---

### Task 2.2: Remove `generate_thumbnails_gpu_raw` wrapper

**Files:**
- Modify: `src/io/thumbnails.rs` (lines ~81-91 and call sites)

- [ ] **Step 1: Locate all callers**

Run:
```bash
grep -rn "generate_thumbnails_gpu_raw" src/ --include='*.rs'
```

- [ ] **Step 2: Replace call sites with CPU variant**

At each caller, replace `generate_thumbnails_gpu_raw(args)` with `generate_thumbnails_cpu_raw(args)` directly (same signature).

- [ ] **Step 3: Delete the wrapper**

Delete the function body at `src/io/thumbnails.rs:81-91`.

- [ ] **Step 4: Build + test**

```bash
cargo build
cargo test
```

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "refactor: drop generate_thumbnails_gpu_raw alias (GPU path was removed)"
```

---

### Task 2.3: Remove dead tone-mapping safe-wrapper variants

**Files:**
- Modify: `src/processing/tone_mapping.rs` (lines ~301-400)

- [ ] **Step 1: Confirm wrappers are unused**

```bash
grep -rn "tone_map_and_gamma_simd_safe\|tone_map_and_gamma_safe" src/ --include='*.rs'
```
Expected: definitions only, no callers.

- [ ] **Step 2: Delete the two `*_safe` functions** from `src/processing/tone_mapping.rs`.

- [ ] **Step 3: Build + test**

```bash
cargo build
cargo test
```

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "chore: remove dead tone_map_and_gamma *_safe wrappers"
```

---

### Task 2.4: Remove `ToneMapModeId` wrapper

**Files:**
- Modify: `src/processing/tone_mapping.rs` (lines ~50-100 and call sites)

- [ ] **Step 1: Locate usage**

```bash
grep -rn "ToneMapModeId" src/ --include='*.rs'
```

- [ ] **Step 2: Replace the wrapper with direct `u8`/`ToneMapMode` usage**

Wherever a parameter is `ToneMapModeId`, switch to the underlying raw id (`u8`) and convert with the existing `ToneMapMode::from_id` helper. Delete the `ToneMapModeId` struct and its impl.

- [ ] **Step 3: Build + test + UI smoke**

```bash
cargo build
cargo test
cargo run --bin EXruster
```
Expected: tone-mapping dropdown still switches modes.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "refactor: remove ToneMapModeId wrapper, use ToneMapMode directly"
```

---

### Task 2.5: Sweep remaining `#[allow(dead_code)]` markers

**Files:**
- Modify: `src/io/fast_exr_metadata.rs`, `src/processing/histogram.rs`, `src/io/lazy_exr_loader.rs:48`, any file still flagged.

- [ ] **Step 1: List all occurrences**

```bash
grep -rn "#\[allow(dead_code)\]" src/ --include='*.rs'
```

- [ ] **Step 2: For each marker, verify whether the annotated item is truly unused**

Run the `grep` above, then for each item name: `grep -rn "<item>" src/` to find real callers. For anything unused, delete the item and the marker. For items that *are* used in some build configuration, remove only the marker and see whether compilation still passes.

- [ ] **Step 3: Build + test**

```bash
cargo build --features unified_simd
cargo build
cargo test
```

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "chore: remove dead fields/functions and their #[allow(dead_code)] markers"
```

---

# Phase 3 — Hot-Path Performance Fixes

Intent: remove redundant work from the per-frame / per-pixel code paths.

### Task 3.1: Reuse source `Arc` in `image_cache.rs:478`

**Files:**
- Modify: `src/io/image_cache.rs` around line 470-490

- [ ] **Step 1: Read the offending block**

Open `src/io/image_cache.rs:460-500`. Current code:
```rust
// line ~478
let channel_arc: Arc<[f32]> = Arc::from(layer.channel_data.as_slice());
```
This allocates a new `Arc<[f32]>` and copies the whole buffer.

- [ ] **Step 2: Change `layer.channel_data` storage to `Arc<[f32]>`**

Where `LayerChannelData` is declared (same file, near top), ensure the field is already `Arc<[f32]>`. If it is `Vec<f32>`, change to `Arc<[f32]>` in the struct and at its construction sites.

- [ ] **Step 3: Replace the copy with an `Arc::clone`**

```rust
// BEFORE
let channel_arc: Arc<[f32]> = Arc::from(layer.channel_data.as_slice());

// AFTER
let channel_arc: Arc<[f32]> = Arc::clone(&layer.channel_data);
```

- [ ] **Step 4: Build + test + load a large (>500 MB) EXR**

```bash
cargo run --release --bin EXruster -- <big.exr>
```
Expected: file opens; switch layers; no panics; memory growth noticeably smaller than before (optional: observe RSS in Task Manager).

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "perf: reuse Arc<[f32]> for layer channel data (avoid full-buffer copy)"
```

---

### Task 3.2: Hoist `exposure_multiplier` and `use_srgb` out of SIMD inner loop

**Files:**
- Modify: `src/processing/tone_mapping.rs` (lines 301-360)
- Modify: `src/processing/simd_processing.rs` call sites

- [ ] **Step 1: Promote frame-constants to function parameters**

In `tone_map_and_gamma_simd` (tone_mapping.rs near line 301), change signature to accept the precomputed scalar:

```rust
// BEFORE (partial)
pub fn tone_map_and_gamma_simd(
    input: &[f32],
    output: &mut [f32],
    exposure: f32,
    gamma: f32,
    tone_map_id: u8,
    use_srgb: bool,
) {
    // inner loop:
    let exposure_multiplier = 2.0_f32.powf(exposure);
    // ...
}

// AFTER
pub fn tone_map_and_gamma_simd(
    input: &[f32],
    output: &mut [f32],
    exposure_multiplier: f32,  // precomputed by caller
    gamma: f32,
    tone_map_id: u8,
    use_srgb: bool,
) {
    // inner loop now just uses exposure_multiplier directly
}
```

- [ ] **Step 2: Move the `powf` to the caller**

In `src/processing/simd_processing.rs` and any other caller, compute `let exposure_multiplier = 2.0_f32.powf(exposure);` once per frame, then pass it in.

- [ ] **Step 3: Hoist the `use_srgb` branch**

Inside `tone_map_and_gamma_simd`, split the loop into two: one that calls the sRGB OETF, one that doesn't. Decide which to run based on `use_srgb` *before* the loop:

```rust
if use_srgb {
    for chunk in chunks { /* ...srgb... */ }
} else {
    for chunk in chunks { /* ...pure gamma... */ }
}
```

- [ ] **Step 4: Build + test**

```bash
cargo build --release
cargo test --release
```

- [ ] **Step 5: Smoke-benchmark**

Load a 4K EXR, drag the exposure slider continuously for ~5 s. UI stays responsive; no visual change vs. before.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "perf: hoist exposure_multiplier and sRGB branch out of SIMD inner loop"
```

---

### Task 3.3: Fuse histogram min/max + binning pass

**Files:**
- Modify: `src/processing/histogram.rs` (lines 83-160)

- [ ] **Step 1: Replace two `par_iter` passes with one fold**

Current code does pass A (min/max) and then pass B (bin fill). Combine into one `rayon` parallel fold that carries `(min, max, local_bins)` per chunk and reduces pairwise:

```rust
let (min, max, bins) = pixels
    .par_chunks(CHUNK)
    .fold(
        || (f32::INFINITY, f32::NEG_INFINITY, vec![0u32; bin_count]),
        |(mut lo, mut hi, mut local), chunk| {
            for &v in chunk {
                if v.is_finite() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            // second walk (still in-cache for this chunk) for binning:
            let range = (hi - lo).max(1e-6);
            for &v in chunk {
                if v.is_finite() {
                    let i = (((v - lo) / range) * (bin_count as f32 - 1.0)) as usize;
                    local[i.min(bin_count - 1)] += 1;
                }
            }
            (lo, hi, local)
        },
    )
    .reduce(
        || (f32::INFINITY, f32::NEG_INFINITY, vec![0u32; bin_count]),
        |(a_lo, a_hi, mut a_bins), (b_lo, b_hi, b_bins)| {
            for (a, b) in a_bins.iter_mut().zip(b_bins) {
                *a += b;
            }
            (a_lo.min(b_lo), a_hi.max(b_hi), a_bins)
        },
    );
```

Note: binning inside the chunk uses the chunk-local min/max, which is *wrong* for a final histogram. If fusing changes semantics, fall back to keeping the global-range binning pass but reuse a thread-local `Vec<u32>` via `rayon`'s `fold` (eliminate the per-chunk `vec![0u32; bin_count]` allocation at least).

- [ ] **Step 2: Add a unit test to pin the behavior**

In `src/processing/histogram.rs` `#[cfg(test)]`:
```rust
#[test]
fn histogram_matches_reference() {
    let data: Vec<f32> = (0..1_000).map(|i| i as f32 / 1000.0).collect();
    let h = compute_histogram(&data, 10);
    assert_eq!(h.bins.iter().sum::<u32>(), 1_000);
    assert!(h.min <= 0.0 && h.max >= 0.999);
}
```

- [ ] **Step 3: Run it**

```bash
cargo test histogram_matches_reference -- --nocapture
```

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "perf: reuse per-thread bin buffers in histogram (drop per-chunk alloc)"
```

---

### Task 3.4: Fix fake LRU in `lazy_exr_loader.rs`

**Files:**
- Modify: `src/io/lazy_exr_loader.rs` (lines ~180-200)

- [ ] **Step 1: Replace the custom map with the `lru` crate**

Top of file:
```rust
use lru::LruCache;
use std::num::NonZeroUsize;
```

Replace the ad-hoc `HashMap<Key, Val>` + `cache.keys().next()` eviction with:
```rust
cache: Mutex<LruCache<Key, Val>>,
```
and in construction:
```rust
cache: Mutex::new(LruCache::new(NonZeroUsize::new(capacity).unwrap())),
```

- [ ] **Step 2: Update call sites**

- `get(key)` → `cache.lock().unwrap().get(&key).cloned()`
- `put(key, val)` → `cache.lock().unwrap().put(key, val)`

Remove the comment admitting the fake LRU.

- [ ] **Step 3: Build + test + run the app on a dataset with many layers**

```bash
cargo build
cargo test
```

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "perf: replace fake LRU in lazy_exr_loader with lru::LruCache"
```

---

### Task 3.5: Preallocate `Arc<Vec<String>>` in lazy loader

**Files:**
- Modify: `src/io/lazy_exr_loader.rs` (line ~131)

- [ ] **Step 1: Replace `Arc::make_mut(&mut arc).push(s)` pattern**

Build the full `Vec<String>` as a plain `Vec` first, then wrap in `Arc::new(...)` at the end:

```rust
// BEFORE
let mut names: Arc<Vec<String>> = Arc::new(Vec::new());
for entry in entries {
    Arc::make_mut(&mut names).push(entry.name.clone());
}

// AFTER
let mut names: Vec<String> = Vec::with_capacity(entries.len());
for entry in entries {
    names.push(entry.name.clone());
}
let names = Arc::new(names);
```

- [ ] **Step 2: Build + test**

```bash
cargo build
cargo test
```

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "perf: build layer-name Vec before Arc::new (avoid Arc::make_mut growth)"
```

---

# Phase 4 — De-duplication

### Task 4.1: Merge `compose_rgb_from_channels` across modules

**Files:**
- Modify: `src/io/image_cache.rs` (make `compose_composite_from_channels` accept a mode enum).
- Modify: `src/processing/layer_export.rs` (delete its copy, call the one from `image_cache`).

- [ ] **Step 1: Identify the divergent parameters**

Diff the two functions (`image_cache::compose_composite_from_channels` vs `layer_export::compose_rgb_from_channels`). Expect a small difference: output channel count or output type.

- [ ] **Step 2: Extract a shared function**

Move the unified body into `src/processing/composite.rs` (new file) or into `src/processing/image_processing.rs`, with:
```rust
pub enum CompositeMode { Rgb, Rgba }
pub fn compose_from_channels(
    channels: &[ChannelRef<'_>],
    mode: CompositeMode,
    out: &mut [f32],
) { /* unified */ }
```

- [ ] **Step 3: Delete both old copies; call the shared function from the two original call sites**

- [ ] **Step 4: Build + test + export a layer**

Open the app, export a layer to PNG, compare with a pre-refactor export pixel-for-pixel:
```bash
cargo run --release --bin EXruster -- <file.exr>
```
Save layer as PNG twice (before + after) and diff.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "refactor: merge compose_rgb_from_channels and compose_composite_from_channels"
```

---

### Task 4.2: Inline `UnifiedLayerInfo` single-use abstraction

**Files:**
- Modify: `src/io/metadata_traits.rs`
- Modify: `src/io/image_cache.rs` (`find_best_layer`)

- [ ] **Step 1: Inline the conversion**

If `UnifiedLayerInfo` is used only inside `find_best_layer`, change `find_best_layer` to operate directly on the native `LayerInfo` type. Delete the trait and adapter.

- [ ] **Step 2: Delete `src/io/metadata_traits.rs`** if nothing else references it.

- [ ] **Step 3: Remove module from `src/io/mod.rs`.**

- [ ] **Step 4: Build + test + smoke**

```bash
cargo build
cargo test
cargo run --bin EXruster
```

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "refactor: inline UnifiedLayerInfo (single-use abstraction)"
```

---

### Task 4.3: Delete SSE2 prefix matcher

**Files:**
- Modify: `src/processing/channel_classification.rs` (lines 226-269)

- [ ] **Step 1: Replace callers with `str::starts_with`**

Every call site of the custom SSE2 prefix matcher currently goes through `determine_channel_group_ultra_fast` — swap for the standard library version:

```rust
// BEFORE
if sse2_prefix_eq(name.as_bytes(), b"diffuse") { ... }

// AFTER
if name.starts_with("diffuse") { ... }
```

- [ ] **Step 2: Delete the SSE2 helper function and any unsafe `#[target_feature]` attribute.**

- [ ] **Step 3: Build + test**

```bash
cargo build --release
cargo test
```
Expected: classification behavior identical. If there's a benchmark for channel classification, it should remain under 1 ms for realistic channel counts.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "chore: drop hand-rolled SSE2 prefix matcher (starts_with is sufficient)"
```

---

# Phase 5 — Code Quality

### Task 5.1: Consolidate logging

**Files:**
- Modify: `src/utils/logging.rs` (already exists — extend it).
- Modify: `src/main.rs`, `src/ui/setup.rs`, `src/ui/file_handlers.rs`, any file using `println!`/`eprintln!` for runtime messages.

- [ ] **Step 1: Define macros in `src/utils/logging.rs`**

```rust
#[macro_export]
macro_rules! log_info  { ($($t:tt)*) => { eprintln!("[INFO] {}", format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_warn  { ($($t:tt)*) => { eprintln!("[WARN] {}", format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { eprintln!("[ERROR] {}", format_args!($($t)*)) } }
```

(Keep this minimal — adding a full logger crate is out of scope.)

- [ ] **Step 2: Find offending call sites**

```bash
grep -rn "println!\|eprintln!" src/ --include='*.rs' | grep -v -E 'logging\.rs|tests?\.rs|#\[cfg\(test\)\]'
```

- [ ] **Step 3: Replace**

Swap each runtime `println!(...)` with `log_info!(...)` and each `eprintln!(...)` with `log_warn!` or `log_error!` as appropriate. Leave test-only prints alone.

- [ ] **Step 4: Build + test + run**

```bash
cargo build
cargo test
cargo run --bin EXruster
```
Expected: console output now has `[INFO] / [WARN] / [ERROR]` prefixes.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "chore: route runtime messages through log_info/log_warn/log_error macros"
```

---

### Task 5.2: Final audit sweep

**Files:**
- Check-only (no edits unless needed).

- [ ] **Step 1: Clippy strict**

```bash
cargo clippy --all-targets -- -D warnings
```
Expected: zero warnings.

- [ ] **Step 2: Full test**

```bash
cargo test
cargo test --features unified_simd
```

- [ ] **Step 3: Release build size check**

```bash
cargo build --release
ls -l target/release/EXruster*
```
Record size before/after the whole plan in the final commit message.

- [ ] **Step 4: Manual regression smoke test**

Open a small EXR (<50 MB), a medium (200 MB), and a large (>700 MB) file.
For each: switch layers, drag exposure slider, toggle sRGB, toggle tone-map mode, export a PNG. No panics, no visual changes vs. pre-plan screenshots.

- [ ] **Step 5: Commit**

```bash
git commit --allow-empty -m "chore: optimization audit complete — zero clippy warnings, all regressions clean"
```

---

## Rollback Plan

Each task is one commit. To revert any phase, `git revert <range>` — no schema migrations, no external state. The only delete that can't be partial is `buffer_pool.rs` (Task 2.1) — revert restores the file.

## Risk Notes

- **Task 3.2 (hoist exposure)** and **Task 3.1 (Arc reuse)** touch hot paths — the smoke test after each is non-negotiable; if the image looks wrong, revert immediately.
- **Task 3.3 (histogram fuse)** can change results if semantics drift. The unit test at Step 2 guards this — if it ever fails, stop and re-pair the binning range with the global min/max.
- **Task 4.1 (compose merge)** risks changing exported pixel values. The pixel diff at Step 4 must be byte-identical.
- **Task 2.4 (delete ToneMapModeId)** is a structural change — if any downstream callback relies on the wrapper's `Display` impl, replace with `ToneMapMode::name()`.
