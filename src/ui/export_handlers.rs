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

/// Handle base layer export
pub fn handle_export_base_layer(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
    export_config: UiExportConfig,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let _prog = patterns::processing(ui.as_weak(), "Exporting base layer");

        match export_base_layer_impl(&ui, &full_cache, &current_file_path, export_config) {
            Ok(output_path) => {
                let msg = format!("[export] Base layer saved to: {}", output_path.display());
                push_console(&ui, &console, msg);
                ui.set_status_text("Base layer export completed".into());
            }
            Err(e) => {
                let msg = format!("[export] Failed to export base layer: {}", e);
                push_console(&ui, &console, msg);
                ui.set_status_text("Base layer export failed".into());
            }
        }
    }
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

    Ok(ExportParams {
        exposure,
        gamma,
        tonemap_mode,
    })
}

/// Helper function to show export dialog and get configuration
pub fn show_export_dialog(
    ui_handle: Weak<AppWindow>,
    current_file_path: &CurrentFilePathType,
    console: ConsoleModel,
) -> Option<UiExportConfig> {
    if let Some(ui) = ui_handle.upgrade() {
        // For now, use a simple implementation that exports to the same directory as the source file
        // In a full implementation, this would show a proper file dialog

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

        Some(UiExportConfig {
            format: ExportFormat::Png16,
            output_directory: output_dir,
            base_filename,
            use_current_params: true,
            exposure: 0.0,
            gamma: 2.2,
            tonemap_mode: 0,
        })
    } else {
        None
    }
}

/// Export base layer with PNG 16-bit format (convenience function)
pub fn export_base_layer_png16(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    if let Some(config) = show_export_dialog(ui_handle.clone(), &current_file_path, console.clone()) {
        let export_config = UiExportConfig {
            format: ExportFormat::Png16,
            ..config
        };
        
        handle_export_base_layer(ui_handle, full_cache, current_file_path, console, export_config);
    }
}

/// Export base layer with TIFF 16-bit format (convenience function)
pub fn export_base_layer_tiff16(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    if let Some(config) = show_export_dialog(ui_handle.clone(), &current_file_path, console.clone()) {
        let export_config = UiExportConfig {
            format: ExportFormat::Tiff16,
            ..config
        };
        
        handle_export_base_layer(ui_handle, full_cache, current_file_path, console, export_config);
    }
}

/// Export base layer with TIFF 32-bit float format (convenience function)
pub fn export_base_layer_tiff32_float(
    ui_handle: Weak<AppWindow>,
    full_cache: FullExrCache,
    current_file_path: CurrentFilePathType,
    console: ConsoleModel,
) {
    if let Some(config) = show_export_dialog(ui_handle.clone(), &current_file_path, console.clone()) {
        let export_config = UiExportConfig {
            format: ExportFormat::Tiff32Float,
            ..config
        };
        
        handle_export_base_layer(ui_handle, full_cache, current_file_path, console, export_config);
    }
}