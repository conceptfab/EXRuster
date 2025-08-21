
/// Common trait for all layer descriptor types
/// Provides a unified interface for accessing layer metadata
pub trait LayerDescriptor {
    /// Get the layer name
    fn name(&self) -> &str;
    
    /// Get the list of channel names
    fn channel_names(&self) -> Vec<String>;
    
    /// Get layer dimensions as (width, height) tuple if available
    #[allow(dead_code)]
    fn dimensions(&self) -> Option<(u32, u32)>;
    
    /// Check if layer has specified channel
    #[allow(dead_code)]
    fn has_channel(&self, channel_name: &str) -> bool {
        self.channel_names().iter().any(|c| c == channel_name)
    }
}

/// Channel information structure
#[derive(Clone, Debug)]
pub struct ChannelInfo {
    pub name: String,           // short name (after last dot)
}

impl ChannelInfo {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

/// Unified layer information structure
/// Combines all fields from LayerInfo, LayerMetadata, and LazyLayerMetadata
#[derive(Clone, Debug)]
pub struct UnifiedLayerInfo {
    pub name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub channels: Vec<ChannelInfo>,
    pub channel_names: Vec<String>,  // For compatibility with LazyLayerMetadata
    pub attributes: Vec<(String, String)>,
}

impl UnifiedLayerInfo {
    /// Create from basic layer info (for UI usage)
    pub fn from_basic(name: String, channels: Vec<ChannelInfo>) -> Self {
        let channel_names = channels.iter().map(|c| c.name.clone()).collect();
        Self {
            name,
            width: None,
            height: None,
            channels,
            channel_names,
            attributes: Vec::new(),
        }
    }
    
    /// Create from metadata (for EXR metadata usage)
    pub fn from_metadata(name: String, width: u32, height: u32, attributes: Vec<(String, String)>) -> Self {
        Self {
            name,
            width: Some(width),
            height: Some(height),
            channels: Vec::new(),
            channel_names: Vec::new(),
            attributes,
        }
    }
    
    /// Create from lazy metadata (for lazy loading usage)
    pub fn from_lazy(name: String, width: u32, height: u32, channel_names: Vec<String>) -> Self {
        let channels = channel_names.iter().map(|name| ChannelInfo::new(name.clone())).collect();
        Self {
            name,
            width: Some(width),
            height: Some(height),
            channels,
            channel_names: channel_names.clone(),
            attributes: Vec::new(),
        }
    }
}

impl LayerDescriptor for UnifiedLayerInfo {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn channel_names(&self) -> Vec<String> {
        if !self.channel_names.is_empty() {
            self.channel_names.clone()
        } else {
            self.channels.iter().map(|c| c.name.clone()).collect()
        }
    }
    
    fn dimensions(&self) -> Option<(u32, u32)> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        }
    }
}

/// Conversion traits for backwards compatibility

/// Convert from original LayerInfo (image_cache.rs)
impl From<crate::io::image_cache::LayerInfo> for UnifiedLayerInfo {
    fn from(layer_info: crate::io::image_cache::LayerInfo) -> Self {
        Self::from_basic(layer_info.name, layer_info.channels.into_iter().map(|c| ChannelInfo::new(c.name)).collect())
    }
}

/// Convert to original LayerInfo for backwards compatibility
impl From<UnifiedLayerInfo> for crate::io::image_cache::LayerInfo {
    fn from(unified: UnifiedLayerInfo) -> Self {
        Self {
            name: unified.name,
            channels: unified.channels.into_iter().map(|c| crate::io::image_cache::ChannelInfo { name: c.name }).collect(),
        }
    }
}

/// Convert from LayerMetadata (exr_metadata.rs)
impl From<crate::io::exr_metadata::LayerMetadata> for UnifiedLayerInfo {
    fn from(metadata: crate::io::exr_metadata::LayerMetadata) -> Self {
        Self::from_metadata(metadata.name, metadata.width, metadata.height, metadata.attributes)
    }
}

/// Convert to LayerMetadata for backwards compatibility  
impl From<UnifiedLayerInfo> for crate::io::exr_metadata::LayerMetadata {
    fn from(unified: UnifiedLayerInfo) -> Self {
        Self {
            name: unified.name,
            width: unified.width.unwrap_or(0),
            height: unified.height.unwrap_or(0),
            attributes: unified.attributes,
        }
    }
}

/// Convert from LazyLayerMetadata (lazy_exr_loader.rs)
impl From<crate::io::lazy_exr_loader::LazyLayerMetadata> for UnifiedLayerInfo {
    fn from(lazy: crate::io::lazy_exr_loader::LazyLayerMetadata) -> Self {
        Self::from_lazy(lazy.name, lazy.width, lazy.height, lazy.channel_names)
    }
}

/// Convert to LazyLayerMetadata for backwards compatibility
impl From<UnifiedLayerInfo> for crate::io::lazy_exr_loader::LazyLayerMetadata {
    fn from(unified: UnifiedLayerInfo) -> Self {
        Self {
            name: unified.name,
            width: unified.width.unwrap_or(0),
            height: unified.height.unwrap_or(0),
            channel_names: unified.channel_names,
        }
    }
}


/// Utility functions for layer operations
pub mod utils {
    use super::*;
    
    /// Find the best layer for display from a collection
    pub fn find_best_layer<T: LayerDescriptor>(layers: &[T]) -> Option<&T> {
        // Priority order: RGBA > RGB > first available
        layers.iter()
            .find(|layer| {
                let channels = layer.channel_names();
                channels.contains(&"R".to_string()) && 
                channels.contains(&"G".to_string()) && 
                channels.contains(&"B".to_string()) && 
                channels.contains(&"A".to_string())
            })
            .or_else(|| {
                layers.iter().find(|layer| {
                    let channels = layer.channel_names();
                    channels.contains(&"R".to_string()) && 
                    channels.contains(&"G".to_string()) && 
                    channels.contains(&"B".to_string())
                })
            })
            .or_else(|| layers.first())
    }
    
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_unified_layer_info_creation() {
        let layer = UnifiedLayerInfo::from_lazy("test_layer".to_string(), 1920, 1080, 
            vec!["R".to_string(), "G".to_string(), "B".to_string()]);
            
        assert_eq!(layer.name(), "test_layer");
        assert_eq!(layer.dimensions(), Some((1920, 1080)));
        assert_eq!(layer.channel_names(), vec!["R", "G", "B"]);
        assert!(layer.has_channel("R"));
        assert!(!layer.has_channel("A"));
    }
    
    #[test]
    fn test_find_best_layer() {
        let layers = vec![
            UnifiedLayerInfo::from_lazy("mono".to_string(), 100, 100, vec!["Y".to_string()]),
            UnifiedLayerInfo::from_lazy("rgb".to_string(), 100, 100, vec!["R".to_string(), "G".to_string(), "B".to_string()]),
            UnifiedLayerInfo::from_lazy("rgba".to_string(), 100, 100, vec!["R".to_string(), "G".to_string(), "B".to_string(), "A".to_string()]),
        ];
        
        let best = utils::find_best_layer(&layers);
        assert_eq!(best.unwrap().name(), "rgba");
    }
}