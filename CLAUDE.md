# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

EXRuster is an EXR (High Dynamic Range) image viewer and processor written in Rust with a Slint UI framework. The application supports advanced image processing features including tone mapping, SIMD optimizations, layer handling, and real-time parameter adjustments.

## Build Commands

**Primary build commands:**
```bash
# Standard development build
cargo build

# Release build with optimizations
cargo build --release

# Run the application
cargo run --bin EXruster

# Alternative build using Python script (includes auto-detection and additional features)
python build.py --release
```

**Development profiles available:**
- `dev` - Standard development with opt-level=1
- `dev-fast` - Faster compilation with opt-level=0, no debug info
- `release` - Full optimization with LTO and stripping
- `release-with-debug` - Release build but with debug symbols

**Testing:**
```bash
# Run all tests
cargo test

# Run a single test
cargo test --bin EXruster <test_name> -- --nocapture
```

**Code quality:**
```bash
# Format code
cargo fmt

# Lint code  
cargo clippy

# Check without building
cargo check
```

## Architecture

The codebase follows a modular architecture organized into distinct functional areas:

### Core Modules

**`src/io/`** - File I/O and caching systems
- `image_cache.rs` - The loaded image: composite pixels, layer/channel switching, render API
- `full_exr_cache.rs` - Whole-file EXR cache in RAM (all layers, all channels)
- `lazy_exr_loader.rs` - On-demand layer loading for large files (>700 MB), mmap above 500 MB
- `selective_layer_reader.rs` - Decode a single named layer instead of the whole file
- `thumbnails.rs` - Thumbnail generation + LRU cache
- `exr_metadata.rs` - Meta tab presenter
- `fast_exr_metadata.rs` - Raw header parser used by exr_metadata (bails on multipart)
- `hdr_loader.rs` - Radiance HDR decoding
- `folder_tree.rs`, `file_operations.rs`, `progress_reader.rs`

**`src/processing/`** - Image processing pipeline
- `image_processing.rs` - Per-pixel scalar pipeline
- `simd_processing.rs` - SIMD tone-map/gamma kernels (portable_simd)
- `tone_mapping.rs` - Tone-map curves (ACES, Reinhard, Filmic, Hable), sRGB LUT
- `color_processing.rs` - Chromaticities -> sRGB matrix, cached (keyed on path+layer+mtime)
- `histogram.rs` - Histogram generation and analysis
- `channel_classification.rs` - Group channels/layers by config
- `layer_export.rs` - PNG16 (display-referred) and 32-bit float TIFF (scene-referred) export

**`src/ui/`** - User interface components
- `ui_handlers.rs` - Main UI event handlers and callbacks
- `state.rs` - Application state management
- `setup.rs` - UI initialization and callback setup
- `file_handlers.rs` - Opening EXR/HDR files, layer model construction
- `layers.rs` - Layer tree handling for EXR files
- `image_controls.rs` - Exposure/gamma controls + background preview rendering
- `browser_handlers.rs`, `thumbnails.rs` - Folder tree and thumbnail browser
- `export_handlers.rs` - Export callbacks
- `progress.rs` - Progress reporting system

**`src/utils/`** - Utility modules
- `channel_config.rs` - Channel-group configuration (JSON)
- `error_handling.rs` - Error handling and reporting
- `logging.rs` - Logging infrastructure
- `progress.rs` - Progress tracking utilities

### Key Technologies

- **Slint** - Modern UI framework with declarative syntax
- **exr** - EXR file format handling
- **rayon** - Parallel processing
- **glam** - Linear algebra and SIMD math
- **lru** - LRU caching
- **memmap2** - Memory-mapping for large files (>500 MB, lazy mode)

### SIMD Support

SIMD is always on. The kernels live in `src/processing/simd_processing.rs` and use
Rust's portable SIMD feature (hence the nightly toolchain).

### Platform-Specific Code

Windows-specific functionality is in `src/platform/platform_win.rs` and includes:
- Runtime window icon setting
- Windows API integration

### UI System

The UI is defined in `.slint` files in the `ui/` directory:
- `appwindow.slint` - Main application window
- `components/` - Reusable UI components
- `console_window.slint` - Debug console
- `meta_window.slint` - Metadata display

## Development Notes

**Rust Toolchain:** Uses nightly Rust channel with portable SIMD features enabled.

**Threading rule (important):** All image processing and file I/O runs on the rayon
pool. The Slint event loop only assigns `slint::Image`s and properties — it must
never tone-map, decode, or touch the disk. Renders go through
`image_cache::render_to_buffer` + `RenderSnapshot`: workers produce a
`SharedPixelBuffer` (which is `Send`, unlike `slint::Image`), and the event loop
just wraps it. Preview renders carry a generation counter so a slow frame cannot
overwrite a newer one.

**Memory Management:** `ImageCache::raw_pixels` is an `Arc<Vec<f32>>`, so a background
render can hold the pixels without copying them or holding the app-state lock;
mutations go through `Arc::make_mut`.

**Error Handling:** Comprehensive error handling with custom error types and recovery mechanisms.

**Threading:** Rayon thread pool is configured to use `num_cpus - 1` threads, leaving one core for UI responsiveness.

**Caching Strategy:** Multi-level caching system:
- `ImageCache` holds the current composite + the current layer's planar channels
- `FullExrCacheData` (full mode) or `LazyExrLoader` (lazy mode) behind `ExrDataSource`
- LRU caches for thumbnails (keyed on path+mtime+params) and colour matrices
  (keyed on path+layer+mtime)

## Common Development Tasks

**Adding new image processing algorithms:** Add to `src/processing/` and integrate through the pipeline system.

**UI modifications:** Edit `.slint` files in `ui/` directory, then rebuild to regenerate Rust bindings.

**Performance optimization:** Focus on `src/processing/simd_processing.rs` for the SIMD
kernels. Before adding work to a UI callback, check the threading rule above.

**Adding file format support:** Extend `src/io/file_operations.rs` and related caching systems.