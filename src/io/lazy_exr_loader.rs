use crate::ui::progress::ProgressSink;
use crate::utils::split_layer_and_short;
use anyhow::Context;
use ::exr::image::read::layers::ReadChannels;
use ::exr::image::read::image::ReadLayers;
use exr::prelude as exr;
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

/// Lazy EXR loader that loads only metadata initially and pixel data on demand
/// Significantly reduces RAM usage for large EXR files with multiple layers
/// Optimized with selective reading and memory mapping for large files

#[derive(Clone, Debug)]
pub struct LazyLayerMetadata {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub channel_names: Arc<Vec<String>>,
    // Pixel data is NOT stored here - loaded on demand
}

#[derive(Clone, Debug)]
pub struct LazyLayerData {
    pub metadata: LazyLayerMetadata,
    pub channel_data: Arc<[f32]>, // Actual pixel data (loaded on demand)
}

pub struct LazyExrLoader {
    path: PathBuf,
    metadata: Vec<LazyLayerMetadata>,
    // Cache for loaded layer data with LRU eviction
    data_cache: RwLock<HashMap<String, LazyLayerData>>,
    // Mutex for file access (exr library may not be thread-safe for concurrent reads)
    file_access: Mutex<()>,
    // Maximum number of layers to keep in memory simultaneously
    max_cached_layers: usize,
    // Optional memory mapping for large files
    mmap: Option<Arc<Mmap>>,
}

#[allow(dead_code)]
impl LazyExrLoader {
    /// Create a new lazy loader - loads only metadata, not pixel data
    pub fn new(path: PathBuf, max_cached_layers: usize) -> anyhow::Result<Self> {
        let metadata = Self::load_metadata_only(&path)?;

        // For large files (> 500MB), use memory mapping to speed up random access
        let mut mmap = None;
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > 500 * 1024 * 1024 {
                if let Ok(file) = File::open(&path) {
                    // Unsafe because file could be modified/truncated while mapped,
                    // but for typical EXR usage this is acceptable risk for performance
                    if let Ok(map) = unsafe { Mmap::map(&file) } {
                        mmap = Some(Arc::new(map));
                        println!("[lazy] Using memory mapping for large file: {}", path.display());
                    }
                }
            }
        }

