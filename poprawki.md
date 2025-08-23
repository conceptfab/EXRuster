> chce dodac do kodu funkcję exportu warstw w róznych konfiguracjach, np tylko base, tylko 
  obiekty sceny, tylko warstwy techniczne, tylko światła, wszystkie, lub wybrane wg dowolnego 
  schematu. przygotuj wysoko wydajny kod. na początek tylko tylko dla warstwy Base. Pliki 
  wynikowe to PNG 16 bit i TIFF 16/32 bit

● I'll help you implement high-performance layer export functionality, starting with Base layer
  export to PNG 16-bit and TIFF 16/32-bit formats. Let me first analyze the codebase structure
  and then create a comprehensive plan.

● Update Todos
  ⎿  ☐ Analyze existing layer handling and EXR structure
     ☐ Design export module architecture for different layer configurations
     ☐ Implement Base layer export functionality
     ☐ Add PNG 16-bit export support
     ☐ Add TIFF 16/32-bit export support
     ☐ Integrate export functionality into UI
     ☐ Add performance optimizations and SIMD support

● Update Todos
  ⎿  ☐ Analyze existing layer handling and EXR structure
     ☐ Design export module architecture for different layer configurations
     ☐ Implement Base layer export functionality
     ☐ Add PNG 16-bit export support
     ☐ Add TIFF 16/32-bit export support
     ☐ Integrate export functionality into UI
     ☐ Add performance optimizations and SIMD support

● Read(src\ui\layers.rs)
  ⎿  Read 301 lines (ctrl+r to expand)

● Search(pattern: "src/**/*.rs")
  ⎿  Found 33 files (ctrl+r to expand)

● Read(Cargo.toml)
  ⎿  Read 102 lines (ctrl+r to expand)

● Read(src\io\image_cache.rs)
  ⎿  Read 644 lines (ctrl+r to expand)

● Read(src\processing\image_processing.rs)
  ⎿  Read 66 lines (ctrl+r to expand)

● Update Todos
  ⎿  ☒ Analyze existing layer handling and EXR structure
     ☐ Design export module architecture for different layer configurations
     ☐ Implement Base layer export functionality
     ☐ Add PNG 16-bit export support
     ☐ Add TIFF 16/32-bit export support
     ☐ Integrate export functionality into UI
     ☐ Add performance optimizations and SIMD support

● Based on the codebase analysis, I'll create a high-performance layer export module. The
  architecture will support different layer configurations and export formats as requested.

  Write(src\processing\layer_export.rs)
  ⎿  Wrote 1 lines to src\processing\layer_export.rs
     use anyhow::Result;
     use std::path::PathBuf;
     use std::sync::Arc;
     use rayon::prelude::*;
     use crate::io::image_cache::{ImageCache, LayerInfo, LayerChannels};
     use crate::io::full_exr_cache::FullExrCacheData;
     use crate::processing::tone_mapping::{tone_map_and_gamma, ToneMapMode};
     use crate::utils::buffer_pool::BufferPool;
     use std::sync::OnceLock;

  Update Todos
  ⎿  ☒ Analyze existing layer handling and EXR structure
     ☒ Design export module architecture for different layer configurations
     ☒ Implement Base layer export functionality
     ☒ Add PNG 16-bit export support
     ☐ Add TIFF 16/32-bit export support
     ☐ Integrate export functionality into UI
     ☐ Add performance optimizations and SIMD support

● Now let me complete the TIFF 32-bit float support and add the module to the processing module:

● Read(src\processing\mod.rs)
  ⎿  Read 8 lines (ctrl+r to expand)

  Update(src\processing\mod.rs)
  ⎿  Updated src\processing\mod.rs with 1 addition
        4    pub mod simd_processing;
        5    pub mod histogram;
        6    pub mod channel_classification;
        7 +  pub mod layer_export;
        8    
        9    // Re-export key types and functions used by other modules
       10   \ No newline at end of file

