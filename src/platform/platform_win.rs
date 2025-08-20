#[cfg(target_os = "windows")]
pub fn try_set_runtime_window_icon() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetSystemMetrics, LoadImageW, SendMessageW, SetClassLongPtrW, GCLP_HICON,
        GCLP_HICONSM, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_LOADFROMFILE, SM_CXICON, SM_CYICON,
        WM_SETICON,
    };

    // Znajdź uchwyt okna po tytule ustawionym w `ui/appwindow.slint`
    let title_wide: Vec<u16> = OsStr::new("EXRuster").encode_wide().chain(Some(0)).collect();
    unsafe {
        let hwnd = match FindWindowW(PCWSTR(std::ptr::null()), PCWSTR(title_wide.as_ptr())) {
            Ok(h) => h,
            Err(_) => return false,
        };
        if hwnd.0.is_null() {
            return false;
        }

        // Poszukaj ikony w kilku lokalizacjach (relatywnie do CWD i do katalogu exe)
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let mut candidates: Vec<std::path::PathBuf> = vec![
            Path::new("resources/img/icon.ico").to_path_buf(),
            Path::new("resources/icon.ico").to_path_buf(),
            Path::new("icon.ico").to_path_buf(),
        ];
        if let Some(ed) = &exe_dir {
            candidates.push(ed.join("resources/img/icon.ico"));
            candidates.push(ed.join("resources/icon.ico"));
            candidates.push(ed.join("icon.ico"));
        }

        if let Some(icon_path) = candidates.into_iter().find(|p| p.exists()) {
            // Załaduj wielkość zgodnie z metrykami systemowymi
            let big_w = GetSystemMetrics(SM_CXICON);
            let big_h = GetSystemMetrics(SM_CYICON);

            let path_wide: Vec<u16> =
                OsStr::new(icon_path.as_os_str()).encode_wide().chain(Some(0)).collect();
            let hicon =
                match LoadImageW(None, PCWSTR(path_wide.as_ptr()), IMAGE_ICON, big_w, big_h, LR_LOADFROMFILE) {
                    Ok(h) => h,
                    Err(_) => return false,
                };

            if !hicon.0.is_null() {
                // Ustawienie na poziomie instancji klasy okna (fallback gdy WM_SETICON nie działa)
                if GetModuleHandleW(None).is_ok() {
                    let _ = SetClassLongPtrW(hwnd, GCLP_HICON, hicon.0 as isize);
                    let _ = SetClassLongPtrW(hwnd, GCLP_HICONSM, hicon.0 as isize);
                }

                // Spróbuj też przez WM_SETICON (niektóre toolkit-y reagują dopiero po tym)
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(hicon.0 as isize)),
                );
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(LPARAM(hicon.0 as isize)),
                );
                return true;
            }
        }
    }
    false
}