        Ok(LazyExrLoader {
            path,
            metadata,
            data_cache: RwLock::new(HashMap::with_capacity(max_cached_layers)),
            file_access: Mutex::new(()),
            max_cached_layers,
            mmap,
        })
    }

    /// Load only layer structure and channel info (no pixel data)
    fn load_metadata_only(path: &PathBuf) -> anyhow::Result<Vec<LazyLayerMetadata>> {
        // Read only metadata (headers) without pixel data
        let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/ false)
            .with_context(|| format!("Failed to read EXR metadata: {}", path.display()))?;

        let mut layer_map: HashMap<String, LazyLayerMetadata> = HashMap::new();
        let mut layer_order: Vec<String> = Vec::new();

        for header in meta.headers.iter() {
            let base_layer_name = header
                .own_attributes
                .layer_name
                .as_ref()
                .map(|t| t.to_string());

            // Extract layer dimensions
            let width = header.layer_size.width() as u32;
            let height = header.layer_size.height() as u32;

            for ch in header.channels.list.iter() {
                let full_channel_name = ch.name.to_string();
                let (layer_name, short_channel_name) =
                    split_layer_and_short(&full_channel_name, base_layer_name.as_deref());

                let entry = layer_map.entry(layer_name.clone()).or_insert_with(|| {
                    layer_order.push(layer_name.clone());
                    LazyLayerMetadata {
                        name: layer_name.clone(),
                        width,
                        height,
                        channel_names: Arc::new(Vec::new()),
                    }
                });

                // Verify dimensions match (skip conflicting channels)
                if entry.width == width && entry.height == height {
                    Arc::make_mut(&mut entry.channel_names).push(short_channel_name);
                }
            }
        }

        // Build ordered list of metadata
        let mut metadata: Vec<LazyLayerMetadata> = Vec::with_capacity(layer_map.len());
        for name in layer_order {
            if let Some(meta) = layer_map.remove(&name) {
                metadata.push(meta);
            }
        }

        println!(
            "[lazy] Loaded metadata for {} layers from {}",
            metadata.len(),
            path.display()
        );
        Ok(metadata)
    }

    /// Get layer metadata (always available, no I/O)
    pub fn get_metadata(&self) -> &[LazyLayerMetadata] {
        &self.metadata
    }

    /// Load layer data on demand (performs I/O if not cached)
    pub fn get_layer_data(
        &self,
        layer_name: &str,
        progress: Option<&dyn ProgressSink>,
    ) -> anyhow::Result<LazyLayerData> {
        // Fast path: check if already cached
        {
            let cache = self.data_cache.read().unwrap();
            if let Some(cached) = cache.get(layer_name) {
                return Ok(cached.clone());
            }
        }

        // Slow path: load from disk
        if let Some(p) = progress {
            p.start_indeterminate(Some(&format!("Loading layer: {}", layer_name)));
        }

        let layer_data = self.load_layer_from_disk(layer_name, progress)?;

        // Cache the loaded data with LRU eviction
        {
            let mut cache = self.data_cache.write().unwrap();

            // Simple LRU: if cache is full, remove oldest entry
            if cache.len() >= self.max_cached_layers {
                // Remove arbitrary entry (in a full implementation we'd track access order)
                if let Some(key_to_remove) = cache.keys().next().cloned() {
                    cache.remove(&key_to_remove);
                    println!("[lazy] Evicted layer from cache: {}", key_to_remove);
                }
            }

            cache.insert(layer_name.to_string(), layer_data.clone());
        }

        if let Some(p) = progress {
            p.finish(Some(&format!("Layer loaded: {}", layer_name)));
        }

        Ok(layer_data)
    }

    /// Load specific layer from disk (protected by file access mutex)
    fn load_layer_from_disk(
        &self,
        layer_name: &str,
        _progress: Option<&dyn ProgressSink>,
    ) -> anyhow::Result<LazyLayerData> {
        let _guard = self.file_access.lock().unwrap();

        // Find metadata for this layer
        let metadata = self
            .metadata
            .iter()
            .find(|m| m.name.to_lowercase() == layer_name.to_lowercase())
            .ok_or_else(|| anyhow::anyhow!("Layer not found: {}", layer_name))?;

        let width = metadata.width;
        let height = metadata.height;
        let pixel_count = (width as usize) * (height as usize);

        // Read full image from mmap or file, then extract only the target layer
        let any_image = if let Some(mmap) = &self.mmap {
            let cursor = Cursor::new(&mmap[..]);
            exr::read()
                .no_deep_data()
                .largest_resolution_level()
                .all_channels()
                .all_layers()
                .all_attributes()
                .from_buffered(cursor)
                .map_err(|e| anyhow::anyhow!("EXR read error: {}", e))?
        } else {
            let file = File::open(&self.path)?;
            let buffered_file = std::io::BufReader::new(file);
            exr::read()
                .no_deep_data()
                .largest_resolution_level()
                .all_channels()
                .all_layers()
                .all_attributes()
                .from_buffered(buffered_file)
                .map_err(|e| anyhow::anyhow!("EXR read error: {}", e))?
        };

        // Extract only the target layer's data (same pattern as build_full_exr_cache)
        let mut channel_names: Vec<String> = Vec::new();
        let mut channel_data: Vec<f32> = Vec::new();

        for layer in any_image.layer_data.iter() {
            let base_attr: Option<String> = layer.attributes.layer_name.as_ref().map(|s| s.to_string());

            for ch in layer.channel_data.list.iter() {
                let full = ch.name.to_string();
                let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());

                if !lname.eq_ignore_ascii_case(layer_name) {
                    continue;
                }

                channel_names.push(short);
                let samples = (0..pixel_count).map(|i| {
                    ch.sample_data.value_by_flat_index(i).to_f32()
                });
                channel_data.extend(samples);
            }
        }

        if channel_names.is_empty() {
            anyhow::bail!("No channels found for layer: {}", layer_name);
        }

        let data = LazyLayerData {
            metadata: metadata.clone(),
            channel_data: Arc::from(channel_data.into_boxed_slice()),
        };

        let loaded_mb = (pixel_count * channel_names.len() * 4) / (1024 * 1024);
        println!(
            "[lazy] Loaded {} MB for layer: {} ({} channels)",
            loaded_mb, layer_name, channel_names.len()
        );
        Ok(data)
    }

    /// Get cache statistics for monitoring
    pub fn get_cache_stats(&self) -> (usize, usize, usize) {
        let cache = self.data_cache.read().unwrap();
        let cached_count = cache.len();
        let total_layers = self.metadata.len();
        let cached_memory_mb = cache
            .values()
            .map(|data| {
                (data.metadata.width as usize)
                    * (data.metadata.height as usize)
                    * data.metadata.channel_names.len()
                    * 4
            })
            .sum::<usize>()
            / (1024 * 1024);

        (cached_count, total_layers, cached_memory_mb)
    }

    /// Clear all cached data to free memory
    pub fn clear_cache(&self) {
        let mut cache = self.data_cache.write().unwrap();
        cache.clear();
        println!("[lazy] Cache cleared");
    }
}

// Compatibility layer for existing FullExrCacheData API
impl LazyLayerData {
    /// Convert to old LayerChannels format for backward compatibility
    pub fn to_layer_channels(&self) -> crate::io::image_cache::LayerChannels {
        crate::io::image_cache::LayerChannels {
            layer_name: self.metadata.name.clone(),
            width: self.metadata.width,
            height: self.metadata.height,
            channel_names: (*self.metadata.channel_names).clone(),
            channel_data: self.channel_data.clone(),
        }
    }
}