● Now let me complete the TIFF 32-bit float implementation:

  Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 26 additions and 4 removals
       504        }
       505    
       506        /// Save as TIFF 32-bit float
       507 -      fn save_tiff32_float(&self, _pixels: &ProcessedPixels, _output_path: &PathBuf) 
     -> Result<()> {
           - -> Result<()> {
       508 -          // TODO: Implement 32-bit float TIFF export using tiff crate directly
       509 -          // The image crate doesn't support 32-bit float TIFF export directly
       510 -          anyhow::bail!("TIFF 32-bit float export not yet implemented")
       507 +      fn save_tiff32_float(&self, pixels: &ProcessedPixels, output_path: &PathBuf)       
     -> Result<()> {
           +  Result<()> {
       508 +          use std::fs::File;
       509 +          use tiff::encoder::{TiffEncoder, colortype};
       510 +
       511 +          // Convert u16 data back to f32 for 32-bit float export
       512 +          let float_data: Vec<f32> = pixels.data
       513 +              .par_iter()
       514 +              .map(|&value| (value as f32) / 65535.0)
       515 +              .collect();
       516 +
       517 +          let file = File::create(output_path)?;
       518 +          let mut tiff = TiffEncoder::new(file)?;
       519 +
       520 +          let color_type = if pixels.channels == 4 {
       521 +              colortype::RGBA32Float
       522 +          } else {
       523 +              colortype::RGB32Float
       524 +          };
       525 +
       526 +          tiff.write_image::<colortype::RGB32Float>(
       527 +              pixels.width,
       528 +              pixels.height,
       529 +              &float_data,
       530 +          ).map_err(|e| anyhow::anyhow!("Failed to write TIFF: {}", e))?;
       531 +  
       532 +          Ok(())
       533        }
       534    }
       535

● Now let me add a more efficient implementation for 32-bit float processing and fix the TIFF
  implementation:

  Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 9 additions and 2 removals
       273            let rgb_pixels = self.compose_rgb_from_channels(layer_channels);
       274
       275            // Apply tone mapping and gamma correction in parallel
       276 +          let has_alpha = layer_channels.channel_names.iter().any(|name| {
       277 +              let n = name.to_ascii_uppercase();
       278 +              n == "A" || n.starts_with("ALPHA")
       279 +          });
       280 +
       281 +          let channels = if has_alpha { 4 } else { 3 };
       282 +
       283            let processed_data = match layer_channels.channel_names.len() {
       284                1 => {
       285                    // Grayscale channel - expand to RGB
     ...
       280                }
       281                _ => {
       282                    // RGB/RGBA channels - full color processing
       283 -                  self.process_color_pixels(&rgb_pixels, pixel_count)?
       283 +                  self.process_color_pixels(&rgb_pixels, pixel_count, has_alpha)?        
       284                }
       285            };
       286
     ...
       288                data: processed_data,
       289                width: layer_channels.width,
       290                height: layer_channels.height,
       291 -              channels: if layer_channels.channel_names.len() >= 4 { 4 } else { 3 },     
       291 +              channels,
       292            })
       293        }
       294

  Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 1 addition and 2 removals
       379        }
       380    
       381        /// Process color pixels with tone mapping and color correction
       382 -      fn process_color_pixels(&self, pixels: &[f32], pixel_count: usize) -> 
     Result<Vec<u16>> {
           - Result<Vec<u16>> {
       383 -          let has_alpha = pixels.len() == pixel_count * 4;
       382 +      fn process_color_pixels(&self, pixels: &[f32], pixel_count: usize, has_alpha: 
     bool) -> Result<Vec<u16>> {
           + bool) -> Result<Vec<u16>> {
       383            let output_channels = if has_alpha { 4 } else { 3 };
       384            let mut processed = Vec::with_capacity(pixel_count * output_channels);
       385

● Now let me fix the TIFF 32-bit float implementation properly:

  Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 24 additions and 11 removals
       523            let file = File::create(output_path)?;
       524            let mut tiff = TiffEncoder::new(file)?;
       525
       526 -          let color_type = if pixels.channels == 4 {
       527 -              colortype::RGBA32Float
       528 -          } else {
       529 -              colortype::RGB32Float
       530 -          };
       531 -
       532 -          tiff.write_image::<colortype::RGB32Float>(
       533 -              pixels.width,
       534 -              pixels.height,
       535 -              &float_data,
       536 -          ).map_err(|e| anyhow::anyhow!("Failed to write TIFF: {}", e))?;
       526 +          match pixels.channels {
       527 +              4 => {
       528 +                  tiff.write_image::<colortype::RGBA32Float>(
       529 +                      pixels.width,
       530 +                      pixels.height,
       531 +                      &float_data,
       532 +                  ).map_err(|e| anyhow::anyhow!("Failed to write RGBA TIFF: {}",         
     e))?;
           + e))?;
       533 +              }
       534 +              3 => {
       535 +                  tiff.write_image::<colortype::RGB32Float>(
       536 +                      pixels.width,
       537 +                      pixels.height,
       538 +                      &float_data,
       539 +                  ).map_err(|e| anyhow::anyhow!("Failed to write RGB TIFF: {}", 
     e))?;
       540 +              }
       541 +              1 => {
       542 +                  tiff.write_image::<colortype::Gray32Float>(
       543 +                      pixels.width,
       544 +                      pixels.height,
       545 +                      &float_data,
       546 +                  ).map_err(|e| anyhow::anyhow!("Failed to write grayscale TIFF:         
     {}",
           +  e))?;
       547 +              }
       548 +              _ => anyhow::bail!("Unsupported channel count for TIFF: {}", 
           + pixels.channels)
       549 +          }
       550
       551            Ok(())
       552        }

  Update Todos
  ⎿  ☒ Analyze existing layer handling and EXR structure
     ☒ Design export module architecture for different layer configurations
     ☒ Implement Base layer export functionality
     ☒ Add PNG 16-bit export support
     ☒ Add TIFF 16/32-bit export support
     ☐ Integrate export functionality into UI
     ☐ Add performance optimizations and SIMD support

