use clap::Parser;
use exr::prelude as exr;
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use std::time::{Instant, Duration};

/// A fast EXR to thumbnail converter with linear color space support
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Source folder containing EXR files
    #[arg(short = 's', long)]
    source_folder: PathBuf,

    /// Destination folder for thumbnails
    #[arg(short = 'd', long)]
    dest_folder: PathBuf,

    /// Height of the thumbnail in pixels (width is scaled proportionally)
    #[arg(short = 't', long)]
    height: u32,

    /// Filename for the conversion statistics report
    #[arg(short, long, default_value = "conversion_stats.txt")]
    info: String,

    /// Enable linear color space tone mapping
    #[arg(short = 'l', long)]
    linear_tone_mapping: bool,

    /// Gamma value for color correction (default: 2.2)
    #[arg(short = 'g', long, default_value = "2.2")]
    gamma: f32,

    /// Scaling filter algorithm (lanczos3, gaussian, cubic, triangle)
    #[arg(short = 'f', long, default_value = "lanczos3")]
    filter: String,
}

/// Statistics for timing operations
struct TimingStats {
    total_load_time: AtomicU64,    // Total time for loading/creating thumbnails (in nanoseconds)
    total_save_time: AtomicU64,    // Total time for saving thumbnails (in nanoseconds)
}

impl TimingStats {
    fn new() -> Self {
        Self {
            total_load_time: AtomicU64::new(0),
            total_save_time: AtomicU64::new(0),
        }
    }

    fn add_load_time(&self, duration: Duration) {
        self.total_load_time.fetch_add(duration.as_nanos() as u64, Ordering::SeqCst);
    }

    fn add_save_time(&self, duration: Duration) {
        self.total_save_time.fetch_add(duration.as_nanos() as u64, Ordering::SeqCst);
    }

    fn get_load_time(&self) -> Duration {
        Duration::from_nanos(self.total_load_time.load(Ordering::SeqCst))
    }

    fn get_save_time(&self) -> Duration {
        Duration::from_nanos(self.total_save_time.load(Ordering::SeqCst))
    }

    fn get_total_time(&self) -> Duration {
        self.get_load_time() + self.get_save_time()
    }
}

/// Color processing configuration
struct ColorConfig {
    linear_tone_mapping: bool,
    gamma: f32,
}

impl ColorConfig {
    fn new(linear_tone_mapping: bool, gamma: f32) -> Self {
        Self {
            linear_tone_mapping,
            gamma,
        }
    }
}

