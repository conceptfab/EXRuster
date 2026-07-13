use crate::processing::layer_export::{ExportFormat, ExportParams, LayerExporter};
use crate::processing::tone_mapping::ToneMapMode;
use crate::ui::progress::{ProgressSink, UiProgress};
use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{push_console, ConsoleModel};
use crate::AppWindow;
use anyhow::Result;
use slint::{ComponentHandle, Weak};
use std::path::PathBuf;
use std::sync::Arc;

/// Export configuration passed from UI
#[derive(Clone, Debug)]
pub struct UiExportConfig {
    pub format: ExportFormat,
    pub output_directory: PathBuf,
    pub base_filename: String,
    pub use_current_params: bool,
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: ToneMapMode,
}

/// Export type enumeration
#[derive(Clone, Debug)]
pub enum ExportType {
    Beauty,
    All,
    Scene,
    Objects,
    Cryptomatte,
    Lights,
}

// Convenience functions for each export type
pub fn export_beauty(ui_handle: Weak<AppWindow>, app_state: SharedAppState, console: ConsoleModel) {
    handle_export(ExportType::Beauty, ui_handle, app_state, console);
}

pub fn export_all(ui_handle: Weak<AppWindow>, app_state: SharedAppState, console: ConsoleModel) {
    handle_export(ExportType::All, ui_handle, app_state, console);
}

pub fn export_scene(ui_handle: Weak<AppWindow>, app_state: SharedAppState, console: ConsoleModel) {
    handle_export(ExportType::Scene, ui_handle, app_state, console);
}

pub fn export_objects(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    handle_export(ExportType::Objects, ui_handle, app_state, console);
}

pub fn export_cryptomatte(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    handle_export(ExportType::Cryptomatte, ui_handle, app_state, console);
}

pub fn export_lights(ui_handle: Weak<AppWindow>, app_state: SharedAppState, console: ConsoleModel) {
    handle_export(ExportType::Lights, ui_handle, app_state, console);
}

/// Create export parameters from UI state
fn create_export_params(ui: &AppWindow, config: &UiExportConfig) -> Result<ExportParams> {
    let (exposure, gamma, tonemap_mode) = if config.use_current_params {
        // Use current UI parameters
        (
            ui.get_exposure_value(),
            ui.get_gamma_value(),
            ToneMapMode::from(ui.get_tonemap_mode()),
        )
    } else {
        // Use explicit parameters
        (config.exposure, config.gamma, config.tonemap_mode)
    };

    // IMPORTANT: All exported layers should use gamma 2.2 as default for consistency
    // This ensures cryptomatte and other layers match the expected appearance
    let export_gamma = if gamma < 1.1 { 2.2 } else { gamma };

    Ok(ExportParams {
        exposure,
        gamma: export_gamma,
        tonemap_mode,
    })
}

/// Helper function to create export configuration from UI state
pub fn create_export_config_from_ui(
    ui_handle: Weak<AppWindow>,
    app_state: &SharedAppState,
    console: ConsoleModel,
) -> Option<UiExportConfig> {
    if let Some(ui) = ui_handle.upgrade() {
        // Get current file path for default output directory
        let file_path = {
            if let Ok(state) = app_state.read() {
                match state.current_file_path.as_ref() {
                    Some(path) => path.clone(),
                    None => {
                        push_console(&ui, &console, "[export] No file loaded".to_string());
                        return None;
                    }
                }
            } else {
                push_console(
                    &ui,
                    &console,
                    "[export] Failed to read app state".to_string(),
                );
                return None;
            }
        };

        let output_dir = file_path.parent().unwrap_or(&file_path).to_path_buf();
        let base_filename = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("exported")
            .to_string();

        // Read checkbox states from UI
        let apply_corrections = ui.get_export_apply_corrections();
        let use_32bit = ui.get_export_32bit();

        // Determine format based on 32-bit checkbox
        let format = if use_32bit {
            ExportFormat::Tiff32Float
        } else {
            ExportFormat::Png16
        };

        Some(UiExportConfig {
            format,
            output_directory: output_dir,
            base_filename,
            use_current_params: apply_corrections,
            exposure: if !apply_corrections {
                0.0
            } else {
                ui.get_exposure_value()
            },
            gamma: if !apply_corrections {
                2.2
            } else {
                ui.get_gamma_value()
            },
            tonemap_mode: if !apply_corrections {
                ToneMapMode::from(2)
            } else {
                ToneMapMode::from(ui.get_tonemap_mode())
            },
        })
    } else {
        None
    }
}

