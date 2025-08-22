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
cargo run --bin EXruster_nightly

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

# Run specific GPU tests with timeout
timeout 45s cargo test test_gpu_thumbnail_generation -- --nocapture

# Run with SIMD features
cargo check --features unified_simd
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
- `image_cache.rs` - Basic image caching
- `full_exr_cache.rs` - Advanced EXR file caching with metadata
- `lazy_exr_loader.rs` - Lazy loading for performance
- `thumbnails.rs` - Thumbnail generation and management
- `file_operations.rs` - File system operations
- `exr_metadata.rs` - EXR metadata extraction and handling

**`src/processing/`** - Image processing pipeline
- `image_processing.rs` - Core image processing algorithms
- `simd_processing.rs` - SIMD-optimized processing routines
- `tone_mapping.rs` - HDR tone mapping algorithms (Reinhard, etc.)
- `color_processing.rs` - Color space conversions and corrections
- `histogram.rs` - Histogram generation and analysis
- `pipeline.rs` - Processing pipeline coordination

**`src/ui/`** - User interface components
- `ui_handlers.rs` - Main UI event handlers and callbacks
- `state.rs` - Application state management
- `setup.rs` - UI initialization and callback setup
- `layers.rs` - Layer tree handling for EXR files
- `image_controls.rs` - Image parameter controls (exposure, gamma)
- `progress.rs` - Progress reporting system

**`src/utils/`** - Utility modules
- `buffer_pool.rs` - Memory buffer pooling for performance
- `cache.rs` - Generic caching utilities
- `error_handling.rs` - Error handling and reporting
- `logging.rs` - Logging infrastructure
- `progress.rs` - Progress tracking utilities

### Key Technologies

- **Slint** - Modern UI framework with declarative syntax
- **exr** - EXR file format handling
- **rayon** - Parallel processing
- **glam** - Linear algebra and SIMD math
- **tokio** - Async runtime
- **lru** - LRU caching

### SIMD Support

The project includes SIMD optimizations that can be enabled with:
```bash
cargo build --features unified_simd
```

SIMD code is located in `src/processing/simd_processing.rs` and uses Rust's portable SIMD feature.

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

**Memory Management:** The application uses buffer pooling (`BufferPool`) for efficient memory management during image processing.

**Error Handling:** Comprehensive error handling with custom error types and recovery mechanisms.

**Threading:** Rayon thread pool is configured to use `num_cpus - 1` threads, leaving one core for UI responsiveness.

**Caching Strategy:** Multi-level caching system:
- `ImageCache` for basic image data
- `FullExrCache` for complete EXR files with metadata
- LRU eviction policies for memory management

## Common Development Tasks

**Adding new image processing algorithms:** Add to `src/processing/` and integrate through the pipeline system.

**UI modifications:** Edit `.slint` files in `ui/` directory, then rebuild to regenerate Rust bindings.

**Performance optimization:** Focus on `src/processing/simd_processing.rs` for SIMD implementations and `src/utils/buffer_pool.rs` for memory optimization.

**Adding file format support:** Extend `src/io/file_operations.rs` and related caching systems.