/// Ultra-fast channel classification using SIMD patterns and dictionary lookups
/// Based on the optimized implementation from read/simd_patterns.rs

use std::collections::HashMap;
use once_cell::sync::Lazy;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Configuration for channel grouping
#[derive(Deserialize, Serialize, Clone)]
pub struct ChannelGroupConfig {
    pub basic_rgb_channels: Vec<String>,
    pub group_priority_order: Vec<String>,
    pub fallback_names: FallbackNames,
    pub groups: HashMap<String, GroupDefinition>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct FallbackNames {
    pub basic_rgb: String,
    pub default: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct GroupDefinition {
    pub name: String,
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub basic_rgb: bool,
}

/// Pre-computed hash-based pattern matching for ultra-fast channel classification
static CHANNEL_PREFIX_MAP: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut map = HashMap::new();
    
    // Base channels
    map.insert("Beauty", "base");
    map.insert("R", "base");
    map.insert("G", "base");
    map.insert("B", "base");
    map.insert("A", "base");
    
    // Scene channels
    map.insert("Background", "scene");
    map.insert("Translucency", "scene");
    map.insert("Translucency0", "scene");
    map.insert("VirtualBeauty", "scene");
    map.insert("ZDepth", "scene");
    
    // Technical channels
    map.insert("RenderStamp", "technical");
    map.insert("RenderStamp0", "technical");
    
    // Light channels
    map.insert("Sky", "light");
    map.insert("Sun", "light");
    map.insert("LightMix", "light");
    
    // Cryptomatte channels
    map.insert("Cryptomatte", "cryptomatte");
    map.insert("Cryptomatte0", "cryptomatte");
    
    map
});

/// String interning cache for group names to avoid repeated allocations
pub static GROUP_NAME_CACHE: Lazy<HashMap<&'static str, String>> = Lazy::new(|| {
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

/// Default configuration for channel grouping
pub fn create_default_config() -> ChannelGroupConfig {
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
        basic_rgb_channels: vec!["R".to_string(), "G".to_string(), "B".to_string(), "A".to_string()],
        group_priority_order: vec!["cryptomatte".to_string(), "light".to_string(), "scene".to_string(), "technical".to_string(), "scene_objects".to_string()],
        fallback_names: FallbackNames {
            basic_rgb: "Basic RGB".to_string(),
            default: "Other".to_string(),
        },
        groups,
    }
}

/// Ultra-fast channel group determination using precomputed lookups + SIMD patterns
pub fn determine_channel_group_ultra_fast(channel_name: &str) -> &'static str {
    // Fast path: Check if it's a basic RGB channel
    if matches!(channel_name, "R" | "G" | "B" | "A") {
        return "base";
    }
    
    // Extract prefix (before first dot)
    let prefix = if let Some(dot_pos) = channel_name.find('.') {
        &channel_name[..dot_pos]
    } else {
        channel_name
    };
    
    // Ultra-fast O(1) lookup for common prefixes
    if let Some(&group_key) = CHANNEL_PREFIX_MAP.get(prefix) {
        return group_key;
    }
    
    // Pattern matching for wildcards using SIMD when possible
    if matches_pattern_simd(prefix, "Light*") {
        return "light";
    }
    
    if matches_pattern_simd(prefix, "ID*") || matches_pattern_simd(prefix, "_*") {
        return "scene_objects";
    }
    
    // Default fallback
    "scene_objects"
}

/// SIMD-accelerated pattern matching
pub fn matches_pattern_simd(text: &str, pattern: &str) -> bool {
    // Fast paths for common patterns
    if pattern == "*" {
        return true;
    }
    
    if pattern.is_empty() {
        return text.is_empty();
    }
    
    // Prefix pattern (e.g., "Light*")
    if let Some(prefix) = pattern.strip_suffix('*') {
        return matches_prefix_simd(text, prefix);
    }
    
    // Suffix pattern (e.g., "*Mix")
    if let Some(suffix) = pattern.strip_prefix('*') {
        return matches_suffix_simd(text, suffix);
    }
    
    // Exact match
    text == pattern
}

#[cfg(target_arch = "x86_64")]
fn matches_prefix_simd(text: &str, prefix: &str) -> bool {
    let text_bytes = text.as_bytes();
    let prefix_bytes = prefix.as_bytes();
    
    if prefix_bytes.len() > text_bytes.len() {
        return false;
    }
    
    // Use SIMD for longer prefixes
    if prefix_bytes.len() >= 16 && is_x86_feature_detected!("sse2") {
        unsafe {
            return matches_prefix_sse2(text_bytes, prefix_bytes);
        }
    }
    
    // Fallback for shorter prefixes or non-SIMD systems
    text_bytes.starts_with(prefix_bytes)
}

