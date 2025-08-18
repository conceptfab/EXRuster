use std::fs;
use std::path::Path;
use anyhow::Context;
use ::exr::meta::attribute::AttributeValue;
use crate::utils::human_size;

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
pub fn read_and_group_metadata(path: &Path) -> anyhow::Result<ExrMetadata> {
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
    let mut groups: Vec<MetadataGroup> = Vec::new();
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
        let mut layer_items: Vec<(String, String)> = Vec::new();
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
