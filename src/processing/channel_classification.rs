use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
/// Channel classification using dictionary lookups and simple prefix/suffix patterns.
use std::collections::HashMap;

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
static CHANNEL_PREFIX_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut map = HashMap::new();

    // Base channels
    map.insert("Beauty", "base");
    map.insert("R", "base");
    map.insert("G", "base");
    map.insert("B", "base");
    map.insert("A", "base");

    // Scene channels
    map.insert("Background", "scene");
    map.insert("VirtualBeauty", "scene");
    map.insert("ZDepth", "scene");

    // Technical channels (note: now uses patterns, but keeping for ultra-fast fallback)

    // Light channels
    map.insert("Sky", "light");
    map.insert("Sun", "light");
    map.insert("LightMix", "light");

    // Cryptomatte channels (note: now uses patterns, but keeping for ultra-fast fallback)

    map
});

/// String interning cache for group names to avoid repeated allocations
pub static GROUP_NAME_CACHE: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
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

    groups.insert(
        "base".to_string(),
        GroupDefinition {
            name: "Base".to_string(),
            prefixes: vec!["Beauty".to_string()],
            patterns: vec![],
            basic_rgb: true,
        },
    );

    groups.insert(
        "scene".to_string(),
        GroupDefinition {
            name: "Scene".to_string(),
            prefixes: vec![
                "Background".to_string(),
                "VirtualBeauty".to_string(),
                "ZDepth".to_string(),
            ],
            patterns: vec!["Translucency*".to_string(), "translucency*".to_string()],
            basic_rgb: false,
        },
    );

    groups.insert(
        "technical".to_string(),
        GroupDefinition {
            name: "Technical".to_string(),
            prefixes: vec![],
            patterns: vec!["RenderStamp*".to_string(), "renderstamp*".to_string()],
            basic_rgb: false,
        },
    );

    groups.insert(
        "light".to_string(),
        GroupDefinition {
            name: "Light".to_string(),
            prefixes: vec!["Sky".to_string(), "Sun".to_string(), "LightMix".to_string()],
            patterns: vec!["Light*".to_string(), "light*".to_string()],
            basic_rgb: false,
        },
    );

    groups.insert(
        "cryptomatte".to_string(),
        GroupDefinition {
            name: "Cryptomatte".to_string(),
            prefixes: vec![],
            patterns: vec!["Cryptomatte*".to_string(), "cryptomatte*".to_string()],
            basic_rgb: false,
        },
    );

    groups.insert(
        "scene_objects".to_string(),
        GroupDefinition {
            name: "Scene Objects".to_string(),
            prefixes: vec![],
            patterns: vec!["ID*".to_string(), "Object*".to_string(), "_*".to_string()],
            basic_rgb: false,
        },
    );

    ChannelGroupConfig {
        basic_rgb_channels: vec![
            "R".to_string(),
            "G".to_string(),
            "B".to_string(),
            "A".to_string(),
        ],
        group_priority_order: vec![
            "cryptomatte".to_string(),
            "light".to_string(),
            "scene".to_string(),
            "technical".to_string(),
            "scene_objects".to_string(),
        ],
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

    // Pattern matching will be handled by the configuration-based function
    // This ultra-fast path only handles exact prefix matches

    // Default fallback
    "scene_objects"
}

/// Match a channel-classification glob pattern against `text`.
///
/// Supported patterns:
///   "*"         — matches anything
///   ""          — matches only the empty string
///   "prefix*"   — matches strings starting with "prefix"
///   "*suffix"   — matches strings ending with "suffix"
///   "exact"     — matches only "exact"
///
/// Channel group names are short (well under 32 bytes), so `str::starts_with`
/// and `str::ends_with` outperform the hand-rolled SSE2 matcher they replaced:
/// the standard library already uses SIMD-accelerated memcmp internally, and
/// the previous implementation only kicked in for prefixes ≥16 bytes anyway.
pub fn matches_pattern_simd(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.is_empty() {
        return text.is_empty();
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return text.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return text.ends_with(suffix);
    }
    text == pattern
}

/// Fast parallel channel grouping with configuration support
pub fn group_channels_parallel(
    channels: &[crate::io::fast_exr_metadata::ChannelInfo],
    config: Option<&ChannelGroupConfig>,
) -> HashMap<String, Vec<String>> {
    let channel_groups: DashMap<String, Vec<String>> = DashMap::new();

    // Process channels in parallel without locks
    channels.iter().for_each(|channel| {
        let group_name = if let Some(cfg) = config {
            determine_channel_group_with_config(&channel.name, cfg)
        } else {
            determine_channel_group_ultra_fast(&channel.name).to_string()
        };

        channel_groups
            .entry(group_name)
            .or_default()
            .push(channel.name.clone());
    });

    // Convert to regular HashMap
    channel_groups.into_iter().collect()
}