/// Generic export handler for all export types.
///
/// Gathers everything that needs the UI thread (config, params), then hands the
/// actual work to rayon: composing, tone-mapping, encoding and writing a
/// multi-layer 4K EXR takes seconds, and used to take them on the event loop.
pub fn handle_export(
    export_type: ExportType,
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
) {
    let Some(ui) = ui_handle.upgrade() else {
        return;
    };

    let Some(export_config) =
        create_export_config_from_ui(ui_handle.clone(), &app_state, console.clone())
    else {
        return;
    };

    let export_params = match create_export_params(&ui, &export_config) {
        Ok(p) => p,
        Err(e) => {
            push_console(&ui, &console, format!("[export] config error: {}", e));
            return;
        }
    };

    let export_name = match export_type {
        ExportType::Beauty => "beauty",
        ExportType::All => "all layers",
        ExportType::Scene => "scene layers",
        ExportType::Objects => "object layers",
        ExportType::Cryptomatte => "cryptomatte layers",
        ExportType::Lights => "light layers",
    }
    .to_string();

    push_console(&ui, &console, format!("[export] {} started", export_name));

    let progress = Arc::new(UiProgress::new(ui.as_weak()));
    progress.start_indeterminate(Some(&format!("Exporting {}", export_name)));

    let ui_weak = ui.as_weak();

    rayon::spawn(move || {
        let result = run_export(&app_state, &export_config, export_params, &export_type);

        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            match result {
                Ok(paths) => {
                    // ConsoleModel is Rc and cannot cross threads, so append to the
                    // console property directly instead.
                    let mut log = ui.get_console_text().to_string();
                    for p in &paths {
                        if !log.is_empty() {
                            log.push('\n');
                        }
                        log.push_str(&format!("[export] -> {}", p.display()));
                    }
                    ui.set_console_text(log.into());
                    ui.set_status_text(
                        format!("{} export completed ({} files)", export_name, paths.len()).into(),
                    );
                }
                Err(e) => {
                    ui.set_status_text(format!("{} export failed: {}", export_name, e).into());
                }
            }
            progress.finish(None);
        });
    });
}

/// Runs on the rayon pool — must not touch UI objects.
fn run_export(
    app_state: &SharedAppState,
    config: &UiExportConfig,
    params: ExportParams,
    export_type: &ExportType,
) -> Result<Vec<PathBuf>> {
    // Take the data source, not full_exr_cache: in lazy mode the latter is None,
    // which used to make every export fail with "No EXR cache available".
    let (source, layers_info) = {
        let state = app_state
            .read()
            .map_err(|_| anyhow::anyhow!("Failed to read app state"))?;
        let cache = state
            .image_cache
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No image loaded"))?;
        (cache.data_source(), cache.layers_info.clone())
    };

    let exporter = LayerExporter::new(source, layers_info).with_params(params);
    let dir = &config.output_directory;
    let base = &config.base_filename;

    match export_type {
        ExportType::Beauty => exporter
            .export_base_layer(config.format.clone(), dir, base)
            .map(|p| vec![p]),
        ExportType::All => exporter.export_all_layers(config.format.clone(), dir, base),
        ExportType::Scene => exporter.export_layer_group("scene", config.format.clone(), dir, base),
        ExportType::Objects => {
            exporter.export_layer_group("objects", config.format.clone(), dir, base)
        }
        ExportType::Cryptomatte => {
            exporter.export_layer_group("cryptomatte", config.format.clone(), dir, base)
        }
        ExportType::Lights => {
            exporter.export_layer_group("lights", config.format.clone(), dir, base)
        }
    }
}
