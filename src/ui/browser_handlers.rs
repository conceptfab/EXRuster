use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{push_console, ConsoleModel};
use crate::{log_error, log_warn, AppWindow};
use slint::Weak;
use std::path::PathBuf;

/// Copy the absolute path of the currently open EXR to the system clipboard.
pub fn handle_copy_current_path(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };

    let path_opt = app_state
        .read()
        .ok()
        .and_then(|s| s.current_file_path.clone());

    let Some(path) = path_opt else {
        ui.set_status_text("No file open — nothing to copy".into());
        push_console(&ui, &console, "[copy-path] no file open".to_string());
        return;
    };

    let text = path.display().to_string();
    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone())) {
        Ok(()) => {
            ui.set_status_text(format!("Path copied: {}", text).into());
            push_console(&ui, &console, format!("[copy-path] copied {}", text));
        }
        Err(e) => {
            log_error!("Clipboard error: {}", e);
            ui.set_status_text(format!("Clipboard error: {}", e).into());
        }
    }
}

/// Prompt the user for a destination folder and copy the currently open EXR there.
pub fn handle_copy_current_file_to(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    let Some(ui) = ui_handle.upgrade() else { return; };

    let src_opt: Option<PathBuf> = app_state
        .read()
        .ok()
        .and_then(|s| s.current_file_path.clone());

    let Some(src) = src_opt else {
        ui.set_status_text("No file open — nothing to copy".into());
        return;
    };

    let Some(dest_dir) = rfd::FileDialog::new()
        .set_title("Select destination folder")
        .pick_folder()
    else {
        push_console(&ui, &console, "[copy-file] canceled".to_string());
        return;
    };

    let file_name = match src.file_name() {
        Some(n) => n.to_owned(),
        None => {
            log_warn!("Source path has no file name: {}", src.display());
            return;
        }
    };
    let dest = dest_dir.join(&file_name);

    match std::fs::copy(&src, &dest) {
        Ok(bytes) => {
            ui.set_status_text(format!("Copied {} bytes to {}", bytes, dest.display()).into());
            push_console(
                &ui,
                &console,
                format!("[copy-file] {} -> {}", src.display(), dest.display()),
            );
        }
        Err(e) => {
            log_error!("Copy failed: {}", e);
            ui.set_status_text(format!("Copy failed: {}", e).into());
        }
    }
}
