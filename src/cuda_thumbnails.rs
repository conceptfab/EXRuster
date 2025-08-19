// CUDA-accelerated thumbnail generation for EXRuster
// Specialized high-performance thumbnail processing with batch support

use crate::cuda_context::CudaContext;
use crate::cuda_processing;
use crate::thumbnails::{ExrThumbWork, ColorConfig, TimingStats};
use crate::progress::ProgressSink;
use anyhow::Result;
use std::path::Path;
use std::fs;
use rayon::prelude::*;
use itertools::{Either, Itertools};

/// CUDA-specific thumbnail generation for single EXR file
pub async fn generate_single_exr_thumbnail_cuda(
    cuda_context: &CudaContext,
    file_path: &Path,
    thumb_height: u32,
    color_config: &ColorConfig,
    timing_stats: &TimingStats,
) -> Result<ExrThumbWork> {
    let cuda_start = std::time::Instant::now();
    
    // Load EXR dimensions and channel info
    let (width, height, channels) = crate::file_operations::load_exr_dimensions(file_path)?;
    
    // Calculate thumbnail width maintaining aspect ratio
    let aspect_ratio = width as f32 / height as f32;
    let thumb_width = (thumb_height as f32 * aspect_ratio).round() as u32;
    
    // Load EXR pixel data
    let exr_data = crate::file_operations::load_exr_data(file_path)?;
    
    // Use CUDA for thumbnail generation
    let (thumbnail_bytes, actual_thumb_width, actual_thumb_height) = 
        cuda_processing::cuda_generate_thumbnail_from_pixels(
            cuda_context,
            &exr_data,
            width,
            height,
            thumb_height,
            color_config.exposure,
            color_config.gamma,
            color_config.tonemap_mode,
            None, // color_matrix - TODO: Add color matrix support
        ).await?;
    
    // Create thumbnail work result
    let thumb_work = ExrThumbWork {
        path: file_path.to_path_buf(),
        file_name: file_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string(),
        file_size_bytes: fs::metadata(file_path)
            .map(|m| m.len())
            .unwrap_or(0),
        width: actual_thumb_width,
        height: actual_thumb_height,
        num_layers: channels.len(),
        pixels: thumbnail_bytes,
    };
    
    // Add timing stats
    timing_stats.add_load_time(cuda_start.elapsed());
    
    Ok(thumb_work)
}

