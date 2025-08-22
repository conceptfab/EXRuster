use std::fs;
use std::path::Path;
use anyhow::Context;
use ::exr::meta::attribute::AttributeValue;
use crate::utils::human_size;
use crate::io::fast_exr_metadata::{read_exr_metadata_ultra_fast, FastEXRMetadata};
use crate::processing::channel_classification::{group_channels_parallel, determine_channel_group_with_config};
use crate::utils::channel_config::{load_channel_config, get_fallback_config};

#[derive(Debug, Clone)]
pub struct MetadataGroup {
    pub name: String,
    pub items: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct LayerMetadata {
    pub name: String,               // pusta nazwa oznacza warstwę bazową bez prefiksu
    pub width: u32,
    pub height: u32,
    // Usunięte nieużywane pole channel_groups
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct ExrMetadata {
    // Usunięte nieużywane pola path i file_size_bytes
    pub groups: Vec<MetadataGroup>,
    pub layers: Vec<LayerMetadata>,
}

/// Publiczne API: odczytuje metadane z pliku EXR, porządkuje je i zwraca strukturę
/// Wykorzystuje ultra-szybki parser z optymalizacjami SIMD
pub fn read_and_group_metadata(path: &Path) -> anyhow::Result<ExrMetadata> {
    // Spróbuj najpierw ultra-szybkiej metody
    match read_and_group_metadata_fast(path) {
        Ok(metadata) => return Ok(metadata),
        Err(e) => {
            eprintln!("Fast metadata reading failed, falling back to standard method: {}", e);
        }
    }
    
    // Fallback do standardowej metody
    read_and_group_metadata_standard(path)
}

/// Ultra-szybka metoda odczytu metadanych z optymalizacjami
fn read_and_group_metadata_fast(path: &Path) -> anyhow::Result<ExrMetadata> {
    let fast_meta = read_exr_metadata_ultra_fast(path)
        .with_context(|| format!("Błąd ultra-szybkiego odczytu EXR: {}", path.display()))?;
    
    // Grupa ogólna: podstawowe informacje o pliku i obrazie
    let mut general_items: Vec<(String, String)> = Vec::new();
    general_items.push(("Ścieżka".into(), path.display().to_string()));
    general_items.push(("Rozmiar pliku".into(), human_size(fs::metadata(path).map(|m| m.len()).unwrap_or(0))));
    general_items.push(("Kanały".into(), fast_meta.channels.len().to_string()));
    
    // Nagłówek z ultra-szybkich metadanych
    let mut header_items: Vec<(String, String)> = Vec::new();
    header_items.push(("display_window".into(), format!("{:?}", fast_meta.display_window)));
    header_items.push(("pixel_aspect".into(), format!("{:.3}", fast_meta.pixel_aspect)));
    header_items.push(("compression".into(), fast_meta.compression.clone()));
    header_items.push(("line_order".into(), fast_meta.line_order.clone()));
    
    if let Some(layer_name) = &fast_meta.layer_name {
        header_items.push(("layer_name".into(), layer_name.clone()));
    }
    
    // Dodaj niestandardowe atrybuty
    for (name, value) in &fast_meta.custom_attributes {
        header_items.push((name.clone(), value.clone()));
    }
    
    // Grupowanie kanałów z użyciem słownika z pliku JSON
    let config = load_channel_config().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load channel config: {}. Using fallback.", e);
        get_fallback_config()
    });
    let channel_groups = group_channels_parallel(&fast_meta.channels, Some(&config));
    
    // Dodaj informacje o grupach kanałów do nagłówka
    let mut channel_info = Vec::new();
    for (group_name, channels) in &channel_groups {
        channel_info.push(format!("{}: {}", group_name, channels.join(", ")));
    }
    header_items.push(("channel_groups".into(), channel_info.join("; ")));
    
    let mut groups: Vec<MetadataGroup> = Vec::with_capacity(3);
    groups.push(MetadataGroup { name: "Ogólne".into(), items: general_items });
    groups.push(MetadataGroup { name: "Nagłówek".into(), items: header_items });
    
    // Dodaj grupę z podziałem kanałów
    let mut channel_group_items = Vec::new();
    for (group_name, channels) in channel_groups {
        channel_group_items.push((group_name, channels.join(", ")));
    }
    groups.push(MetadataGroup { name: "Grupy kanałów".into(), items: channel_group_items });
    
    // Warstwy - używamy informacji z fast metadata
    let layer_name = fast_meta.layer_name.unwrap_or_else(|| "".to_string());
    let layers = vec![LayerMetadata {
        name: layer_name,
        width: (fast_meta.display_window.2 - fast_meta.display_window.0 + 1) as u32,
        height: (fast_meta.display_window.3 - fast_meta.display_window.1 + 1) as u32,
        attributes: Vec::new(),
    }];
    
    Ok(ExrMetadata { groups, layers })
}

