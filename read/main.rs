// Level 3 Optimization: Custom memory allocator removed due to Windows MSVC compatibility
// All other Level 3 optimizations (SIMD, lock-free, custom parser) remain active

use std::fs;
use std::path::Path;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use serde::{Deserialize, Serialize};
use rayon::prelude::*;
use once_cell::sync::Lazy;
// Level 2 optimizations imports
use tokio::io::AsyncWriteExt;
use tokio::fs as async_fs;
// Level 3 optimizations imports
use dashmap::DashMap;

mod fast_exr;
mod simd_patterns;

// String interning cache for group names to avoid repeated allocations
static GROUP_NAME_CACHE: Lazy<HashMap<&'static str, String>> = Lazy::new(|| {
    let mut cache = HashMap::new();
    cache.insert("base", "Base".to_string());
    cache.insert("scene", "Scene".to_string());
    cache.insert("technical", "Technical".to_string());
    cache.insert("light", "Light".to_string());
    cache.insert("cryptomatte", "Cryptomatte".to_string());
    cache.insert("scene_objects", "Scene Objects".to_string());
    cache.insert("basic_rgb", "Basic RGB".to_string());
    cache.insert("other", "Other".to_string());
    cache
});

#[derive(Deserialize, Serialize)]
struct ConfigSettings {
    basic_rgb_channels: Vec<String>,
    group_priority_order: Vec<String>,
    fallback_names: FallbackNames,
    paths: ConfigPaths,
}

#[derive(Deserialize, Serialize)]
struct FallbackNames {
    basic_rgb: String,
    default: String,
}

#[derive(Deserialize, Serialize)]
struct ConfigPaths {
    data_folder: String,
}

#[derive(Deserialize, Serialize)]
struct GroupDefinition {
    name: String,
    #[serde(default)]
    prefixes: Vec<String>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    basic_rgb: bool,
}

#[derive(Deserialize, Serialize)]
struct ChannelGroupConfig {
    config: ConfigSettings,
    groups: HashMap<String, GroupDefinition>,
    default_group: String,
}