● Now let me add UI integration by creating handler functions and updating the UI handlers:

● Read(src\ui\ui_handlers.rs)
  ⎿  Read 50 lines (ctrl+r to expand)

● Read(src\ui\ui_handlers.rs)
  ⎿  Read 18 lines (ctrl+r to expand)

● Now let me create export handler functions:

  Write(src\ui\export_handlers.rs)
  ⎿  Wrote 1 lines to src\ui\export_handlers.rs
     use slint::{Weak, ComponentHandle, SharedString};
     use std::sync::{Arc, Mutex};
     use std::path::PathBuf;
     use anyhow::Result;
     use crate::AppWindow;
     use crate::processing::layer_export::{LayerExporter, LayerExportConfig, ExportFormat,
     ExportParams};
     use crate::processing::tone_mapping::ToneMapMode;
     use crate::io::full_exr_cache::FullExrCacheData;
     use crate::io::image_cache::LayerInfo;
     use crate::ui::ui_handlers::{push_console, lock_or_recover, ConsoleModel, FullExrCache,
     CurrentFilePathType};

  Update(src\ui\mod.rs)
  ⎿  Error editing file

● Read(src\ui\mod.rs)
  ⎿  Read 19 lines (ctrl+r to expand)

  Update(src\ui\mod.rs)
  ⎿  Updated src\ui\mod.rs with 1 addition
        6    pub mod thumbnails;
        7    pub mod file_handlers;
        8    pub mod setup;
        9 +  pub mod export_handlers;
       10    
       11    // Essential re-exports used by main.rs and internal modules
       12    pub use ui_handlers::{

  Update(src\ui\mod.rs)
  ⎿  Updated src\ui\mod.rs with 4 additions
       14        handle_parameter_changed_throttled, load_thumbnails_for_directory,
       15        update_preview_image, ImageCacheType, CurrentFilePathType, FullExrCache, 
            ThrottledUpdate
       16    };
       17 +  pub use export_handlers::{
       18 +      handle_export_base_layer, handle_export_layers, export_base_layer_png16, 
       19 +      export_base_layer_tiff16, export_base_layer_tiff32_float, UiExportConfig
       20 +  };
       21    pub use layers::{handle_layer_tree_click, toggle_all_layer_groups};
       22    pub use state::{SharedUiState, create_shared_state};
       23    pub use setup::{setup_ui_callbacks};

  Update Todos
  ⎿  ☒ Analyze existing layer handling and EXR structure
     ☒ Design export module architecture for different layer configurations
     ☒ Implement Base layer export functionality
     ☒ Add PNG 16-bit export support
     ☒ Add TIFF 16/32-bit export support
     ☒ Integrate export functionality into UI
     ☐ Add performance optimizations and SIMD support

