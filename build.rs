fn main() {
    // Kompilacja plików Slint
    slint_build::compile("ui/appwindow.slint").unwrap();

    // Osadzanie ikony aplikacji na Windows (jeśli istnieje plik .ico)
    #[cfg(target_os = "windows")]
    {
        use std::path::Path;

        let mut res = winres::WindowsResource::new();
        // Szukaj ikony w kilku typowych lokalizacjach
        let candidates = [
            "resources/img/icon.ico",
            "resources/icon.ico",
            "icon.ico",
        ];

        if let Some(found) = candidates.iter().find(|p| Path::new(p).exists()) {
            res.set_icon(found);
            if let Err(e) = res.compile() {
                panic!("Błąd kompilacji zasobów Windows (ikona): {}", e);
            }
        } else {
            println!(
                "cargo:warning=Nie znaleziono pliku ikony (.ico). Umieść go w 'resources/img/icon.ico' (lub 'resources/icon.ico' / 'icon.ico'). Pomijam osadzanie ikony."
            );
        }
    }
}