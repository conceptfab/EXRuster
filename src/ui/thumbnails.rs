use slint::{Weak, ComponentHandle, ModelRc, VecModel, SharedPixelBuffer, Rgba8Pixel, Image};
use std::path::Path;
use std::time::Instant;
use crate::ui::progress::{UiProgress, ProgressSink};
use crate::utils::human_size;
use crate::{AppWindow, ThumbItem};
use crate::ui::ui_handlers::{push_console, ConsoleModel};

// Thumbnail height constant - adjust here to change resolution
const THUMBNAIL_HEIGHT: u32 = 130;

/// Common function for loading thumbnails for the specified directory and updating UI.
/// Used both at application startup (after file argument) and after folder selection from UI.
/// Now works asynchronously in a separate thread to avoid blocking UI.
pub fn load_thumbnails_for_directory(
    ui_handle: Weak<AppWindow>,
    directory: &Path,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        push_console(&ui, &console, format!("[folder] loading thumbnails: {}", directory.display()));
        ui.set_status_text(format!("Loading thumbnails: {}", directory.display()).into());
        

        let prog = UiProgress::new(ui.as_weak());
        prog.start_indeterminate(Some("🔍 Scanning folder for EXR files..."));
        
        // Clear cache to force regeneration of thumbnails with new parameters
        crate::io::thumbnails::clear_thumb_cache();
        
        // Use constant, optimized values for thumbnails (not from UI!)
        let exposure = 0.0;     // Neutral exposure for thumbnails
        let gamma = 2.2;        // Standard gamma for thumbnails  
        let tonemap_mode = 0;   // ACES tone mapping for thumbnails
        
    
        
        // Generate thumbnails in main thread (safer and with cache)
        let ui_weak = ui.as_weak();
        let directory_path = directory.to_path_buf();
        
        // Generate thumbnails in background with cache - using simpler approach
        std::thread::spawn(move || {
            let t0 = Instant::now();
            
            // Generate thumbnails in separate thread using existing function with cache
            let files = match crate::io::thumbnails::list_exr_files(&directory_path) {
                Ok(files) => files,
                Err(e) => {
                    let ui_weak_clone = ui_weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.set_status_text(format!("❌ Error scanning folder: {}", e).into());
                            prog.reset();
                        }
                    }).unwrap();
                    return;
                }
            };
            
            // Check if folder is not empty
            let total_files = files.len();
            if total_files == 0 {
                let ui_weak_clone = ui_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak_clone.upgrade() {
                        ui.set_status_text("⚠️ No EXR files found in selected folder".into());
                        prog.finish(Some("⚠️ No EXR files found"));
                    }
                }).unwrap();
                return;
            }
            
            // Use new, efficient function for generating thumbnails
            prog.set(0.1, Some(&format!("📁 Found {} EXR files, starting processing...", total_files)));
            
            // Generate thumbnails in separate thread - use GPU if available
            let thumbnail_works = match crate::io::thumbnails::generate_thumbnails_gpu_raw(
                files,
                THUMBNAIL_HEIGHT, 
                exposure, 
                gamma, 
                tonemap_mode,
                Some(&prog)
            ) {
                Ok(works) => works,
                Err(e) => {
                    eprintln!("Failed to generate thumbnails: {}", e);
                    let ui_weak_clone = ui_weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.set_status_text(format!("❌ Failed to generate thumbnails: {}", e).into());
                            prog.finish(Some("❌ Thumbnail generation failed"));
                        }
                    }).unwrap();
                    return;
                }
            };
            
            prog.set(0.9, Some("📊 Sorting thumbnails alphabetically..."));
            let mut sorted_works = thumbnail_works;
            sorted_works.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
            
            let count = sorted_works.len();
            let ms = t0.elapsed().as_millis();
            
            // Update UI in main thread via invoke_from_event_loop
            let ui_weak_clone = ui_weak.clone();
            let count_clone = count;
            let ms_clone = ms;
            
            slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_clone.upgrade() {
                    prog.set(0.95, Some("🎨 Converting thumbnails to UI format..."));
                    
                    // Convert thumbnails to UI format in main thread
                    let items: Vec<ThumbItem> = sorted_works
                        .into_iter()
                        .map(|w| {
                            // Convert raw RGBA8 pixels to slint::Image
                            let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(w.width, w.height);
                            let slice = buffer.make_mut_slice();
                            
                            // Copy RGBA8 pixels
                            for (dst, chunk) in slice.iter_mut().zip(w.pixels.chunks_exact(4)) {
                                *dst = Rgba8Pixel { 
                                    r: chunk[0], 
                                    g: chunk[1], 
                                    b: chunk[2], 
                                    a: chunk[3] 
                                };
                            }
                            
                            let image = Image::from_rgba8(buffer);
                            
                            ThumbItem {
                                img: image,
                                name: w.file_name.into(),
                                size: human_size(w.file_size_bytes).into(),
                                layers: format!("{} layers", w.num_layers).into(),
                                path: w.path.display().to_string().into(),
                                width: w.width as i32,
                                height: w.height as i32,
                            }
                        })
                        .collect();
                    
                    // Update UI
                    ui.set_thumbnails(ModelRc::new(VecModel::from(items)));
                    ui.set_bottom_panel_visible(true);
                    
                    ui.set_status_text("Thumbnails loaded".into());
                    prog.finish(Some(&format!("✅ Successfully loaded {} thumbnails in {} ms", count_clone, ms_clone)));
                    
                    // Log to console in separate call - using simple status text
                    ui.set_status_text(format!("[folder] {} EXR files | thumbnails in {} ms", count_clone, ms_clone).into());
                }
            }).unwrap();
        });
    }
}