fn process_exr_file(
    exr_path: &Path,
    dest_folder: &Path,
    height: u32,
    timing_stats: &TimingStats,
    color_config: &ColorConfig,
    filter_type: image::imageops::FilterType,
) -> Result<PathBuf, String> {
    let file_name = exr_path.file_name().ok_or("Invalid file name")?;
    let file_name_str = file_name.to_string_lossy();
    let mut out_path = dest_folder.to_path_buf();
    out_path.push(file_name_str.as_ref());
    out_path.set_extension("png");

    let load_start = Instant::now();

    // Copy color config data to avoid lifetime issues
    let linear_tone_mapping = color_config.linear_tone_mapping;
    let gamma = color_config.gamma;

    // Read the EXR file using the existing working API
    let reader = exr::read_first_rgba_layer_from_file(
        exr_path,
        // A function that generates the pixel data for the image
        |resolution, _| exr::pixel_vec::PixelVec {
            resolution,
            pixels: vec![image::Rgba([0u8; 4]); resolution.width() * resolution.height()],
        },
        // A function that fills the previously generated pixel data with color processing
        move |pixel_vec, position, (r, g, b, a): (f32, f32, f32, f32)| {
            let index = position.y() * pixel_vec.resolution.width() + position.x();
            
            // Process pixel with copied color config
            let (r, g, b) = if linear_tone_mapping {
                // Reinhard tone mapping dla HDR
                let tone_map = |x: f32| x / (1.0 + x);
                (tone_map(r), tone_map(g), tone_map(b))
            } else {
                (r, g, b)
            };

            // Gamma correction
            let gamma_correct = |x: f32| x.powf(1.0 / gamma);
            
            let processed = [
                (gamma_correct(r.max(0.0).min(1.0)) * 255.0) as u8,
                (gamma_correct(g.max(0.0).min(1.0)) * 255.0) as u8,
                (gamma_correct(b.max(0.0).min(1.0)) * 255.0) as u8,
                (a.max(0.0).min(1.0) * 255.0) as u8,
            ];
            
            pixel_vec.pixels[index] = image::Rgba(processed);
        },
    )
    .map_err(|e| e.to_string())?;

    // Access the pixel data correctly
    let image_data = reader.layer_data.channel_data.pixels;
    let (width, img_height) = (
        image_data.resolution.width() as u32,
        image_data.resolution.height() as u32,
    );

    let thumb_width = (width as f32 / img_height as f32 * height as f32) as u32;

    // Create a dynamic image from the raw pixel data
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        width,
        img_height,
        image_data.pixels.into_iter().flat_map(|rgba| rgba.0).collect::<Vec<u8>>(),
    )
    .ok_or("Could not create image buffer")?;

    // Resize the image using the specified filter
    let thumbnail = image::imageops::resize(&img, thumb_width, height, filter_type);

    let load_duration = load_start.elapsed();
    timing_stats.add_load_time(load_duration);

    let save_start = Instant::now();
    thumbnail.save(&out_path).map_err(|e| e.to_string())?;
    let save_duration = save_start.elapsed();
    timing_stats.add_save_time(save_duration);

    Ok(out_path)
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let start_time = Instant::now();

    if !args.source_folder.is_dir() {
        eprintln!("Error: Source path is not a valid directory.");
        return Ok(());
    }

    fs::create_dir_all(&args.dest_folder)?;

    let color_config = ColorConfig::new(args.linear_tone_mapping, args.gamma);

    // Parsowanie filtru skalowania
    let filter_type = match args.filter.as_str() {
        "lanczos3" => image::imageops::FilterType::Lanczos3,
        "gaussian" => image::imageops::FilterType::Gaussian,
        "cubic" => image::imageops::FilterType::CatmullRom,
        "triangle" => image::imageops::FilterType::Triangle,
        _ => {
            eprintln!("Warning: Unknown filter '{}', using Lanczos3", args.filter);
            image::imageops::FilterType::Lanczos3
        }
    };

    // Find all EXR files
    let exr_files: Vec<PathBuf> = fs::read_dir(&args.source_folder)?
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                let path = e.path();
                if path.is_file() && path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("exr")) {
                    Some(path)
                } else {
                    None
                }
            })
        })
        .collect();

    let total_files = exr_files.len();
    let success_count = AtomicUsize::new(0);
    let failure_count = AtomicUsize::new(0);
    let timing_stats = TimingStats::new();

    println!(
        "Found {} EXR files. Starting conversion to {}px height thumbnails...",
        total_files, args.height
    );

    // Process files in parallel
    exr_files.par_iter().for_each(|exr_path| {
        match process_exr_file(exr_path, &args.dest_folder, args.height, &timing_stats, &color_config, filter_type) {
            Ok(thumb_path) => {
                println!("Successfully created thumbnail: {}", thumb_path.display());
                success_count.fetch_add(1, Ordering::SeqCst);
            }
            Err(e) => {
                eprintln!("Failed to process {}: {}", exr_path.display(), e);
                failure_count.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    let total_duration = start_time.elapsed();
    let successes = success_count.load(Ordering::SeqCst);
    let failures = failure_count.load(Ordering::SeqCst);
    
    // Get timing statistics
    let load_time = timing_stats.get_load_time();
    let save_time = timing_stats.get_save_time();
    let processing_time = timing_stats.get_total_time();

    println!("\n=== Conversion Statistics ===");
    println!("Total execution time: {:.2}ms", total_duration.as_millis());
    println!("Processing time breakdown (parallel processing):");
    println!("  - Loading/Creation: {:.2}ms (sum of all files)", load_time.as_millis());
    println!("  - Saving: {:.2}ms (sum of all files)", save_time.as_millis());
    println!("  - Total processing: {:.2}ms (sum of all files)", processing_time.as_millis());
    println!("Files: Success: {}, Failure: {}", successes, failures);
    println!("\nNote: Times are summed across all files due to parallel processing.");
    println!("Total execution time is much shorter than sum of individual file times.");

    // Write detailed statistics to info file
    let stats_path = args.dest_folder.join(&args.info);
    let mut stats_file = File::create(&stats_path)?;
    writeln!(stats_file, "=== EXR to Thumbnail Conversion Statistics ===")?;
    writeln!(stats_file, "Source Folder: {}", args.source_folder.display())?;
    writeln!(stats_file, "Destination Folder: {}", args.dest_folder.display())?;
    writeln!(stats_file, "Target Thumbnail Height: {}px", args.height)?;
    writeln!(stats_file, "============================================")?;
    writeln!(stats_file, "Total files found: {}", total_files)?;
    writeln!(stats_file, "Successfully converted: {}", successes)?;
    writeln!(stats_file, "Failed to convert: {}", failures)?;
    writeln!(stats_file, "============================================")?;
    writeln!(stats_file, "Timing Breakdown (Parallel Processing):")?;
    writeln!(stats_file, "  Total execution time: {:.2}ms", total_duration.as_millis())?;
    writeln!(stats_file, "  Loading/Creation time: {:.2}ms (sum of all files)", load_time.as_millis())?;
    writeln!(stats_file, "  Saving time: {:.2}ms (sum of all files)", save_time.as_millis())?;
    writeln!(stats_file, "  Total processing time: {:.2}ms (sum of all files)", processing_time.as_millis())?;
    writeln!(stats_file, "")?;
    writeln!(stats_file, "Note: Due to parallel processing, total execution time is much shorter")?;
    writeln!(stats_file, "than the sum of individual file processing times.")?;
    if total_files > 0 {
        writeln!(stats_file, "  Average load time per file: {:.2}ms", (load_time.as_millis() as f64 / total_files as f64))?;
        writeln!(stats_file, "  Average save time per file: {:.2}ms", (save_time.as_millis() as f64 / total_files as f64))?;
        writeln!(stats_file, "  Average total time per file: {:.2}ms", (processing_time.as_millis() as f64 / total_files as f64))?;
    }
    writeln!(stats_file, "============================================")?;

    println!("Detailed statistics saved to {}", stats_path.display());

    Ok(())
}