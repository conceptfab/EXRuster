#![windows_subsystem = "windows"]

slint::include_modules!();

mod image_cache;
mod image_processing;
mod file_operations;
mod ui_handlers;
mod thumbnails;
mod exr_metadata;
mod progress;
mod utils;
mod color_processing;

use std::sync::{Arc, Mutex};
use crate::ui_handlers::push_console;
use ui_handlers::{ImageCacheType, CurrentFilePathType};
use slint::{VecModel, SharedString, Model};
use std::rc::Rc;

fn main() -> Result<(), slint::PlatformError> {
    // Ustaw Rayon thread pool na podstawie CPU cores
    rayon::ThreadPoolBuilder::new()
        .num_threads((num_cpus::get() - 1).max(1)) // Zostaw 1 core dla UI
        .build_global()
        .expect("Failed to initialize thread pool");

    let ui = AppWindow::new()?;

    // Windows: ustaw ikonę paska tytułu/taskbara po utworzeniu natywnego okna
    #[cfg(target_os = "windows")]
    {
        use slint::TimerMode;
        use std::cell::Cell;
        use std::rc::{Rc, Weak};
        // Opóźnij i ponawiaj próbę, aż okno zostanie utworzone
        let timer = Rc::new(slint::Timer::default());
        let timer_weak: Weak<slint::Timer> = Rc::downgrade(&timer);
        let retries = Rc::new(Cell::new(0));
        let retries_c = retries.clone();
        timer.start(TimerMode::Repeated, std::time::Duration::from_millis(150), move || {
            let done = try_set_runtime_window_icon();
            let n = retries_c.get();
            if done || n >= 40 {
                if let Some(t) = timer_weak.upgrade() { t.stop(); }
            } else {
                retries_c.set(n + 1);
            }
        });
    }
    
    let image_cache: ImageCacheType = Arc::new(Mutex::new(None));
    let current_file_path: CurrentFilePathType = Arc::new(Mutex::new(None));

    // Setup UI callbacks...
    let console_model = setup_ui_callbacks(&ui, image_cache.clone(), current_file_path.clone());

    // Obsługa argumentów uruchomieniowych: otwórz wskazany plik EXR i ewentualnie wczytaj miniatury folderu
    {
        use std::ffi::OsString;
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        if !args.is_empty() {
            if let Some(first_exr) = args.iter().find_map(|a| {
                let p = std::path::PathBuf::from(a);
                if p.is_file() {
                    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                        if ext.eq_ignore_ascii_case("exr") { return Some(p); }
                    }
                }
                None
            }) {
                ui_handlers::handle_open_exr_from_path(
                    ui.as_weak(),
                    current_file_path.clone(),
                    image_cache.clone(),
                    console_model.clone(),
                    first_exr.clone(),
                );

                if let Some(dir) = first_exr.parent() {
                    if let Ok(read) = std::fs::read_dir(dir) {
                        let exr_count = read
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| p.is_file())
                            .filter(|p| p.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("exr")).unwrap_or(false))
                            .count();

                        if exr_count > 1 {
                            // Użyj ujednoliconej funkcji wczytywania miniatur
                            ui_handlers::load_thumbnails_for_directory(ui.as_weak(), dir, console_model.clone());
                        }
                    }
                }
            }
        }
    }
    
    ui.run()
}

