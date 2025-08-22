// Level 3 Hardcore Optimization: SIMD-accelerated string pattern matching
// Uses vectorized operations for ultra-fast channel name classification

// Level 3: Ultra-fast pattern matching using SIMD when available
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

// Level 3: Vectorized string comparison for pattern matching
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

// Level 3: Precomputed hash-based pattern matching for ultra-fast channel classification
use std::collections::HashMap;
use once_cell::sync::Lazy;

// Pre-compute common channel prefixes with their group assignments for O(1) lookup
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

// Level 3: Ultra-fast channel group determination using precomputed lookups + SIMD patterns
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