● Now let me add SIMD optimizations to the layer export functionality:

● Read(src\processing\simd_processing.rs)
  ⎿  Read 100 lines (ctrl+r to expand)

● Let me add SIMD export optimizations to the layer export module:

  Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 88 additions and 20 removals
       378            Ok(processed)
       379        }
       380    
       381 -      /// Process color pixels with tone mapping and color correction
       381 +      /// Process color pixels with tone mapping and color correction (SIMD 
     optimized)
           + optimized)
       382        fn process_color_pixels(&self, pixels: &[f32], pixel_count: usize, has_alpha: 
             bool) -> Result<Vec<u16>> {
       383            let output_channels = if has_alpha { 4 } else { 3 };
       384            let mut processed = Vec::with_capacity(pixel_count * output_channels);
       385
       386 -          processed.extend(
       387 -              pixels.par_chunks_exact(4)
       388 -                  .flat_map(|chunk| {
       389 -                      let (r, g, b) = tone_map_and_gamma(
       390 -                          chunk[0],
       391 -                          chunk[1],
       392 -                          chunk[2],
       393 -                          self.export_params.exposure,
       386 +          // Use SIMD-optimized processing when feature is enabled
       387 +          #[cfg(feature = "unified_simd")]
       388 +          {
       389 +              processed.extend(self.process_color_pixels_simd_optimized(pixels, 
     has_alpha));
           + has_alpha));
       390 +          }
       391 +
       392 +          #[cfg(not(feature = "unified_simd"))]
       393 +          {
       394 +              processed.extend(
       395 +                  pixels.par_chunks_exact(4)
       396 +                      .flat_map(|chunk| {
       397 +                          let (r, g, b) = tone_map_and_gamma(
       398 +                              chunk[0],
       399 +                              chunk[1],
       400 +                              chunk[2],
       401 +                              self.export_params.exposure,
       402 +                              self.export_params.gamma,
       403 +                              self.export_params.tonemap_mode,
       404 +                          );
       405 +
       406 +                          let r_u16 = (r.clamp(0.0, 1.0) * 65535.0).round() as u16;      
       407 +                          let g_u16 = (g.clamp(0.0, 1.0) * 65535.0).round() as u16;      
       408 +                          let b_u16 = (b.clamp(0.0, 1.0) * 65535.0).round() as u16;      
       409 +
       410 +                          if has_alpha {
       411 +                              let a_u16 = (chunk[3].clamp(0.0, 1.0) * 
           + 65535.0).round() as u16;
       412 +                              vec![r_u16, g_u16, b_u16, a_u16]
       413 +                          } else {
       414 +                              vec![r_u16, g_u16, b_u16]
       415 +                          }
       416 +                      })
       417 +              );
       418 +          }
       419 +
       420 +          Ok(processed)
       421 +      }
       422 +  
       423 +      #[cfg(feature = "unified_simd")]
       424 +      /// SIMD-optimized color pixel processing
       425 +      fn process_color_pixels_simd_optimized(&self, pixels: &[f32], has_alpha: bool)     
           + -> Vec<u16> {
       426 +          use std::simd::{f32x4, Simd, SimdFloat, SimdUint, u16x4};
       427 +
       428 +          let exposure_multiplier = 2.0f32.powf(self.export_params.exposure);
       429 +          let gamma_inv = 1.0 / self.export_params.gamma;
       430 +
       431 +          // Process pixels in SIMD chunks of 4
       432 +          pixels.par_chunks_exact(16) // 4 pixels * 4 channels = 16 floats
       433 +              .flat_map(|chunk_16| {
       434 +                  // Load 4 RGBA pixels at once
       435 +                  let pixel_data: [f32x4; 4] = [
       436 +                      f32x4::from_slice(&chunk_16[0..4]),   // Pixel 0: RGBA
       437 +                      f32x4::from_slice(&chunk_16[4..8]),   // Pixel 1: RGBA
       438 +                      f32x4::from_slice(&chunk_16[8..12]),  // Pixel 2: RGBA
       439 +                      f32x4::from_slice(&chunk_16[12..16]), // Pixel 3: RGBA
       440 +                  ];
       441 +
       442 +                  let mut results = Vec::with_capacity(if has_alpha { 16 } else { 12     
           + });
       443 +
       444 +                  for pixel_rgba in pixel_data {
       445 +                      // Apply exposure
       446 +                      let exposed = pixel_rgba * f32x4::splat(exposure_multiplier);      
       447 +
       448 +                      // Extract RGB channels for tone mapping
       449 +                      let r = exposed[0];
       450 +                      let g = exposed[1];
       451 +                      let b = exposed[2];
       452 +                      let a = exposed[3];
       453 +
       454 +                      // Apply tone mapping (scalar for now, could be vectorized         
           + further)
       455 +                      let (tone_r, tone_g, tone_b) = tone_map_and_gamma(
       456 +                          r, g, b,
       457 +                          0.0, // exposure already applied
       458                            self.export_params.gamma,
       459                            self.export_params.tonemap_mode,
       460                        );
       461
       462 -                      let r_u16 = (r.clamp(0.0, 1.0) * 65535.0).round() as u16;
       463 -                      let g_u16 = (g.clamp(0.0, 1.0) * 65535.0).round() as u16;
       464 -                      let b_u16 = (b.clamp(0.0, 1.0) * 65535.0).round() as u16;
       462 +                      // Convert to u16
       463 +                      let r_u16 = (tone_r.clamp(0.0, 1.0) * 65535.0).round() as u16;     
       464 +                      let g_u16 = (tone_g.clamp(0.0, 1.0) * 65535.0).round() as u16;     
       465 +                      let b_u16 = (tone_b.clamp(0.0, 1.0) * 65535.0).round() as u16;     
       466
       467 +                      results.push(r_u16);
       468 +                      results.push(g_u16);
       469 +                      results.push(b_u16);
       470 +
       471                        if has_alpha {
       472 -                          let a_u16 = (chunk[3].clamp(0.0, 1.0) * 65535.0).round()       
     as u16;
           -  u16;
       473 -                          vec![r_u16, g_u16, b_u16, a_u16]
       474 -                      } else {
       475 -                          vec![r_u16, g_u16, b_u16]
       472 +                          let a_u16 = (a.clamp(0.0, 1.0) * 65535.0).round() as u16;      
       473 +                          results.push(a_u16);
       474                        }
       475 -                  })
       476 -          );
       477 -
       478 -          Ok(processed)
       475 +                  }
       476 +
       477 +                  results
       478 +              })
       479 +              .collect()
       480        }
       481
       482        /// Generate output file path

