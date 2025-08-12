use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use exr::prelude as exr;

use crate::progress::ProgressSink;
use crate::utils::split_layer_and_short;

#[derive(Clone, Debug)]
pub struct FullLayer {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub channels: HashMap<String, Vec<f32>>, // short_name -> values
}

#[derive(Clone, Debug)]
pub struct FullExrCacheData {
    pub layers: Vec<FullLayer>,
}

/// Buduje pełny cache z pliku EXR: wszystkie warstwy i kanały w pamięci (float32)
pub fn build_full_exr_cache(
    path: &PathBuf,
    progress: Option<&dyn ProgressSink>,
) -> anyhow::Result<FullExrCacheData> {
    if let Some(p) = progress { p.set(0.18, Some("Reading EXR (pixels)...")); }
    let any_image = exr::read_all_flat_layers_from_file(path)
        .with_context(|| format!("Błąd wczytania EXR: {}", path.display()))?;

    let mut out_layers: Vec<FullLayer> = Vec::with_capacity(any_image.layer_data.len());

    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);

        let base_attr: Option<String> = layer
            .attributes
            .layer_name
            .as_ref()
            .map(|s| s.to_string());
        // Nazwa warstwy ("" dla głównej)
        let layer_name = base_attr.clone().unwrap_or_else(|| "".to_string());

        let mut channels: HashMap<String, Vec<f32>> = HashMap::new();

        for (idx, ch) in layer.channel_data.list.iter().enumerate() {
            let full = ch.name.to_string();
            let (_lname, short) = split_layer_and_short(&full, base_attr.as_deref());

            let mut v: Vec<f32> = Vec::with_capacity(pixel_count);
            for i in 0..pixel_count {
                v.push(layer.channel_data.list[idx].sample_data.value_by_flat_index(i).to_f32());
            }
            channels.insert(short, v);
        }

        out_layers.push(FullLayer { name: layer_name, width, height, channels });
    }

    if let Some(p) = progress { p.set(0.24, Some("EXR in RAM")); }
    Ok(FullExrCacheData { layers: out_layers })
}

pub fn find_layer_by_name<'a>(cache: &'a FullExrCacheData, wanted: &str) -> Option<&'a FullLayer> {
    let wanted_lower = wanted.to_lowercase();
    cache.layers.iter().find(|l| {
        let lname = l.name.to_lowercase();
        if wanted_lower.is_empty() && lname.is_empty() { return true; }
        if wanted_lower.is_empty() || lname.is_empty() { return false; }
        lname == wanted_lower || lname.contains(&wanted_lower) || wanted_lower.contains(&lname)
    })
}


