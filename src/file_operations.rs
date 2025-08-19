use rfd::FileDialog;
use std::path::PathBuf;

/// Otwiera dialog wyboru pliku i zwraca wybraną ścieżkę
/// 
/// # Returns
/// * `Option<PathBuf>` - ścieżka do wybranego pliku lub None jeśli anulowano
pub fn open_file_dialog() -> Option<PathBuf> {
    FileDialog::new()
        .add_filter("Obrazy", &["exr", "png", "jpg", "jpeg", "gif"])
        .add_filter("Wszystkie pliki", &["*"])
        .set_title("Otwórz plik obrazu")
        .pick_file()
}


/// Pobiera nazwę pliku z ścieżki
/// 
/// # Arguments
/// * `path` - ścieżka do pliku
/// 
/// # Returns
/// * `String` - nazwa pliku lub "Nieznany plik" jeśli nie można pobrać nazwy
pub fn get_file_name(path: &PathBuf) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Nieznany plik")
        .to_string()
}

/// Otwiera dialog wyboru folderu roboczego
pub fn open_folder_dialog() -> Option<PathBuf> {
    FileDialog::new()
        .set_title("Wybierz folder roboczy")
        .pick_folder()
}

/// Load EXR dimensions and channel information (for CUDA thumbnails)
pub fn load_exr_dimensions(file_path: &std::path::Path) -> anyhow::Result<(u32, u32, Vec<String>)> {
    use exr::prelude::*;
    
    // Read only the header without pixel data
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(|resolution, _| {
            // Just return empty vectors - we only want dimensions
            vec![vec![0.0f32; (resolution.width() * resolution.height()) as usize]; 4]
        })
        .first_valid_layer()
        .from_file(file_path)?;
    
    let layer = &image.layer_data;
    let resolution = layer.size;
    let width = resolution.width() as u32;
    let height = resolution.height() as u32;
    
    // Get channel names
    let channel_names: Vec<String> = layer.channel_data.list.iter()
        .map(|ch| ch.name.clone())
        .collect();
    
    Ok((width, height, channel_names))
}

/// Load EXR pixel data as RGBA f32 (for CUDA thumbnails)
pub fn load_exr_data(file_path: &std::path::Path) -> anyhow::Result<Vec<f32>> {
    use exr::prelude::*;
    
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(|resolution, _| {
            // Allocate RGBA channels
            vec![vec![0.0f32; (resolution.width() * resolution.height()) as usize]; 4]
        })
        .first_valid_layer()
        .from_file(file_path)?;
    
    let layer = &image.layer_data;
    let channels = &layer.channel_data.list;
    let resolution = layer.size;
    let pixel_count = (resolution.width() * resolution.height()) as usize;
    
    // Create interleaved RGBA data
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);
    
    // Find R, G, B, A channels by name
    let r_channel = channels.iter().find(|ch| ch.name == "R");
    let g_channel = channels.iter().find(|ch| ch.name == "G");
    let b_channel = channels.iter().find(|ch| ch.name == "B");
    let a_channel = channels.iter().find(|ch| ch.name == "A");
    
    for i in 0..pixel_count {
        // Red channel
        if let Some(ch) = r_channel {
            rgba_data.push(ch.sample_data[i]);
        } else {
            rgba_data.push(0.0);
        }
        
        // Green channel
        if let Some(ch) = g_channel {
            rgba_data.push(ch.sample_data[i]);
        } else {
            rgba_data.push(0.0);
        }
        
        // Blue channel
        if let Some(ch) = b_channel {
            rgba_data.push(ch.sample_data[i]);
        } else {
            rgba_data.push(0.0);
        }
        
        // Alpha channel
        if let Some(ch) = a_channel {
            rgba_data.push(ch.sample_data[i]);
        } else {
            rgba_data.push(1.0); // Default alpha = 1.0
        }
    }
    
    Ok(rgba_data)
}