● Now let me add buffer pool optimizations and complete the performance optimizations:

  Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 92 additions and 18 removals
       299            })
       300        }
       301    
       302 -      /// Compose RGB from layer channels (optimized version)
       302 +      /// Compose RGB from layer channels (highly optimized version)
       303        fn compose_rgb_from_channels(&self, layer_channels: &LayerChannels) -> 
     Vec<f32>
              {
       304            let pixel_count = (layer_channels.width * layer_channels.height) as usize;     
       305
     ...
       314                Vec::with_capacity(buffer_size)
       315            };
       316
       317 -          // Find RGB and Alpha channels
       317 +          // Find RGB and Alpha channels with optimized lookup
       318            let find_channel = |name: &str| -> Option<usize> {
       319 +              let name_upper = name.to_ascii_uppercase();
       320                layer_channels.channel_names.iter().position(|n| {
       321                    let n_upper = n.to_ascii_uppercase();
       322 -                  let name_upper = name.to_ascii_uppercase();
       322                    n_upper == name_upper || n_upper.starts_with(&name_upper)
       323                })
       324            };
     ...
       328            let b_idx = find_channel("B").unwrap_or(g_idx);
       329            let a_idx = find_channel("A");
       330
       331 -          // Extract channel slices
       332 -          let r_slice = &layer_channels.channel_data[r_idx * pixel_count..(r_idx +       
     1) * pixel_count];
           -  * pixel_count];
       333 -          let g_slice = &layer_channels.channel_data[g_idx * pixel_count..(g_idx +       
     1) * pixel_count];
           -  * pixel_count];
       334 -          let b_slice = &layer_channels.channel_data[b_idx * pixel_count..(b_idx +       
     1) * pixel_count];
           -  * pixel_count];
       331 +          // Extract channel slices with bounds checking
       332 +          let channel_data = &layer_channels.channel_data;
       333 +          let r_slice = &channel_data[r_idx * pixel_count..(r_idx + 1) * 
     pixel_count];
           + pixel_count];
       334 +          let g_slice = &channel_data[g_idx * pixel_count..(g_idx + 1) * 
     pixel_count];
           + pixel_count];
       335 +          let b_slice = &channel_data[b_idx * pixel_count..(b_idx + 1) * 
           + pixel_count];
       336            let a_slice = a_idx.map(|idx| {
       337 -              &layer_channels.channel_data[idx * pixel_count..(idx + 1) * 
     pixel_count]
           - pixel_count]
       337 +              &channel_data[idx * pixel_count..(idx + 1) * pixel_count]
       338            });
       339
       340 -          // Optimized parallel composition
       340 +          // High-performance SIMD composition when available
       341 +          #[cfg(feature = "unified_simd")]
       342 +          {
       343 +              self.compose_rgb_simd_optimized(
       344 +                  &mut rgb_pixels, 
       345 +                  r_slice, 
       346 +                  g_slice, 
       347 +                  b_slice, 
       348 +                  a_slice, 
       349 +                  pixel_count
       350 +              );
       351 +          }
       352 +  
       353 +          #[cfg(not(feature = "unified_simd"))]
       354 +          {
       355 +              // Fallback to optimized parallel composition
       356 +              unsafe {
       357 +                  rgb_pixels.set_len(buffer_size);
       358 +                  let rgb_ptr = rgb_pixels.as_mut_ptr();
       359 +  
       360 +                  (0..pixel_count).into_par_iter().for_each(|i| {
       361 +                      let base_idx = i * 4;
       362 +                      *rgb_ptr.add(base_idx) = r_slice[i];
       363 +                      *rgb_ptr.add(base_idx + 1) = g_slice[i];
       364 +                      *rgb_ptr.add(base_idx + 2) = b_slice[i];
       365 +                      *rgb_ptr.add(base_idx + 3) = a_slice.map_or(1.0, |a| a[i]);        
       366 +                  });
       367 +              }
       368 +          }
       369 +  
       370 +          rgb_pixels
       371 +      }
       372 +  
       373 +      #[cfg(feature = "unified_simd")]
       374 +      /// SIMD-optimized RGB composition for maximum performance
       375 +      fn compose_rgb_simd_optimized(
       376 +          &self,
       377 +          rgb_pixels: &mut Vec<f32>,
       378 +          r_slice: &[f32],
       379 +          g_slice: &[f32],
       380 +          b_slice: &[f32],
       381 +          a_slice: Option<&[f32]>,
       382 +          pixel_count: usize,
       383 +      ) {
       384 +          use std::simd::{f32x4, Simd};
       385 +  
       386 +          let buffer_size = pixel_count * 4;
       387            unsafe {
       388                rgb_pixels.set_len(buffer_size);
       389 -              let rgb_ptr = rgb_pixels.as_mut_ptr();
       389 +          }
       390
       391 -              (0..pixel_count).into_par_iter().for_each(|i| {
       392 -                  let base_idx = i * 4;
       393 -                  *rgb_ptr.add(base_idx) = r_slice[i];
       394 -                  *rgb_ptr.add(base_idx + 1) = g_slice[i];
       395 -                  *rgb_ptr.add(base_idx + 2) = b_slice[i];
       396 -                  *rgb_ptr.add(base_idx + 3) = a_slice.map_or(1.0, |a| a[i]);
       397 -              });
       391 +          let chunks = pixel_count / 4;
       392 +          let remainder = pixel_count % 4;
       393 +  
       394 +          // Process in SIMD chunks of 4 pixels
       395 +          for chunk_idx in 0..chunks {
       396 +              let base_pixel = chunk_idx * 4;
       397 +              let base_output = base_pixel * 4;
       398 +  
       399 +              // Load 4 values from each channel
       400 +              let r_chunk = f32x4::from_slice(&r_slice[base_pixel..base_pixel + 4]);     
       401 +              let g_chunk = f32x4::from_slice(&g_slice[base_pixel..base_pixel + 4]);     
       402 +              let b_chunk = f32x4::from_slice(&b_slice[base_pixel..base_pixel + 4]);     
       403 +              let a_chunk = if let Some(a) = a_slice {
       404 +                  f32x4::from_slice(&a[base_pixel..base_pixel + 4])
       405 +              } else {
       406 +                  f32x4::splat(1.0)
       407 +              };
       408 +  
       409 +              // Interleave RGBA data efficiently
       410 +              for i in 0..4 {
       411 +                  rgb_pixels[base_output + i * 4] = r_chunk[i];
       412 +                  rgb_pixels[base_output + i * 4 + 1] = g_chunk[i];
       413 +                  rgb_pixels[base_output + i * 4 + 2] = b_chunk[i];
       414 +                  rgb_pixels[base_output + i * 4 + 3] = a_chunk[i];
       415 +              }
       416            }
       417
       418 -          rgb_pixels
       418 +          // Handle remaining pixels
       419 +          for i in 0..remainder {
       420 +              let pixel_idx = chunks * 4 + i;
       421 +              let output_idx = pixel_idx * 4;
       422 +
       423 +              rgb_pixels[output_idx] = r_slice[pixel_idx];
       424 +              rgb_pixels[output_idx + 1] = g_slice[pixel_idx];
       425 +              rgb_pixels[output_idx + 2] = b_slice[pixel_idx];
       426 +              rgb_pixels[output_idx + 3] = a_slice.map_or(1.0, |a| a[pixel_idx]);        
       427 +          }
       428        }
       429
       430        /// Process grayscale pixels with tone mapping

  Update Todos
  ⎿  ☒ Analyze existing layer handling and EXR structure
     ☒ Design export module architecture for different layer configurations
     ☒ Implement Base layer export functionality
     ☒ Add PNG 16-bit export support
     ☒ Add TIFF 16/32-bit export support
     ☒ Integrate export functionality into UI
     ☒ Add performance optimizations and SIMD support