/// CUDA-accelerated batch thumbnail generation for multiple EXR files
pub async fn generate_thumbnails_cuda_batch(
    cuda_context: &CudaContext,
    files: Vec<std::path::PathBuf>,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
) -> Result<Vec<ExrThumbWork>> {
    let total_files = files.len();
    let timing_stats = TimingStats::new();
    let color_config = ColorConfig::new(gamma, exposure, tonemap_mode);

    progress.map(|p| p.set(0.05, Some("🚀 CUDA-accelerated thumbnail generation...")));

    println!("CUDA: Starting batch processing of {} files", total_files);
    let batch_start = std::time::Instant::now();

    // CUDA batch processing - process multiple files in parallel using CPU threads
    // while each individual thumbnail uses CUDA for the heavy lifting
    let batch_size = 4; // Process 4 files concurrently to avoid GPU memory pressure
    let results: Vec<_> = files
        .par_chunks(batch_size)
        .enumerate()
        .flat_map(|(batch_idx, batch_files)| {
            let timing_stats = timing_stats.clone();
            let color_config = color_config.clone();
            
            // Process each file in the batch
            batch_files
                .iter()
                .enumerate()
                .filter_map(move |(file_idx, file_path)| {
                    let global_idx = batch_idx * batch_size + file_idx;
                    
                    // Update progress
                    if file_idx == 0 {
                        progress.map(|p| {
                            p.set(
                                0.1 + 0.8 * (global_idx as f32) / (total_files as f32),
                                Some(&format!("🚀 CUDA Batch {} processing {} files", 
                                            batch_idx + 1, batch_files.len()))
                            )
                        });
                    }

                    let file_start = std::time::Instant::now();
                    
                    // Try CUDA thumbnail generation
                    let cuda_result = pollster::block_on(
                        generate_single_exr_thumbnail_cuda(
                            cuda_context,
                            file_path,
                            thumb_height,
                            &color_config,
                            &timing_stats,
                        )
                    );
                    
                    match cuda_result {
                        Ok(thumb_work) => {
                            timing_stats.add_load_time(file_start.elapsed());
                            println!("CUDA: ✅ {}", file_path.file_name()
                                .and_then(|n| n.to_str()).unwrap_or("?"));
                            Some(thumb_work)
                        }
                        Err(e) => {
                            eprintln!("⚠️ CUDA thumbnail failed for {}, falling back to CPU: {}", 
                                    file_path.display(), e);
                            
                            // CPU fallback
                            match crate::thumbnails::generate_single_exr_thumbnail_work_new(
                                file_path, 
                                thumb_height, 
                                &color_config, 
                                &timing_stats
                            ) {
                                Ok(thumb_work) => {
                                    timing_stats.add_load_time(file_start.elapsed());
                                    println!("CPU: ✅ {}", file_path.file_name()
                                        .and_then(|n| n.to_str()).unwrap_or("?"));
                                    Some(thumb_work)
                                }
                                Err(cpu_e) => {
                                    eprintln!("❌ Both CUDA and CPU failed for {}: {}", 
                                            file_path.display(), cpu_e);
                                    None
                                }
                            }
                        }
                    }
                })
        })
        .collect();

    let batch_duration = batch_start.elapsed();
    let total_duration = timing_stats.get_total_time();

    progress.map(|p| {
        p.set(1.0, Some(&format!(
            "✅ CUDA thumbnails: {} files in {:.2}s (avg: {:.1}ms/file)", 
            results.len(),
            batch_duration.as_secs_f32(),
            batch_duration.as_millis() as f32 / results.len() as f32
        )))
    });

    println!("CUDA: Batch processing completed:");
    println!("  📊 Files processed: {}/{}", results.len(), total_files);
    println!("  ⏱️  Total time: {:.2}s", batch_duration.as_secs_f32());
    println!("  🚀 Average per file: {:.1}ms", 
             batch_duration.as_millis() as f32 / results.len() as f32);
    
    timing_stats.print_timing_summary();

    Ok(results)
}

/// Check if CUDA acceleration should be used for thumbnails
pub fn should_use_cuda_for_thumbnails() -> bool {
    // TODO: Add user preference checking
    // TODO: Add minimum file size threshold
    // TODO: Add GPU memory availability checking
    true
}

/// Hybrid thumbnail generation - uses CUDA when beneficial, CPU otherwise
pub async fn generate_thumbnails_hybrid(
    cuda_context: Option<&CudaContext>,
    files: Vec<std::path::PathBuf>,
    thumb_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    progress: Option<&dyn ProgressSink>,
) -> Result<Vec<ExrThumbWork>> {
    // Check if CUDA is available and beneficial
    let use_cuda = cuda_context.is_some() 
        && should_use_cuda_for_thumbnails()
        && files.len() >= 2; // Use CUDA for 2+ files
    
    if use_cuda {
        let cuda_ctx = cuda_context.unwrap();
        if cuda_ctx.is_available() {
            println!("Using CUDA acceleration for {} thumbnails", files.len());
            return generate_thumbnails_cuda_batch(
                cuda_ctx,
                files,
                thumb_height,
                exposure,
                gamma,
                tonemap_mode,
                progress,
            ).await;
        }
    }
    
    // Fallback to CPU processing
    println!("Using CPU processing for {} thumbnails", files.len());
    progress.map(|p| p.set(0.05, Some("Using CPU processing...")));
    crate::thumbnails::generate_thumbnails_cpu_raw(
        files,
        thumb_height,
        exposure,
        gamma,
        tonemap_mode,
        progress,
    )
}