#[cfg(target_arch = "x86_64")]
unsafe fn matches_prefix_sse2(text: &[u8], prefix: &[u8]) -> bool {
    let chunks = prefix.len() / 16;
    
    for i in 0..chunks {
        let text_chunk = _mm_loadu_si128(text.as_ptr().add(i * 16) as *const __m128i);
        let prefix_chunk = _mm_loadu_si128(prefix.as_ptr().add(i * 16) as *const __m128i);
        
        let cmp = _mm_cmpeq_epi8(text_chunk, prefix_chunk);
        let mask = _mm_movemask_epi8(cmp);
        
        if mask != 0xFFFF {
            return false;
        }
    }
    
    // Handle remaining bytes
    let remaining = prefix.len() % 16;
    if remaining > 0 {
        let start = chunks * 16;
        return text[start..start + remaining] == prefix[start..];
    }
    
    true
}

#[cfg(not(target_arch = "x86_64"))]
fn matches_prefix_simd(text: &str, prefix: &str) -> bool {
    text.starts_with(prefix)
}

fn matches_suffix_simd(text: &str, suffix: &str) -> bool {
    // For suffix matching, SIMD optimization is less beneficial due to alignment issues
    // Use optimized standard library implementation
    text.ends_with(suffix)
}

/// Fast parallel channel grouping with configuration support
pub fn group_channels_parallel(
    channels: &[crate::io::fast_exr_metadata::ChannelInfo],
    config: Option<&ChannelGroupConfig>
) -> HashMap<String, Vec<String>> {
    let channel_groups: DashMap<String, Vec<String>> = DashMap::new();
    
    // Process channels in parallel without locks
    channels
        .iter()
        .for_each(|channel| {
            let group_name = if let Some(cfg) = config {
                determine_channel_group_with_config(&channel.name, cfg)
            } else {
                determine_channel_group_ultra_fast(&channel.name).to_string()
            };
            
            channel_groups.entry(group_name).or_insert_with(Vec::new).push(channel.name.clone());
        });
    
    // Convert to regular HashMap
    channel_groups.into_iter().collect()
}

/// Channel group determination with configuration support
pub fn determine_channel_group_with_config(channel_name: &str, config: &ChannelGroupConfig) -> String {
    // Check for basic RGB channels first
    if config.basic_rgb_channels.contains(&channel_name.to_string()) {
        for group_def in config.groups.values() {
            if group_def.basic_rgb {
                return GROUP_NAME_CACHE.get("base").cloned()
                    .unwrap_or_else(|| group_def.name.clone());
            }
        }
        return GROUP_NAME_CACHE.get("basic_rgb").cloned()
            .unwrap_or_else(|| config.fallback_names.basic_rgb.clone());
    }
    
    let prefix = if let Some(dot_pos) = channel_name.find('.') {
        &channel_name[..dot_pos]
    } else {
        channel_name
    };
    
    // Check specific groups in priority order
    for group_key in &config.group_priority_order {
        if let Some(group_def) = config.groups.get(group_key) {
            // Check exact prefix matches
            for prefix_str in &group_def.prefixes {
                if prefix == prefix_str {
                    return GROUP_NAME_CACHE.get(group_key.as_str()).cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
            
            // Check pattern matches
            for pattern in &group_def.patterns {
                if matches_pattern_simd(prefix, pattern) {
                    return GROUP_NAME_CACHE.get(group_key.as_str()).cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
        }
    }

    // Check remaining groups not in the priority list to catch misconfigurations
    for (group_key, group_def) in &config.groups {
        if !config.group_priority_order.contains(group_key) {
            // Check exact prefix matches
            for prefix_str in &group_def.prefixes {
                if prefix == prefix_str {
                    return GROUP_NAME_CACHE.get(group_key.as_str()).cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
            
            // Check pattern matches
            for pattern in &group_def.patterns {
                if matches_pattern_simd(prefix, pattern) {
                    return GROUP_NAME_CACHE.get(group_key.as_str()).cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
        }
    }
    
    // Default fallback
    GROUP_NAME_CACHE.get("other").cloned()
        .unwrap_or_else(|| config.fallback_names.default.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_pattern_matching() {
        assert!(matches_pattern_simd("LightMix", "Light*"));
        assert!(matches_pattern_simd("Background", "Back*"));
        assert!(matches_pattern_simd("test", "*"));
        assert!(!matches_pattern_simd("test", "other*"));
    }

    #[test]
    fn test_channel_group_classification() {
        assert_eq!(determine_channel_group_ultra_fast("R"), "base");
        assert_eq!(determine_channel_group_ultra_fast("Beauty.red"), "base");
        assert_eq!(determine_channel_group_ultra_fast("LightMix.blue"), "light");
        assert_eq!(determine_channel_group_ultra_fast("Background.red"), "scene");
        assert_eq!(determine_channel_group_ultra_fast("ID0.red"), "scene_objects");
        assert_eq!(determine_channel_group_ultra_fast("_walls.blue"), "scene_objects");
    }
}