/// Standardowa metoda odczytu metadanych (fallback)
fn read_and_group_metadata_standard(path: &Path) -> anyhow::Result<ExrMetadata> {
    // Odczytaj wyłącznie meta-dane (nagłówki) bez pikseli
    let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/false)
        .with_context(|| format!("Błąd odczytu EXR (nagłówki): {}", path.display()))?;

    // Grupa ogólna (do UI): podstawowe informacje o pliku i obrazie
    let mut general_items: Vec<(String, String)> = Vec::new();
    general_items.push(("Ścieżka".into(), path.display().to_string()));
    general_items.push(("Rozmiar pliku".into(), human_size(fs::metadata(path).map(|m| m.len()).unwrap_or(0))));
    general_items.push(("Warstwy".into(), meta.headers.len().to_string()));

    // Zbierz nagłówek pliku jako key→value. Preferuj typowane atrybuty
    let mut header_items: Vec<(String, String)> = Vec::new();

    // Dodaj podstawowe, typowane pola nagłówka jeśli są dostępne
    // display_window: pozostawiamy format Debug (nie parsujemy dalej w UI)
    // Użyj atrybutów wspólnych z pierwszego nagłówka (w EXR są wspólne dla wszystkich headerów)
    let shared = &meta.headers.first().ok_or_else(|| anyhow::anyhow!("Brak nagłówków w pliku EXR"))?.shared_attributes;
    header_items.push(("display_window".into(), format!("{:?}", shared.display_window)));

    // pixel_aspect: spróbuj z pola typowanego; jeżeli 0 (brak), nadpisze go atrybutem z listy 'other' poniżej
    // (nie wszystkie pliki muszą mieć wpisane to pole wprost w strukturze)
    let pixel_aspect_value = format!("{:.3}", shared.pixel_aspect as f64);
    header_items.push(("pixel_aspect".into(), pixel_aspect_value));

    // Pozostałe atrybuty z listy `other` — spróbuj sformatować znane typy, reszta fallback do Debug
    for (raw_name, value) in shared.other.iter() {
        let name_lower = raw_name.to_string().to_ascii_lowercase();

        // Znormalizuj niektóre często spotykane nazwy
        let normalized_key = if name_lower == "pixel_aspect_ratio" || name_lower == "pixel_aspect" {
            "pixel_aspect".to_string()
        } else if name_lower == "chromaticities" {
            "chromaticities".to_string()
        } else if name_lower == "time_code" || name_lower == "timecode" {
            "time_code".to_string()
        } else {
            raw_name.to_string()
        };

        // Sformatuj wartość bazując na typie
        let pretty_value = format_attribute_value(value, &normalized_key);

        // Jeśli już dodaliśmy wpis o tym samym kluczu (np. pixel_aspect z pola), nadpiszemy go informacją z `other`
        if let Some(existing) = header_items.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&normalized_key)) {
            existing.1 = pretty_value;
        } else {
            header_items.push((normalized_key, pretty_value));
        }
    }
    // Pre-allocate groups vector with known size (at least 2 groups)
    let mut groups: Vec<MetadataGroup> = Vec::with_capacity(2 + meta.headers.len());
    groups.push(MetadataGroup { name: "Ogólne".into(), items: general_items });
    groups.push(MetadataGroup { name: "Nagłówek".into(), items: header_items });

    // Buduj warstwy i ich grupy kanałów
    let mut layers: Vec<LayerMetadata> = Vec::with_capacity(meta.headers.len());
    for header in meta.headers.iter() {
        let base_layer_name: Option<String> = header.own_attributes.layer_name.as_ref().map(|t| t.to_string());

        let w = header.layer_size.width() as u32;
        let h = header.layer_size.height() as u32;

        // Usunięte grupowanie kanałów - nieużywane
        // let channel_groups: Vec<LayerChannelsGroup> = groups.into_sorted_vec();

        // Nazwa warstwy (pusta dla warstwy bazowej)
        let layer_name = base_layer_name.unwrap_or_else(|| "".to_string());
        // Atrybuty warstwy: preferuj typowane wartości, fallback do Debug
        // Pre-allocate based on estimated number of attributes
        let mut layer_items: Vec<(String, String)> = Vec::with_capacity(header.own_attributes.other.len());
        for (raw_name, value) in header.own_attributes.other.iter() {
            let name_lower = raw_name.to_string().to_ascii_lowercase();
            let normalized_key = if name_lower == "pixel_aspect_ratio" || name_lower == "pixel_aspect" {
                "pixel_aspect".to_string()
            } else if name_lower == "chromaticities" {
                "chromaticities".to_string()
            } else if name_lower == "time_code" || name_lower == "timecode" {
                "time_code".to_string()
            } else {
                raw_name.to_string()
            };

            let pretty_value = format_attribute_value(value, &normalized_key);
            layer_items.push((normalized_key, pretty_value));
        }
        layers.push(LayerMetadata { name: layer_name, width: w, height: h, attributes: layer_items });
    }

    // Posortuj warstwy: najpierw bez nazwy (bazowa), potem alfabetycznie
    layers.sort_by(|a, b| {
        match (a.name.is_empty(), b.name.is_empty()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    Ok(ExrMetadata { groups, layers })
}

/// Akcesorium: przygotuj proste linie tekstowe na potrzeby UI (np. lista stringów)
pub fn build_ui_lines(meta: &ExrMetadata) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // Sekcje pliku (Ogólne, Nagłówek)
    for g in &meta.groups {
        out.push(format!("[{}]", g.name));
        for (k, v) in &g.items {
            if k.is_empty() { out.push(v.clone()); } else { out.push(format!("{}: {}", k, v)); }
        }
    }

    // Warstwy (tylko atrybuty; bez list kanałów)
    for layer in &meta.layers {
        let title = if layer.name.is_empty() { "(domyślna)".to_string() } else { layer.name.clone() };
        out.push(format!("Warstwa: {}  — {}x{}", title, layer.width, layer.height));
        // Atrybuty warstwy jako key→value
        for (k, v) in &layer.attributes {
            if k.is_empty() { out.push(format!("  {}", v)); } else { out.push(format!("  {}: {}", k, v)); }
        }
    }

    out
}

/// Alternatywa pod tabelę dwukolumnową: buduje pary (klucz, wartość)
pub fn build_ui_rows(meta: &ExrMetadata) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    // Sekcja: Ogólne
    rows.push(("Ogólne".into(), "".into()));
    for g in &meta.groups {
        if g.name == "Ogólne" {
            for (k, v) in &g.items { rows.push((k.clone(), v.clone())); }
        }
    }

    // Sekcja: Nagłówek (wartości już są przygotowane typowo w read_and_group_metadata)
    rows.push(("Nagłówek".into(), "".into()));
    for g in &meta.groups {
        if g.name == "Nagłówek" {
            for (k, v) in &g.items {
                rows.push((k.clone(), v.clone()));
            }
        }
    }

    // Sekcja: Warstwy
    for layer in &meta.layers {
        let label = if layer.name.is_empty() { "(domyślna)".to_string() } else { layer.name.clone() };
        rows.push((format!("Warstwa: {}", label), "".into()));
        rows.push(("Wymiary".into(), format!("{}x{}", layer.width, layer.height)));
        for (k, v) in &layer.attributes {
            let key = k.trim();
            let pretty_k = if key.is_empty() { "Atrybut".to_string() } else { key.to_string() };
            rows.push((pretty_k, v.clone()));
        }
    }
    rows
}

