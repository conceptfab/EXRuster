use rfd::FileDialog;
use std::path::{Path, PathBuf};

/// Otwiera dialog wyboru pliku i zwraca wybraną ścieżkę
///
/// # Returns
/// * `Option<PathBuf>` - ścieżka do wybranego pliku lub None jeśli anulowano
pub fn open_file_dialog() -> Option<PathBuf> {
    // Only exr/hdr: every non-.hdr pick is parsed as EXR, so offering png/jpg/gif
    // just produced a raw read error.
    FileDialog::new()
        .add_filter("Obrazy HDR", &["exr", "hdr"])
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
pub fn get_file_name(path: &Path) -> String {
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
