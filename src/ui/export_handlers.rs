use slint::{Weak, ComponentHandle};
use std::path::PathBuf;
use anyhow::Result;
use crate::AppWindow;
use crate::processing::layer_export::{LayerExporter, ExportFormat, ExportParams};
use crate::processing::tone_mapping::ToneMapMode;
use crate::ui::ui_handlers::{push_console, lock_or_recover, ConsoleModel, FullExrCache, CurrentFilePathType};
use crate::ui::progress::patterns;

/// Export configuration passed from UI
#[derive(Clone, Debug)]
pub struct UiExportConfig {
    pub format: ExportFormat,
    pub output_directory: PathBuf,
    pub base_filename: String,
    pub use_current_params: bool,
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: i32,
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
pub fn export_beauty(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    handle_export(ExportType::Beauty, ui_handle, full_cache, current_file_path, console);
}

pub fn export_all(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    handle_export(ExportType::All, ui_handle, full_cache, current_file_path, console);
}

pub fn export_scene(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    handle_export(ExportType::Scene, ui_handle, full_cache, current_file_path, console);
}

pub fn export_objects(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    handle_export(ExportType::Objects, ui_handle, full_cache, current_file_path, console);
}

pub fn export_cryptomatte(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    handle_export(ExportType::Cryptomatte, ui_handle, full_cache, current_file_path, console);
}

pub fn export_lights(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    handle_export(ExportType::Lights, ui_handle, full_cache, current_file_path, console);
}


/// Implementation for base layer export
fn export_base_layer_impl(
    ui: &AppWindow,
    full_cache: &FullExrCache,
    current_file_path: &CurrentFilePathType,
    export_config: UiExportConfig,
) -> Result<PathBuf> {
    // Get current file path
    let file_path = {
        let guard = lock_or_recover(current_file_path);
        guard.clone().ok_or_else(|| anyhow::anyhow!("No file loaded"))?
    };

    // Get full cache data
    let cache_data = {
        let guard = lock_or_recover(full_cache);
        guard.clone().ok_or_else(|| anyhow::anyhow!("No EXR cache available"))?
    };

    // Extract layers info
    let layers_info = crate::io::image_cache::extract_layers_info(&file_path)?;

    // Create export parameters
    let export_params = create_export_params(ui, &export_config)?;

    // Create exporter
    let exporter = LayerExporter::new(cache_data, layers_info)
        .with_params(export_params);

    // Export base layer
    exporter.export_base_layer(
        export_config.format,
        &export_config.output_directory,
        &export_config.base_filename,
    )
}


/// Create export parameters from UI state
fn create_export_params(ui: &AppWindow, config: &UiExportConfig) -> Result<ExportParams> {
    let (exposure, gamma, tonemap_mode) = if config.use_current_params {
        // Use current UI parameters
        (
            ui.get_exposure_value(),
            ui.get_gamma_value(),
            ui.get_tonemap_mode() as i32,
        )
    } else {
        // Use explicit parameters
        (config.exposure, config.gamma, config.tonemap_mode)
    };

    let tonemap_mode = ToneMapMode::from(tonemap_mode);

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
    current_file_path: &CurrentFilePathType,
    console: ConsoleModel,
) -> Option<UiExportConfig> {
    if let Some(ui) = ui_handle.upgrade() {
        // Get current file path for default output directory
        let file_path = {
            let guard = lock_or_recover(current_file_path);
            match guard.as_ref() {
                Some(path) => path.clone(),
                None => {
                    push_console(&ui, &console, "[export] No file loaded".to_string());
                    return None;
                }
            }
        };

        let output_dir = file_path.parent().unwrap_or(&file_path).to_path_buf();
        let base_filename = file_path.file_stem()
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
            exposure: if !apply_corrections { 0.0 } else { ui.get_exposure_value() },
            gamma: if !apply_corrections { 2.2 } else { ui.get_gamma_value() },
            tonemap_mode: if !apply_corrections { 2 } else { ui.get_tonemap_mode() as i32 },
        })
    } else {
        None
    }
}

