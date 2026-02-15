fn main() {
    // Kompilacja plików Slint
    slint_build::compile("ui/appwindow.slint").unwrap();

    // Data i godzina kompilacji (UTC)
    let build_datetime = {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs() as i64;
        chrono::DateTime::from_timestamp(secs, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };

    let pkg_version = env!("CARGO_PKG_VERSION");
    let version_with_build = format!("{} ({})", pkg_version, build_datetime);

    println!("cargo:rustc-env=BUILD_DATETIME={}", build_datetime);
    println!("cargo:rustc-env=VERSION_WITH_BUILD={}", version_with_build);

    // Osadzanie ikony aplikacji na Windows (jeśli istnieje plik .ico)
    #[cfg(target_os = "windows")]
    {
        use std::path::Path;

        let mut res = winres::WindowsResource::new();
        res.set("FileVersion", &version_with_build);
        res.set("ProductVersion", &version_with_build);

        // Szukaj ikony w kilku typowych lokalizacjach
        let candidates = ["resources/img/icon.ico", "resources/icon.ico", "icon.ico"];

        if let Some(found) = candidates.iter().find(|p| Path::new(p).exists()) {
            res.set_icon(found);
            if let Err(e) = res.compile() {
                panic!("Błąd kompilacji zasobów Windows (ikona): {}", e);
            }
        } else {
            if let Err(e) = res.compile() {
                panic!("Błąd kompilacji zasobów Windows: {}", e);
            }
            println!(
                "cargo:warning=Nie znaleziono pliku ikony (.ico). Umieść go w 'resources/img/icon.ico' (lub 'resources/icon.ico' / 'icon.ico')."
            );
        }
    }
}
