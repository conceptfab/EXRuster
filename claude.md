Schemat algorytmu dla editora EXR
1. Wybór bibliotek

exr (najnowsza: ~1.7) - główna biblioteka do EXR
image (~0.24) - obsługa formatów obrazów
rayon - przetwarzanie równoległe
tokio - asynchroniczne I/O

2. Struktura główna
rustpub struct EXREditor {
    file_path: PathBuf,
    metadata: EXRMetadata,
    layers: Vec<Layer>,
    thumbnail_cache: HashMap<String, Thumbnail>,
}

pub struct EXRMetadata {
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f32,
    pub compression: CompressionType,
    pub channels: Vec<ChannelInfo>,
    pub layers: Vec<LayerInfo>,
}
3. Algorytm wczytywania (etapowy)
Etap 1: Szybka analiza struktury
rustimpl EXREditor {
    pub fn quick_analyze(path: &Path) -> Result<EXRMetadata, Error> {
        // 1. Otwórz plik tylko do odczytu nagłówka
        let file = std::fs::File::open(path)?;
        let reader = exr::prelude::ReadFirstImage::read_from_file(file)?;
        
        // 2. Wyciągnij metadane bez wczytywania pikseli
        let meta = EXRMetadata {
            width: reader.layer_data().absolute_bounds().size().width(),
            height: reader.layer_data().absolute_bounds().size().height(),
            channels: extract_channel_info(&reader),
            layers: extract_layer_info(&reader),
            // ... pozostałe pola
        };
        
        Ok(meta)
    }
}
Etap 2: Generowanie miniaturki
rustpub fn generate_thumbnail(
    &mut self, 
    max_size: u32
) -> Result<Thumbnail, Error> {
    // 1. Oblicz współczynnik skalowania
    let scale = calculate_scale_factor(
        self.metadata.width, 
        self.metadata.height, 
        max_size
    );
    
    // 2. Wczytaj tylko potrzebne próbki (subsampling)
    let step = (1.0 / scale).ceil() as u32;
    
    let image = exr::prelude::ReadFirstImage::read_from_file(
        std::fs::File::open(&self.file_path)?
    )?
    .read_image(|resolution, _| {
        // Alokuj bufor dla miniaturki
        vec![vec![0.0f32; resolution.width() / step as usize]; 
             resolution.height() / step as usize]
    })?;
    
    // 3. Konwertuj do RGB i zastosuj tone mapping
    let thumbnail = self.create_thumbnail_from_samples(image, scale)?;
    
    Ok(thumbnail)
}
Etap 3: Wczytanie podglądu
rustpub fn load_preview(
    &self, 
    layer_idx: usize,
    mip_level: u32
) -> Result<PreviewImage, Error> {
    // 1. Sprawdź cache
    if let Some(cached) = self.preview_cache.get(&format!("{}_{}", layer_idx, mip_level)) {
        return Ok(cached.clone());
    }
    
    // 2. Wczytaj z odpowiednim poziomem szczegółowości
    let reader = exr::prelude::ReadImage::read_from_file(
        &self.file_path,
        ReadImageSettings {
            // Konfiguracja dla konkretnej warstwy
            layer: Some(layer_idx),
            ..Default::default()
        }
    )?;
    
    // 3. Zastosuj tone mapping i korekty
    let preview = self.process_for_display(reader)?;
    
    Ok(preview)
}
4. Optymalizacje wydajności
Asynchroniczne wczytywanie
rustpub async fn load_layers_async(&mut self) -> Result<(), Error> {
    let tasks: Vec<_> = self.metadata.layers
        .iter()
        .enumerate()
        .map(|(idx, layer)| {
            let path = self.file_path.clone();
            tokio::spawn(async move {
                Self::load_layer_data(path, idx).await
            })
        })
        .collect();
    
    let results = futures::future::join_all(tasks).await;
    // Przetwórz wyniki...
    
    Ok(())
}
Cache z LRU
rustuse lru::LruCache;

pub struct ThumbnailCache {
    cache: LruCache<String, Thumbnail>,
    max_memory: usize,
    current_memory: usize,
}
5. Obsługa różnych formatów kanałów
rustpub fn read_channel_data(
    &self, 
    layer: &str, 
    channel: &str
) -> Result<ChannelData, Error> {
    match channel {
        "R" | "G" | "B" => self.read_color_channel(layer, channel),
        "A" => self.read_alpha_channel(layer),
        "Z" => self.read_depth_channel(layer),
        _ => self.read_custom_channel(layer, channel),
    }
}
6. Struktura pliku Cargo.toml
toml[dependencies]
exr = "1.7"
image = "0.24"
rayon = "1.7"
tokio = { version = "1.0", features = ["full"] }
lru = "0.10"
futures = "0.3"
thiserror = "1.0"
7. Przykład użycia
rust#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut editor = EXREditor::new("example.exr").await?;
    
    // Szybka analiza
    let metadata = editor.get_metadata();
    println!("Wymiary: {}x{}", metadata.width, metadata.height);
    println!("Warstwy: {}", metadata.layers.len());
    
    // Miniaturka
    let thumbnail = editor.generate_thumbnail(256).await?;
    
    // Podgląd konkretnej warstwy
    let preview = editor.load_preview(0, 0).await?;
    
    Ok(())
}