● Now let me test if the code compiles correctly:

● Bash(timeout 30s cargo check)
  ⎿  Error: Checking EXruster_nightly v0.3.5 (C:\_cloud\___EXRuster)
     error[E0432]: unresolved imports `image::Rgb16`, `image::Rgba16`
        --> src\processing\layer_export.rs:606:34
         |
     606 |         use image::{ImageBuffer, Rgb16, Rgba16};
         |                                  ^^^^^  ^^^^^^
         |                                  |      |
         |                                  |      no `Rgba16` in the root
         |                                  |      help: a similar name exists in the module: 
     `Rgba`
         |                                  no `Rgb16` in the root
     … +166 lines (ctrl+r to see all)

● Let me fix the compilation errors:

● Update(src\processing\layer_export.rs)
  ⎿  Updated src\processing\layer_export.rs with 3 additions and 3 removals
       603    
       604        /// Save as PNG 16-bit
       605        fn save_png16(&self, pixels: &ProcessedPixels, output_path: &PathBuf) -> 
             Result<()> {
       606 -          use image::{ImageBuffer, Rgb16, Rgba16};
       606 +          use image::{ImageBuffer, Rgba};
       607    
       608            if pixels.channels == 4 {
       609 -              let img_buffer = ImageBuffer::<Rgba16, _>::from_raw(
       609 +              let img_buffer = ImageBuffer::<Rgba<u16>, _>::from_raw(
       610                    pixels.width,
       611                    pixels.height,
       612                    pixels.data.clone(),
     ...
       614
       615                img_buffer.save(output_path)?;
       616            } else {
       617 -              let img_buffer = ImageBuffer::<Rgb16, _>::from_raw(
       617 +              let img_buffer = ImageBuffer::<image::Rgb<u16>, _>::from_raw(
       618                    pixels.width,
       619                    pixels.height,
       620                    pixels.data.clone(),
  ⎿  Claude usage limit reached. Your limit will reset at 12am (Europe/Warsaw).

      • /upgrade to increase your usage limit.