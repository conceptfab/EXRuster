/// Zarządzanie konfiguracją grupowania kanałów z obsługą słownika JSON
/// Automatyczne tworzenie z hardkodowanego szablonu jeśli plik nie istnieje

use std::path::PathBuf;
use std::fs;
use std::env;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::processing::channel_classification::{ChannelGroupConfig, GroupDefinition, FallbackNames};

/// Zwraca ścieżkę do pliku konfiguracyjnego w katalogu .exe
pub fn get_channel_config_path() -> Result<PathBuf> {
    let exe_path = env::current_exe()
        .with_context(|| "Failed to get executable path")?;
    let exe_dir = exe_path.parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to get executable directory"))?;
    Ok(exe_dir.join("channel_groups.json"))
}

/// Simplified configuration structure - consolidated from multiple nested structs
#[derive(Deserialize, Serialize)]
pub struct ConfigFile {
    basic_rgb_channels: Vec<String>,
    group_priority_order: Vec<String>,
    fallback_names: FallbackNames,
    groups: HashMap<String, GroupDefinition>,
    default_group: String,
    data_folder: String,
}

/// Ładuje konfigurację grupowania kanałów z pliku JSON
/// Tworzy plik z domyślną konfiguracją jeśli nie istnieje
pub fn load_channel_config() -> Result<ChannelGroupConfig> {
    let config_path = get_channel_config_path()?;
    
    if !config_path.exists() {
        println!("Creating default channel groups config file: {}", config_path.display());
        let default_config = create_default_config_file();
        save_channel_config(&default_config)?;
        return convert_to_channel_group_config(default_config);
    }
    
    let config_content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;
    
    let config_file: ConfigFile = serde_json::from_str(&config_content)
        .with_context(|| format!("Failed to parse JSON config: {}", config_path.display()))?;
    
    convert_to_channel_group_config(config_file)
}

/// Zapisuje konfigurację do pliku JSON
pub fn save_channel_config(config: &ConfigFile) -> Result<()> {
    let config_path = get_channel_config_path()?;
    let json_content = serde_json::to_string_pretty(config)
        .context("Failed to serialize config to JSON")?;
    
    fs::write(&config_path, json_content)
        .with_context(|| format!("Failed to write config file: {}", config_path.display()))?;
    
    Ok(())
}

/// Tworzy domyślną konfigurację zgodną z hardkodowanym szablonem
fn create_default_config_file() -> ConfigFile {
    let mut groups = HashMap::new();
    
    groups.insert("base".to_string(), GroupDefinition {
        name: "Base".to_string(),
        prefixes: vec!["Beauty".to_string()],
        patterns: vec![],
        basic_rgb: true,
    });
    
    groups.insert("scene".to_string(), GroupDefinition {
        name: "Scene".to_string(),
        prefixes: vec![
            "Background".to_string(), 
            "Translucency".to_string(), 
            "Translucency0".to_string(), 
            "VirtualBeauty".to_string(), 
            "ZDepth".to_string()
        ],
        patterns: vec![],
        basic_rgb: false,
    });
    
    groups.insert("technical".to_string(), GroupDefinition {
        name: "Technical".to_string(),
        prefixes: vec!["RenderStamp".to_string(), "RenderStamp0".to_string()],
        patterns: vec![],
        basic_rgb: false,
    });
    
    groups.insert("light".to_string(), GroupDefinition {
        name: "Light".to_string(),
        prefixes: vec!["Sky".to_string(), "Sun".to_string(), "LightMix".to_string()],
        patterns: vec!["Light*".to_string()],
        basic_rgb: false,
    });
    
    groups.insert("cryptomatte".to_string(), GroupDefinition {
        name: "Cryptomatte".to_string(),
        prefixes: vec!["Cryptomatte".to_string(), "Cryptomatte0".to_string()],
        patterns: vec![],
        basic_rgb: false,
    });
    
    groups.insert("scene_objects".to_string(), GroupDefinition {
        name: "Scene Objects".to_string(),
        prefixes: vec![],
        patterns: vec!["ID*".to_string(), "_*".to_string()],
        basic_rgb: false,
    });
    
    ConfigFile {
        basic_rgb_channels: vec!["R".to_string(), "G".to_string(), "B".to_string(), "A".to_string()],
        group_priority_order: vec![
            "base".to_string(),
            "cryptomatte".to_string(), 
            "light".to_string(), 
            "scene".to_string(), 
            "technical".to_string(), 
            "scene_objects".to_string()
        ],
        fallback_names: FallbackNames {
            basic_rgb: "Basic RGB".to_string(),
            default: "Other".to_string(),
        },
        groups,
        default_group: "Other".to_string(),
        data_folder: ".".to_string(),
    }
}

/// Konwertuje ConfigFile na ChannelGroupConfig używany przez processing module
fn convert_to_channel_group_config(config_file: ConfigFile) -> Result<ChannelGroupConfig> {
    Ok(ChannelGroupConfig {
        basic_rgb_channels: config_file.basic_rgb_channels,
        group_priority_order: config_file.group_priority_order,
        fallback_names: config_file.fallback_names,
        groups: config_file.groups,
    })
}

/// Fallback funkcja - zwraca domyślną konfigurację w przypadku błędu
pub fn get_fallback_config() -> ChannelGroupConfig {
    let config_file = create_default_config_file();
    convert_to_channel_group_config(config_file)
        .unwrap_or_else(|_| crate::processing::channel_classification::create_default_config())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_default_config() {
        let config = create_default_config_file();
        assert!(!config.groups.is_empty());
        assert!(config.groups.contains_key("base"));
        assert!(config.groups.contains_key("light"));
        assert!(config.groups.contains_key("cryptomatte"));
    }

    #[test]
    fn test_convert_to_channel_group_config() {
        let config_file = create_default_config_file();
        let result = convert_to_channel_group_config(config_file);
        assert!(result.is_ok());
        
        let channel_config = result.unwrap();
        assert!(!channel_config.groups.is_empty());
        assert!(!channel_config.basic_rgb_channels.is_empty());
    }
}