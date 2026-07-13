use std::path::PathBuf;

use anyhow::Context;
use exr::prelude as exr;

use crate::ui::progress::ProgressSink;
use crate::utils::split_layer_and_short;

#[derive(Clone, Debug)]
pub struct FullLayer {
    pub name: String,
    pub width: u32,
    pub height: u32,
    // Lista krótkich nazw kanałów w stabilnej kolejności (zgodnie z kolejnością w pliku)
    pub channel_names: Vec<String>,
    // Dane pikseli w układzie planarnym: [ch0(0..N), ch1(0..N), ...]
    pub channel_data: std::sync::Arc<[f32]>,
}

#[derive(Clone, Debug)]
pub struct FullExrCacheData {
    pub layers: Vec<FullLayer>,
}

impl FullExrCacheData {
    /// Build layers info from cached data (avoids re-reading from disk)
    pub fn to_layers_info(&self) -> Vec<crate::io::image_cache::LayerInfo> {
        self.layers
            .iter()
            .map(|fl| crate::io::image_cache::LayerInfo {
                name: fl.name.clone(),
                channels: fl
                    .channel_names
                    .iter()
                    .map(|c| crate::io::fast_exr_metadata::ChannelInfo::new(c.clone()))
                    .collect(),
            })
            .collect()
    }
}

/// Bulk-convert one channel's samples: a single enum dispatch per channel,
/// instead of one per sample via `value_by_flat_index`. For a large EXR that is
/// the difference between billions of dispatches and a handful.
pub(crate) fn flat_samples_to_f32(samples: &exr::FlatSamples) -> Vec<f32> {
    match samples {
        exr::FlatSamples::F16(v) => v.iter().map(|x| x.to_f32()).collect(),
        exr::FlatSamples::F32(v) => v.clone(),
        exr::FlatSamples::U32(v) => v.iter().map(|&x| x as f32).collect(),
    }
}

/// Buduje pełny cache z pliku EXR: wszystkie warstwy i kanały w pamięci (float32)
pub fn build_full_exr_cache(
    path: &PathBuf,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<FullExrCacheData> {
    if let Some(p) = progress {
        p.set(0.18, Some("Reading EXR (pixels)..."));
    }
    let any_image = exr::read_all_flat_layers_from_file(path)
        .with_context(|| format!("Błąd wczytania EXR: {}", path.display()))?;

    use rayon::prelude::*;
    use std::collections::HashMap;

    struct ConvertedChannel {
        layer_name: String,
        short: String,
        width: u32,
        height: u32,
        data: Vec<f32>,
    }

    // Flatten to (layer, channel) pairs, then convert every channel in parallel.
    // The old loop was single-threaded despite running under rayon::spawn.
    let channel_refs: Vec<_> = any_image
        .layer_data
        .iter()
        .flat_map(|layer| {
            let width = layer.size.width() as u32;
            let height = layer.size.height() as u32;
            let base_attr: Option<String> =
                layer.attributes.layer_name.as_ref().map(|s| s.to_string());
            layer
                .channel_data
                .list
                .iter()
                .map(move |ch| (width, height, base_attr.clone(), ch))
        })
        .collect();

    let converted: Vec<ConvertedChannel> = channel_refs
        .into_par_iter()
        .map(|(width, height, base_attr, ch)| {
            let full = ch.name.to_string();
            let (layer_name, short) = split_layer_and_short(&full, base_attr.as_deref());
            ConvertedChannel {
                layer_name,
                short,
                width,
                height,
                data: flat_samples_to_f32(&ch.sample_data),
            }
        })
        .collect();

    // Aggregate sequentially, preserving first-occurrence layer order.
    let mut layer_map: HashMap<String, (u32, u32, Vec<String>, Vec<f32>)> = HashMap::new();
    let mut layer_order: Vec<String> = Vec::with_capacity(any_image.layer_data.len());

    for c in converted {
        let entry = layer_map.entry(c.layer_name.clone()).or_insert_with(|| {
            layer_order.push(c.layer_name.clone());
            (c.width, c.height, Vec::new(), Vec::new())
        });
        // Jeśli rozmiary różnią się (rzadkie), preferuj pierwszy i pomiń konfliktujące kanały
        if entry.0 != c.width || entry.1 != c.height {
            continue;
        }
        entry.2.push(c.short);
        entry.3.extend_from_slice(&c.data);
    }

    let mut out_layers: Vec<FullLayer> = Vec::with_capacity(layer_map.len());
    for name in layer_order {
        if let Some((w, h, channel_names, channel_data)) = layer_map.remove(&name) {
            out_layers.push(FullLayer {
                name,
                width: w,
                height: h,
                channel_names,
                channel_data: std::sync::Arc::from(channel_data.into_boxed_slice()),
            });
        }
    }

    if let Some(p) = progress {
        p.set(0.24, Some("EXR in RAM"));
    }
    Ok(FullExrCacheData { layers: out_layers })
}

// Plik został oczyszczony z nieużywanego kodu zgodnie z analizą optymalizacji
