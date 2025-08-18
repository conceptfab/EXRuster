use rfd::FileDialog;
use std::path::PathBuf;
use anyhow;
use exr::prelude as exr;
use image;

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

/// Wczytuje wymiary obrazu EXR
pub fn load_exr_dimensions(file_path: &std::path::Path) -> anyhow::Result<(u32, u32, Vec<String>)> {
    // Użyj prawidłowego API exr
    let reader = exr::read_first_rgba_layer_from_file(
        file_path,
        |resolution, _| exr::pixel_vec::PixelVec {
            resolution,
            pixels: vec![image::Rgba([0u8; 4]); resolution.width() * resolution.height()],
        },
        |_pixel_vec, _position, _rgba: (f32, f32, f32, f32)| {
            // Nie przetwarzamy pikseli, tylko pobieramy wymiary
        },
    )?;
    
    let image_data = reader.layer_data.channel_data.pixels;
    let (width, height) = (
        image_data.resolution.width() as u32,
        image_data.resolution.height() as u32,
    );
    
    // Pobierz nazwy kanałów z metadanych
    let meta = ::exr::meta::MetaData::read_from_file(file_path, false)?;
    let mut channels: Vec<String> = Vec::new();
    for header in meta.headers.iter() {
        for ch in header.channels.list.iter() {
            channels.push(ch.name.to_string());
        }
    }
    
    Ok((width, height, channels))
}

/// Wczytuje dane obrazu EXR
pub fn load_exr_data(file_path: &std::path::Path) -> anyhow::Result<Vec<f32>> {
    // Użyj prawidłowego API exr
    let reader = exr::read_first_rgba_layer_from_file(
        file_path,
        |resolution, _| exr::pixel_vec::PixelVec {
            resolution,
            pixels: vec![image::Rgba([0u8; 4]); resolution.width() * resolution.height()],
        },
        |pixel_vec, position, (r, g, b, a): (f32, f32, f32, f32)| {
            let index = position.y() * pixel_vec.resolution.width() + position.x();
            pixel_vec.pixels[index] = image::Rgba([
                (r * 255.0).clamp(0.0, 255.0) as u8,
                (g * 255.0).clamp(0.0, 255.0) as u8,
                (b * 255.0).clamp(0.0, 255.0) as u8,
                (a * 255.0).clamp(0.0, 255.0) as u8,
            ]);
        },
    )?;
    
    let image_data = reader.layer_data.channel_data.pixels;
    let (width, height) = (
        image_data.resolution.width() as u32,
        image_data.resolution.height() as u32,
    );
    
    // Konwertuj na Vec<f32> w formacie RGBA
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for rgba in &image_data.pixels {
        pixels.extend_from_slice(&[rgba.0[0] as f32, rgba.0[1] as f32, rgba.0[2] as f32, rgba.0[3] as f32]);
    }
    
    Ok(pixels)
}
