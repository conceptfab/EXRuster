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

    let state = match app_state.read() {
        Ok(s) => s,
        Err(e) => {
            log_error!("App state lock poisoned: {}", e);
            ui.set_status_text("Internal error: app state inaccessible".into());
            return;
        }
    };
    let path_opt = state.current_file_path.clone();
    drop(state); // release lock before doing I/O

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

    let state = match app_state.read() {
        Ok(s) => s,
        Err(e) => {
            log_error!("App state lock poisoned: {}", e);
            ui.set_status_text("Internal error: app state inaccessible".into());
            return;
        }
    };
    let src_opt: Option<PathBuf> = state.current_file_path.clone();
    drop(state); // release lock before doing I/O

    let Some(src) = src_opt else {
        ui.set_status_text("No file open — nothing to copy".into());
        push_console(&ui, &console, "[copy-file] no file open".to_string());
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

    if dest.exists() {
        ui.set_status_text(format!("Destination already exists: {}", dest.display()).into());
        push_console(
            &ui,
            &console,
            format!("[copy-file] refused: destination exists {}", dest.display()),
        );
        return;
    }

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
