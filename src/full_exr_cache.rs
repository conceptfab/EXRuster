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
    // Lista krótkich nazw kanałów w stabilnej kolejności (zgodnie z kolejnością w pliku)
    pub channel_names: Vec<String>,
    // Dane pikseli w układzie planarnym: [ch0(0..N), ch1(0..N), ...]
    pub channel_data: Vec<f32>,
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

    use std::collections::HashMap;
    // Agreguj kanały według efektywnej nazwy warstwy tak samo jak UI (extract_layers_info)
    // Mapowanie: nazwa_warstwy -> (width, height, channel_names, channel_data)
    let mut layer_map: HashMap<String, (u32, u32, Vec<String>, Vec<f32>)> = HashMap::new();
    let mut layer_order: Vec<String> = Vec::new();

    for layer in any_image.layer_data.iter() {
        let width = layer.size.width() as u32;
        let height = layer.size.height() as u32;
        let pixel_count = (width as usize) * (height as usize);

        let base_attr: Option<String> = layer
            .attributes
            .layer_name
            .as_ref()
            .map(|s| s.to_string());

        for (idx, ch) in layer.channel_data.list.iter().enumerate() {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_attr.as_deref());
            let entry = layer_map.entry(lname.clone()).or_insert_with(|| {
                layer_order.push(lname.clone());
                (width, height, Vec::new(), Vec::new())
            });
            // Jeśli rozmiary różnią się (rzadkie), preferuj pierwszy i pomiń konfliktujące kanały
            if entry.0 != width || entry.1 != height { continue; }
            entry.2.push(short);
            let samples = (0..pixel_count)
                .map(|i| layer.channel_data.list[idx].sample_data.value_by_flat_index(i).to_f32());
            entry.3.extend(samples);
        }
    }

    let mut out_layers: Vec<FullLayer> = Vec::with_capacity(layer_map.len());
    for name in layer_order {
        if let Some((w, h, channel_names, channel_data)) = layer_map.remove(&name) {
            out_layers.push(FullLayer { name, width: w, height: h, channel_names, channel_data });
        }
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

#[allow(dead_code)]
impl FullLayer {
    #[inline]
    pub fn pixel_count(&self) -> usize { (self.width as usize) * (self.height as usize) }

    #[inline]
    pub fn num_channels(&self) -> usize { self.channel_names.len() }

    #[inline]
    pub fn channel_slice(&self, channel_index: usize) -> &[f32] {
        let n = self.pixel_count();
        let base = channel_index * n;
        &self.channel_data[base..base + n]
    }
}