#[cfg(target_os = "windows")]
fn try_set_runtime_window_icon() -> bool {
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

fn setup_menu_callbacks(
    ui: &AppWindow,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console_model: Rc<VecModel<SharedString>>,
) {
    ui.on_clear_console({
        let ui_handle = ui.as_weak();
        let console_for_clear = console_model.clone();
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                console_for_clear.set_vec(vec![]);
                ui.set_console_text(SharedString::from(""));
                ui.set_status_text(SharedString::from("Console cleared"));
            }
        }
    });

    ui.on_exit({
        let ui_handle = ui.as_weak();
        move || {
            ui_handlers::handle_exit(ui_handle.clone());
        }
    });

    ui.on_open_exr({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone(); // Use console_model directly
        move || {
            ui_handlers::handle_open_exr(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });

    // Export: Convert (EXR -> TIFF)
    ui.on_export_convert({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move || {
            ui_handlers::handle_export_convert(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });

    // Export: Beauty (PNG16)
    ui.on_export_beauty({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move || {
            ui_handlers::handle_export_beauty(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });

    // Export: Channels (PNG16 grayscale)
    ui.on_export_channels({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move || {
            ui_handlers::handle_export_channels(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console.clone());
        }
    });
}

fn setup_image_control_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
    console_model: Rc<VecModel<SharedString>>,
) {
    let ui_weak_for_throttle = ui.as_weak();
    let cache_weak_for_throttle = image_cache.clone();
    let console_for_throttle = console_model.clone(); // Use console_model directly

    let throttled_updater = ui_handlers::ThrottledUpdate::new(move |exp, gamma| {
        if let Some(_ui) = ui_weak_for_throttle.upgrade() {
            ui_handlers::handle_parameter_changed_throttled(
                ui_weak_for_throttle.clone(), 
                cache_weak_for_throttle.clone(), 
                console_for_throttle.clone(),
                exp, 
                gamma
            );
        }
    });
    let throttled_update = Arc::new(Mutex::new(throttled_updater));

    ui.on_exposure_changed({
        let throttled_update = throttled_update.clone();
        
        move |exposure: f32| {
            let updater = throttled_update.lock().unwrap();
                updater.update_exposure(exposure);
        }
    });

    ui.on_gamma_changed({
        let throttled_update = throttled_update.clone();
        
        move |gamma: f32| {
            let updater = throttled_update.lock().unwrap();
            updater.update_gamma(gamma);
        }
    });

    // Tonemap mode changed
    ui.on_tonemap_mode_changed({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let console = console_model.clone();
        move |mode: i32| {
            if let Some(ui) = ui_handle.upgrade() {
                let cache_guard = ui_handlers::lock_or_recover(&image_cache);
                if let Some(ref cache) = *cache_guard {
                    let exposure = ui.get_exposure_value();
                    let gamma = ui.get_gamma_value();
                    let image = if cache.raw_pixels.len() > 2_000_000 {
                        cache.process_to_thumbnail(exposure, gamma, mode, 2048)
                    } else {
                        cache.process_to_image(exposure, gamma, mode)
                    };
                    ui.set_exr_image(image);
                    push_console(&ui, &console, format!("[preview] updated → tonemap mode: {}", mode));
                    ui.set_status_text(format!("Tonemap: {}", match mode {0=>"ACES",1=>"Reinhard",2=>"Linear", _=>"?"}).into());
                }
            }
        }
    });

    ui.on_layer_tree_clicked({
        let ui_handle = ui.as_weak();
        let image_cache = image_cache.clone();
        let current_file_path = current_file_path.clone();
        let console = console_model.clone(); // Use console_model directly
        move |clicked_item: slint::SharedString| {
            ui_handlers::handle_layer_tree_click(
                ui_handle.clone(),
                image_cache.clone(), 
                clicked_item.to_string(),
                current_file_path.clone(),
                console.clone()
            );
        }
    });
}

fn setup_panel_callbacks(
    ui: &AppWindow,
    current_file_path: CurrentFilePathType,
    image_cache: ImageCacheType,
    console_model: Rc<VecModel<SharedString>>,
) {
    // Debug klawiszy: wypisz do statusu i konsoli
    ui.on_key_pressed_debug({
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone();
        move |key: slint::SharedString| {
            if let Some(ui) = ui_handle.upgrade() {
                let k = if key.is_empty() { SharedString::from("<empty>") } else { key.clone() };
                ui.set_status_text(format!("key: {}", k).into());
                push_console(&ui, &console_model, format!("[key] {}", k));
            }
        }
    });
    ui.on_choose_working_folder({
        let ui_handle = ui.as_weak();
        let console_model = console_model.clone(); // Use console_model directly
        move || {
            if let Some(ui) = ui_handle.upgrade() {
                push_console(&ui, &console_model, "[folder] choosing working folder...".to_string());

                if let Some(dir) = crate::file_operations::open_folder_dialog() {
                    ui_handlers::load_thumbnails_for_directory(ui.as_weak(), &dir, console_model.clone());
                } else {
                    push_console(&ui, &console_model, "[folder] selection canceled".to_string());
                }
            }
        }
    });

    ui.on_open_thumbnail({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console_model = console_model.clone(); // Use console_model directly
        move |path_str: slint::SharedString| {
            if let Some(_ui) = ui_handle.upgrade() {
                let path = std::path::PathBuf::from(path_str.as_str());
                {
                    let line = SharedString::from(format!("[thumbnails] opening file {}", path.display()));
                    console_model.push(line.clone());
                }
                ui_handlers::handle_open_exr_from_path(ui_handle.clone(), current_file_path.clone(), image_cache.clone(), console_model.clone(), path);
            }
        }
    });

    // Nawigacja miniatur klawiszami (delta: -1 wstecz, +1 dalej)
    ui.on_navigate_thumbnails({
        let ui_handle = ui.as_weak();
        let current_file_path = current_file_path.clone();
        let image_cache = image_cache.clone();
        let console_model = console_model.clone();
        move |delta: i32| {
            if delta == 0 { return; }
            if let Some(ui) = ui_handle.upgrade() {
                let model = ui.get_thumbnails();
                let count = model.row_count();
                if count == 0 { return; }

                let current_path = ui.get_opened_thumbnail_path().to_string();
                let mut idx: i32 = -1;
                if !current_path.is_empty() {
                    for i in 0..count {
                        if let Some(item) = model.row_data(i) {
                            if item.path.as_str() == current_path {
                                idx = i as i32;
                                break;
                            }
                        }
                    }
                }

                let next_idx: i32 = if idx >= 0 {
                    (idx + delta).rem_euclid(count as i32)
                } else {
                    if delta > 0 { 0 } else { (count as i32) - 1 }
                };

                if let Some(item) = model.row_data(next_idx as usize) {
                    ui.set_opened_thumbnail_path(item.path.clone());
                    let path = std::path::PathBuf::from(item.path.as_str());
                    ui_handlers::handle_open_exr_from_path(
                        ui.as_weak(),
                        current_file_path.clone(),
                        image_cache.clone(),
                        console_model.clone(),
                        path,
                    );
                }
            }
        }
    });
}

fn setup_ui_callbacks(
    ui: &AppWindow,
    image_cache: ImageCacheType,
    current_file_path: CurrentFilePathType,
) -> Rc<VecModel<SharedString>> {
    let console_model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(vec![]));
    ui.set_console_text(SharedString::from(""));

    setup_menu_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone());
    setup_image_control_callbacks(ui, image_cache.clone(), current_file_path.clone(), console_model.clone());
    setup_panel_callbacks(ui, current_file_path.clone(), image_cache.clone(), console_model.clone());

    console_model
}