fn create_default_config() -> ChannelGroupConfig {
    let mut groups = HashMap::new();
    
    groups.insert("base".to_string(), GroupDefinition {
        name: "Base".to_string(),
        prefixes: vec!["Beauty".to_string()],
        patterns: vec![],
        basic_rgb: true,
    });
    
    groups.insert("scene".to_string(), GroupDefinition {
        name: "Scene".to_string(),
        prefixes: vec!["Background".to_string(), "Translucency".to_string(), "Translucency0".to_string(), "VirtualBeauty".to_string(), "ZDepth".to_string()],
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
    
    
    ChannelGroupConfig {
        config: ConfigSettings {
            basic_rgb_channels: vec!["R".to_string(), "G".to_string(), "B".to_string(), "A".to_string()],
            group_priority_order: vec!["cryptomatte".to_string(), "light".to_string(), "scene".to_string(), "technical".to_string(), "scene_objects".to_string()],
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

fn load_channel_config() -> std::result::Result<ChannelGroupConfig, Box<dyn std::error::Error>> {
    let config_path = "channel_groups.json";
    
    if !std::path::Path::new(config_path).exists() {
        println!("Creating default config file: {}", config_path);
        let default_config = create_default_config();
        let json_content = serde_json::to_string_pretty(&default_config)?;
        fs::write(config_path, json_content)?;
        return Ok(default_config);
    }
    
    let config_content = fs::read_to_string(config_path)?;
    let config: ChannelGroupConfig = serde_json::from_str(&config_content)?;
    Ok(config)
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config = load_channel_config().unwrap_or_else(|e| {
        eprintln!("Warning: Could not load config: {}. Using default.", e);
        create_default_config()
    });
    
    let data_folder = &config.config.paths.data_folder;
    
    if !Path::new(data_folder).exists() {
        eprintln!("Data folder '{}' does not exist", data_folder);
        return Ok(());
    }
    
    let start_time = Instant::now();
    
    // Collect all EXR files first
    let exr_files: Vec<_> = fs::read_dir(data_folder)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|s| s.to_str())
                .map(|ext| ext.to_lowercase() == "exr")
                .unwrap_or(false)
        })
        .collect();
    
    println!("Found {} EXR files to process", exr_files.len());
    
    // Share config between threads
    let config = Arc::new(config);
    
    // Collect progress messages to reduce console locking
    let progress_messages = Arc::new(Mutex::new(Vec::new()));
    
    // Process files in parallel
    let results: Vec<_> = exr_files
        .par_iter()
        .map(|path| {
            let file_start = Instant::now();
            let progress_msgs = progress_messages.clone();
            
            let result = process_exr_file(path, &config);
            let duration = file_start.elapsed();
            
            match result {
                Ok(()) => {
                    let msg = format!("✓ Processed {} in {:.2}s", path.display(), duration.as_secs_f64());
                    progress_msgs.lock().unwrap().push(msg);
                    Ok(path.file_name().unwrap_or_default().to_string_lossy().to_string())
                }
                Err(e) => {
                    let msg = format!("✗ Error processing {}: {}", path.display(), e);
                    progress_msgs.lock().unwrap().push(msg);
                    Err(format!("Error in {}: {}", path.display(), e))
                }
            }
        })
        .collect();
    
    // Print all progress messages at once
    let messages = progress_messages.lock().unwrap();
    for msg in messages.iter() {
        println!("{}", msg);
    }
    
    let total_duration = start_time.elapsed();
    let successful = results.iter().filter(|r| r.is_ok()).count();
    let failed = results.iter().filter(|r| r.is_err()).count();
    
    println!("\n📊 Processing complete:");
    println!("  ✓ Successful: {}", successful);
    println!("  ✗ Failed: {}", failed);
    println!("  ⏱️  Total time: {:.2}s", total_duration.as_secs_f64());
    println!("  🚀 Avg per file: {:.2}s", total_duration.as_secs_f64() / exr_files.len() as f64);
    
    Ok(())
}

fn process_exr_file(exr_path: &Path, config: &Arc<ChannelGroupConfig>) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Level 3 Optimization: Ultra-fast custom EXR parser (bypasses all pixel data)
    let fast_metadata = fast_exr::read_exr_metadata_ultra_fast(exr_path)?;
    
    let file_stem = exr_path.file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Invalid file name")?;
    
    // Level 2 Optimization: Build content in memory first, then write asynchronously
    let mut content = String::new();
    
    content.push_str(&format!("EXR File Analysis: {}\n", exr_path.display()));
    content.push_str("==========================================\n\n");
    
    // Level 3: Use fast metadata structure
    content.push_str("Image Attributes:\n");
    content.push_str(&format!("  Display Window: {:?}\n", fast_metadata.display_window));
    content.push_str(&format!("  Pixel Aspect Ratio: {}\n", fast_metadata.pixel_aspect));
    content.push_str(&format!("  Compression: {}\n", fast_metadata.compression));
    content.push_str(&format!("  Line Order: {}\n", fast_metadata.line_order));
    if let Some(layer_name) = &fast_metadata.layer_name {
        content.push_str(&format!("  Layer Name: {}\n", layer_name));
    }
    content.push('\n');
    
    if !fast_metadata.custom_attributes.is_empty() {
        content.push_str("Custom Attributes:\n");
        for (name, value) in &fast_metadata.custom_attributes {
            content.push_str(&format!("  {}: {}\n", name, value));
        }
        content.push('\n');
    }
    
    // Level 3 Optimization: Process single layer with lock-free data structures
    content.push_str("Layer 1 Information:\n");
    if let Some(layer_name) = &fast_metadata.layer_name {
        content.push_str(&format!("  Layer Name: {}\n", layer_name));
    }
    content.push_str(&format!("  Compression: {}\n", fast_metadata.compression));
    content.push_str(&format!("  Line Order: {}\n", fast_metadata.line_order));
    content.push('\n');
    
    content.push_str("  Channel Groups:\n");
    
    // Level 3: Lock-free parallel channel grouping with DashMap
    let channel_groups: DashMap<String, Vec<&fast_exr::ChannelInfo>> = DashMap::new();
    
    // Process channels in parallel without locks
    fast_metadata.channels
        .par_iter()
        .for_each(|channel| {
            let group_name = determine_channel_group_ultra_fast_with_config(&channel.name, config);
            channel_groups.entry(group_name).or_insert_with(Vec::new).push(channel);
        });
    
    // Convert to sorted output (only for display)
    let mut sorted_groups: Vec<_> = channel_groups.into_iter().collect();
    sorted_groups.sort_by(|a, b| a.0.cmp(&b.0));
    
    for (group_name, channels) in sorted_groups {
        content.push_str(&format!("    {} Channels:\n", group_name));
        for channel in channels {
            content.push_str(&format!("      {}\n", channel.name));
            content.push_str(&format!("        Sample Type: {:?}\n", channel.sample_type));
            content.push_str(&format!("        Sampling: {:?}\n", channel.sampling));
            content.push_str(&format!("        Quantize Linearly: {}\n", channel.quantize_linearly));
        }
        content.push('\n');
    }
    
    // Level 2 Optimization: Async file writing
    let output_path = format!("{}.txt", file_stem);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    
    rt.block_on(async {
        let mut file = async_fs::File::create(&output_path).await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await
    })?;
    Ok(())
}

fn determine_channel_group(channel_name: &str, config: &Arc<ChannelGroupConfig>) -> String {
    // Check for basic RGB channels first (use cached string)
    if ["R", "G", "B", "A"].contains(&channel_name) {
        for group_def in config.groups.values() {
            if group_def.basic_rgb {
                return GROUP_NAME_CACHE.get("base").cloned()
                    .unwrap_or_else(|| group_def.name.clone());
            }
        }
        return GROUP_NAME_CACHE.get("basic_rgb").cloned()
            .unwrap_or_else(|| config.config.fallback_names.basic_rgb.clone());
    }
    
    let prefix = if let Some(dot_pos) = channel_name.find('.') {
        &channel_name[..dot_pos]
    } else {
        channel_name
    };
    
    // Check specific groups in priority order
    for group_key in &config.config.group_priority_order {
        if let Some(group_def) = config.groups.get(group_key) {
            // Check exact prefix matches (use cached strings when possible)
            for prefix_str in &group_def.prefixes {
                if prefix == prefix_str {
                    return GROUP_NAME_CACHE.get(group_key.as_str()).cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
            
            // Check pattern matches
            for pattern in &group_def.patterns {
                if matches_pattern(prefix, pattern) {
                    return GROUP_NAME_CACHE.get(group_key.as_str()).cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
        }
    }
    
    // Default to Scene Objects for unknown channels
    if let Some(_scene_objects_group) = config.groups.get("scene_objects") {
        GROUP_NAME_CACHE.get("scene_objects").cloned()
            .unwrap_or_else(|| config.config.fallback_names.default.clone())
    } else {
        GROUP_NAME_CACHE.get("other").cloned()
            .unwrap_or_else(|| config.config.fallback_names.default.clone())
    }
}

// Level 3: Ultra-fast channel group determination combining SIMD + configuration
fn determine_channel_group_ultra_fast_with_config(channel_name: &str, config: &Arc<ChannelGroupConfig>) -> String {
    // Level 3 Fast path: Use SIMD-optimized pattern matching for common cases
    let fast_group = simd_patterns::determine_channel_group_ultra_fast(channel_name);
    if fast_group != "scene_objects" {
        // Return cached string to avoid allocations
        return GROUP_NAME_CACHE.get(fast_group).cloned()
            .unwrap_or_else(|| fast_group.to_string());
    }
    
    // Fall back to configuration-based matching for edge cases
    determine_channel_group(channel_name, config)
}

fn matches_pattern(text: &str, pattern: &str) -> bool {
    // Level 3: Use SIMD-optimized pattern matching
    simd_patterns::matches_pattern_simd(text, pattern)
}