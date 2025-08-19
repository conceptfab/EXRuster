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




