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
    // Read only metadata (headers) without pixels - more efficient for dimensions
    let meta = ::exr::meta::MetaData::read_from_file(file_path, false)?;
    
    // Get the first header to extract dimensions and channel info
    let header = meta.headers.first()
        .ok_or_else(|| anyhow::anyhow!("No headers found in EXR file"))?;
    
    let width = header.layer_size.width() as u32;
    let height = header.layer_size.height() as u32;
    
    // Extract channel names
    let channel_names: Vec<String> = header.channels.list.iter()
        .map(|ch| ch.name.to_string())
        .collect();
    
    Ok((width, height, channel_names))
}

/// Load EXR pixel data as RGBA f32 (for CUDA thumbnails)
pub fn load_exr_data(file_path: &std::path::Path) -> anyhow::Result<Vec<f32>> {
    use exr::prelude::{read_first_rgba_layer_from_file, pixel_vec::PixelVec};
    
    let reader = read_first_rgba_layer_from_file(
        file_path,
        // Generate pixel buffer for RGBA f32 data
        |resolution, _| PixelVec {
            resolution,
            pixels: vec![[0.0f32; 4]; resolution.width() * resolution.height()],
        },
        // Store pixels in RGBA format
        |pixel_vec, position, (r, g, b, a): (f32, f32, f32, f32)| {
            let index = position.y() * pixel_vec.resolution.width() + position.x();
            if index < pixel_vec.pixels.len() {
                pixel_vec.pixels[index] = [r, g, b, a];
            }
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to read EXR data: {}", e))?;
    
    // Convert [f32; 4] array to interleaved Vec<f32>
    let mut rgba_data = Vec::with_capacity(reader.layer_data.channel_data.pixels.pixels.len() * 4);
    for pixel in reader.layer_data.channel_data.pixels.pixels {
        rgba_data.push(pixel[0]); // Red
        rgba_data.push(pixel[1]); // Green  
        rgba_data.push(pixel[2]); // Blue
        rgba_data.push(pixel[3]); // Alpha
    }
    
    Ok(rgba_data)
}
