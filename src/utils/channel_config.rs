/// Zarządzanie konfiguracją grupowania kanałów z obsługą słownika JSON
/// Automatyczne tworzenie z hardkodowanego szablonu jeśli plik nie istnieje

use std::path::Path;
use std::fs;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::processing::channel_classification::{ChannelGroupConfig, GroupDefinition, FallbackNames};

/// Ścieżka do pliku konfiguracyjnego grupowania kanałów
pub const CHANNEL_CONFIG_PATH: &str = "channel_groups.json";

/// Struktura konfiguracji zgodna z plikiem JSON
#[derive(Deserialize, Serialize)]
pub struct ConfigFile {
    config: ConfigSettings,
    groups: HashMap<String, GroupDefinition>,
    default_group: String,
}

#[derive(Deserialize, Serialize)]
struct ConfigSettings {
    basic_rgb_channels: Vec<String>,
    group_priority_order: Vec<String>,
    fallback_names: FallbackNames,
    paths: ConfigPaths,
}

#[derive(Deserialize, Serialize)]
struct ConfigPaths {
    data_folder: String,
}

/// Ładuje konfigurację grupowania kanałów z pliku JSON
/// Tworzy plik z domyślną konfiguracją jeśli nie istnieje
pub fn load_channel_config() -> Result<ChannelGroupConfig> {
    if !Path::new(CHANNEL_CONFIG_PATH).exists() {
        println!("Creating default channel groups config file: {}", CHANNEL_CONFIG_PATH);
        let default_config = create_default_config_file();
        save_channel_config(&default_config)?;
        return convert_to_channel_group_config(default_config);
    }
    
    let config_content = fs::read_to_string(CHANNEL_CONFIG_PATH)
        .with_context(|| format!("Failed to read config file: {}", CHANNEL_CONFIG_PATH))?;
    
    let config_file: ConfigFile = serde_json::from_str(&config_content)
        .with_context(|| format!("Failed to parse JSON config: {}", CHANNEL_CONFIG_PATH))?;
    
    convert_to_channel_group_config(config_file)
}

/// Zapisuje konfigurację do pliku JSON
pub fn save_channel_config(config: &ConfigFile) -> Result<()> {
    let json_content = serde_json::to_string_pretty(config)
        .context("Failed to serialize config to JSON")?;
    
    fs::write(CHANNEL_CONFIG_PATH, json_content)
        .with_context(|| format!("Failed to write config file: {}", CHANNEL_CONFIG_PATH))?;
    
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
        config: ConfigSettings {
            basic_rgb_channels: vec!["R".to_string(), "G".to_string(), "B".to_string(), "A".to_string()],
            group_priority_order: vec![
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
            paths: ConfigPaths {
                data_folder: "data".to_string(),
            },
        },
        groups,
        default_group: "Other".to_string(),
    }
}

/// Konwertuje ConfigFile na ChannelGroupConfig używany przez processing module
fn convert_to_channel_group_config(config_file: ConfigFile) -> Result<ChannelGroupConfig> {
    Ok(ChannelGroupConfig {
        basic_rgb_channels: config_file.config.basic_rgb_channels,
        group_priority_order: config_file.config.group_priority_order,
        fallback_names: config_file.config.fallback_names,
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