// Usunięte całe grupowanie kanałów - nieużywane
// --- Pomocnicze: grupowanie kanałów ---
// struct GroupBuckets, impl GroupBuckets, enum ChannelGroup, fn classify_channel_group zostały usunięte

// Funkcja pomocnicza do formatowania wartości atrybutów
fn format_attribute_value(value: &AttributeValue, normalized_key: &str) -> String {
    match value {
        AttributeValue::Chromaticities(ch) => {
            let r = (ch.red.x() as f64, ch.red.y() as f64);
            let g = (ch.green.x() as f64, ch.green.y() as f64);
            let b = (ch.blue.x() as f64, ch.blue.y() as f64);
            let w = (ch.white.x() as f64, ch.white.y() as f64);
            format!(
                "R: ({:.3},{:.3})  G: ({:.3},{:.3})  B: ({:.3},{:.3})  W: ({:.3},{:.3})",
                r.0, r.1, g.0, g.1, b.0, b.1, w.0, w.1
            )
        }
        AttributeValue::F32(v) => {
            if normalized_key.eq_ignore_ascii_case("pixel_aspect") {
                format!("{:.3}", *v as f64)
            } else {
                format!("{:.3}", *v as f64)
            }
        }
        AttributeValue::F64(v) => {
            if normalized_key.eq_ignore_ascii_case("pixel_aspect") {
                format!("{:.3}", v)
            } else {
                format!("{:.3}", v)
            }
        }
        other => format!("{:?}", other),
    }
}

/// Publiczne API dla ultra-szybkiego odczytu z grupowaniem kanałów
/// Używa słownika z channel_groups.json
pub fn read_fast_metadata_with_channels(path: &Path) -> anyhow::Result<(FastEXRMetadata, std::collections::HashMap<String, Vec<String>>)> {
    let fast_meta = read_exr_metadata_ultra_fast(path)
        .with_context(|| format!("Błąd ultra-szybkiego odczytu EXR: {}", path.display()))?;
    
    let config = load_channel_config().unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load channel config: {}. Using fallback.", e);
        get_fallback_config()
    });
    let channel_groups = group_channels_parallel(&fast_meta.channels, Some(&config));
    
    Ok((fast_meta, channel_groups))
}