/// Generic export handler for all export types
pub fn handle_export(
    export_type: ExportType,
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let export_config = match create_export_config_from_ui(ui_handle.clone(), &current_file_path, console.clone()) {
            Some(config) => config,
            None => return,
        };

        let export_name = match export_type {
            ExportType::Beauty => "beauty",
            ExportType::All => "all layers",
            ExportType::Scene => "scene layers",
            ExportType::Objects => "object layers",
            ExportType::Cryptomatte => "cryptomatte layers",
            ExportType::Lights => "light layers",
        };

        let _prog = patterns::processing(ui.as_weak(), &format!("Exporting {}", export_name));

        match export_type {
            ExportType::Beauty => {
                match export_base_layer_impl(&ui, &full_cache, &current_file_path, export_config) {
                    Ok(output_path) => {
                        let msg = format!("[export] {} exported to: {}", export_name, output_path.display());
                        push_console(&ui, &console, msg);
                        ui.set_status_text(format!("{} export completed", export_name).into());
                    }
                    Err(e) => {
                        let msg = format!("[export] Failed to export {}: {}", export_name, e);
                        push_console(&ui, &console, msg);
                        ui.set_status_text(format!("{} export failed", export_name).into());
                    }
                }
            }
            _ => {
                match export_layer_group_impl(&ui, &full_cache, &current_file_path, export_config, export_type) {
                    Ok(output_paths) => {
                        let msg = format!("[export] {} exported {} layers", export_name, output_paths.len());
                        push_console(&ui, &console, msg);
                        for path in output_paths {
                            push_console(&ui, &console, format!("  -> {}", path.display()));
                        }
                        ui.set_status_text(format!("{} export completed", export_name).into());
                    }
                    Err(e) => {
                        let msg = format!("[export] Failed to export {}: {}", export_name, e);
                        push_console(&ui, &console, msg);
                        ui.set_status_text(format!("{} export failed", export_name).into());
                    }
                }
            }
        }
    }
}

/// Implementation for layer group export
fn export_layer_group_impl(
    ui: &AppWindow,
    full_cache: &FullExrCache,
    current_file_path: &CurrentFilePathType,
    export_config: UiExportConfig,
    export_type: ExportType,
) -> Result<Vec<PathBuf>> {
    // Get current file path
    let file_path = {
        let guard = lock_or_recover(current_file_path);
        guard.clone().ok_or_else(|| anyhow::anyhow!("No file loaded"))?
    };

    // Get full cache data
    let cache_data = {
        let guard = lock_or_recover(full_cache);
        guard.clone().ok_or_else(|| anyhow::anyhow!("No EXR cache available"))?
    };

    // Extract layers info
    let layers_info = crate::io::image_cache::extract_layers_info(&file_path)?;

    // Create export parameters
    let export_params = create_export_params(ui, &export_config)?;

    // Create exporter
    let exporter = LayerExporter::new(cache_data, layers_info)
        .with_params(export_params);

    match export_type {
        ExportType::All => {
            exporter.export_all_layers(
                export_config.format,
                &export_config.output_directory,
                &export_config.base_filename,
            )
        }
        ExportType::Scene => {
            exporter.export_layer_group(
                "scene",
                export_config.format,
                &export_config.output_directory,
                &export_config.base_filename,
            )
        }
        ExportType::Objects => {
            exporter.export_layer_group(
                "objects",
                export_config.format,
                &export_config.output_directory,
                &export_config.base_filename,
            )
        }
        ExportType::Cryptomatte => {
            exporter.export_layer_group(
                "cryptomatte",
                export_config.format,
                &export_config.output_directory,
                &export_config.base_filename,
            )
        }
        ExportType::Lights => {
            exporter.export_layer_group(
                "lights",
                export_config.format,
                &export_config.output_directory,
                &export_config.base_filename,
            )
        }
        ExportType::Beauty => unreachable!("Beauty export should use base layer function"),
    }
}