/// Channel group determination with configuration support
pub fn determine_channel_group_with_config(
    channel_name: &str,
    config: &ChannelGroupConfig,
) -> String {
    // Check for basic RGB channels first (eq_ignore_ascii_case avoids String allocation)
    if config
        .basic_rgb_channels
        .iter()
        .any(|s| s.eq_ignore_ascii_case(channel_name))
    {
        for group_def in config.groups.values() {
            if group_def.basic_rgb {
                return GROUP_NAME_CACHE
                    .get("base")
                    .cloned()
                    .unwrap_or_else(|| group_def.name.clone());
            }
        }
        return GROUP_NAME_CACHE
            .get("basic_rgb")
            .cloned()
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
                    return GROUP_NAME_CACHE
                        .get(group_key.as_str())
                        .cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }

            // Check pattern matches
            for pattern in &group_def.patterns {
                if matches_pattern_simd(prefix, pattern) {
                    return GROUP_NAME_CACHE
                        .get(group_key.as_str())
                        .cloned()
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
                    return GROUP_NAME_CACHE
                        .get(group_key.as_str())
                        .cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }

            // Check pattern matches
            for pattern in &group_def.patterns {
                if matches_pattern_simd(prefix, pattern) {
                    return GROUP_NAME_CACHE
                        .get(group_key.as_str())
                        .cloned()
                        .unwrap_or_else(|| group_def.name.clone());
                }
            }
        }
    }

    // Default fallback
    GROUP_NAME_CACHE
        .get("other")
        .cloned()
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
        
        // Test generic wildcard patterns with any text
        assert!(matches_pattern_simd("anyPrefix", "any*"));
        assert!(matches_pattern_simd("testValue", "test*"));
        assert!(matches_pattern_simd("somethingElse", "something*"));
        assert!(matches_pattern_simd("IDObject01", "ID*"));
        assert!(matches_pattern_simd("_testLayer", "_*"));
        assert!(matches_pattern_simd("_anything_here", "_*"));
        
        // Negative tests
        assert!(!matches_pattern_simd("notAny", "different*"));
        assert!(!matches_pattern_simd("wrongPrefix", "correct*"));
    }

    #[test]
    fn test_channel_group_classification() {
        assert_eq!(determine_channel_group_ultra_fast("R"), "base");
        assert_eq!(determine_channel_group_ultra_fast("Beauty.red"), "base");
        assert_eq!(determine_channel_group_ultra_fast("LightMix.blue"), "light");
        assert_eq!(
            determine_channel_group_ultra_fast("Background.red"),
            "scene"
        );
        assert_eq!(
            determine_channel_group_ultra_fast("ID0.red"),
            "scene_objects"
        );
        assert_eq!(
            determine_channel_group_ultra_fast("_testLayer.blue"),
            "scene_objects"
        );
    }

    #[test]
    fn test_wildcard_patterns_with_config() {
        let config = create_default_config();
        
        // Test ID* pattern
        let id001_result = determine_channel_group_with_config("ID001", &config);
        println!("ID001 result: {}", id001_result);
        assert_eq!(id001_result, "Scene Objects");
        assert_eq!(determine_channel_group_with_config("IDWalls", &config), "Scene Objects");
        
        // Test _* pattern
        assert_eq!(determine_channel_group_with_config("_testLayer", &config), "Scene Objects");
        assert_eq!(determine_channel_group_with_config("_anything", &config), "Scene Objects");
        
        // Test Light* pattern (case sensitive)
        assert_eq!(determine_channel_group_with_config("LightMix", &config), "Light");
        assert_eq!(determine_channel_group_with_config("LightAny", &config), "Light");
        assert_eq!(determine_channel_group_with_config("lightMix", &config), "Light");
        assert_eq!(determine_channel_group_with_config("lightAny", &config), "Light");
        
        // Test RenderStamp* pattern (case sensitive)
        assert_eq!(determine_channel_group_with_config("RenderStamp1", &config), "Technical");
        assert_eq!(determine_channel_group_with_config("renderstamp2", &config), "Technical");
        
        // Test Cryptomatte* pattern (case sensitive)
        assert_eq!(determine_channel_group_with_config("Cryptomatte3", &config), "Cryptomatte");
        assert_eq!(determine_channel_group_with_config("cryptomatte4", &config), "Cryptomatte");
        
        // Test Translucency* pattern (case sensitive)
        assert_eq!(determine_channel_group_with_config("Translucency1", &config), "Scene");
        assert_eq!(determine_channel_group_with_config("translucency2", &config), "Scene");
        
        // Negative tests - should not match
        assert_ne!(determine_channel_group_with_config("notID", &config), "Scene Objects");
        assert_ne!(determine_channel_group_with_config("noLight", &config), "Light");
    }
}
