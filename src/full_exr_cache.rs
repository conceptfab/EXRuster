use std::path::PathBuf;

use anyhow::Context;
use exr::prelude as exr;

use crate::progress::ProgressSink;
use crate::utils::split_layer_and_short;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct FullLayerData {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub channel_names: Vec<String>,
    pub channel_data: Arc<[f32]>,
}

pub struct FullExrCacheData {
    pub path: PathBuf,
    pub metadata: exr::meta::MetaData,
}

impl FullExrCacheData {
    pub fn new(path: PathBuf, progress: Option<&dyn ProgressSink>) -> anyhow::Result<Self> {
        if let Some(p) = progress { p.set(0.18, Some("Reading EXR metadata...")); }
        let metadata = exr::meta::MetaData::read_from_file(&path, /*pedantic=*/false)
            .with_context(|| format!("Błąd wczytania EXR (nagłówki): {}", path.display()))?;
        if let Some(p) = progress { p.set(0.24, Some("EXR metadata loaded")); }
        Ok(FullExrCacheData { path, metadata })
    }

    pub fn load_layer_data(&self, layer_name: &str, progress: Option<&dyn ProgressSink>) -> anyhow::Result<FullLayerData> {
        if let Some(p) = progress { p.start_indeterminate(Some(&format!("Loading layer '{}'...", layer_name))); }

        let any_image = exr::read_all_flat_layers_from_file(&self.path)
            .with_context(|| format!("Błąd wczytania EXR: {}", self.path.display()))?;

        let wanted_lower = layer_name.to_lowercase();

        for layer in any_image.layer_data.iter() {
            let width = layer.size.width() as u32;
            let height = layer.size.height() as u32;
            let pixel_count = (width as usize) * (height as usize);

            let base_attr: Option<String> = layer
                .attributes
                .layer_name
                .as_ref()
                .map(|s| s.to_string());
            let current_layer_name = base_attr.clone().unwrap_or_else(|| "".to_string());

            let lname_lower = current_layer_name.to_lowercase();
            let matches = if wanted_lower.is_empty() && lname_lower.is_empty() {
                true
            } else if wanted_lower.is_empty() || lname_lower.is_empty() {
                false
            } else {
                lname_lower == wanted_lower || lname_lower.contains(&wanted_lower) || wanted_lower.contains(&lname_lower)
            };

            if matches {
                if let Some(p) = progress { p.set(0.35, Some("Copying channel data...")); }
                let num_channels = layer.channel_data.list.len();
                let mut channel_names: Vec<String> = Vec::with_capacity(num_channels);
                let mut channel_data_vec: Vec<f32> = Vec::with_capacity(pixel_count * num_channels);

                for (idx, ch) in layer.channel_data.list.iter().enumerate() {
                    let full = ch.name.to_string();
                    let (_lname, short) = split_layer_and_short(&full, base_attr.as_deref());
                    channel_names.push(short);

                    for i in 0..pixel_count {
                        channel_data_vec.push(layer.channel_data.list[idx].sample_data.value_by_flat_index(i).to_f32());
                    }
                }
                if let Some(p) = progress { p.finish(Some("Layer data loaded")); }
                return Ok(FullLayerData {
                    name: current_layer_name,
                    width,
                    height,
                    channel_names,
                    channel_data: Arc::from(channel_data_vec.into_boxed_slice()),
                });
            }
        }

        if let Some(p) = progress { p.reset(); }
        anyhow::bail!(format!("Nie znaleziono warstwy '{}' w pliku {}", layer_name, self.path.display()))
    }
}