use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Context;
use ::exr::meta::attribute::AttributeValue;
use crate::utils::{split_layer_and_short, human_size};

#[derive(Debug, Clone)]
pub struct MetadataGroup {
    pub name: String,
    pub items: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct LayerChannelsGroup {
    #[allow(dead_code)]
    pub group_name: String,         // np. "RGB", "Alpha", "Depth", "Cryptomatte", "Normals", "Motion", "Other"
    #[allow(dead_code)]
    pub channels: Vec<String>,      // krótkie nazwy kanałów w tej grupie
}

#[derive(Debug, Clone)]
pub struct LayerMetadata {
    pub name: String,               // pusta nazwa oznacza warstwę bazową bez prefiksu
    pub width: u32,
    pub height: u32,
    #[allow(dead_code)]
    pub channel_groups: Vec<LayerChannelsGroup>,
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct ExrMetadata {
    #[allow(dead_code)]
    pub path: PathBuf,
    #[allow(dead_code)]
    pub file_size_bytes: u64,
    pub groups: Vec<MetadataGroup>,
    pub layers: Vec<LayerMetadata>,
}

/// Publiczne API: odczytuje metadane z pliku EXR, porządkuje je i zwraca strukturę
pub fn read_and_group_metadata(path: &Path) -> anyhow::Result<ExrMetadata> {
    let meta = fs::metadata(path)
        .with_context(|| format!("Nie można pobrać metadata pliku: {}", path.display()))?;
    let file_size_bytes = meta.len();

    // Odczytaj wyłącznie meta-dane (nagłówki) bez pikseli
    let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/false)
        .with_context(|| format!("Błąd odczytu EXR (nagłówki): {}", path.display()))?;

    // Grupa ogólna (do UI): podstawowe informacje o pliku i obrazie
    let mut general_items: Vec<(String, String)> = Vec::new();
    general_items.push(("Ścieżka".into(), path.display().to_string()));
    general_items.push(("Rozmiar pliku".into(), human_size(file_size_bytes)));
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
        let pretty_value: String = match value {
            AttributeValue::Chromaticities(ch) => {
                // Bezpieczne formatowanie z precyzją, bez parsowania tekstu
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
            // Dla innych typów użyj czytelnego Debug bez dalszego parsowania w UI
            other => format!("{:?}", other),
        };

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

        // Grupowanie kanałów według logiki do UI
        let mut groups: GroupBuckets = GroupBuckets::new();
        for ch in &header.channels.list {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_layer_name.as_deref());
            let _ = lname; // lname nieużywane dalej, ale poprawne dla dopasowania
            groups.push(short);
        }

        let channel_groups: Vec<LayerChannelsGroup> = groups.into_sorted_vec();

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

            let pretty_value: String = match value {
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
            };
            layer_items.push((normalized_key, pretty_value));
        }
        layers.push(LayerMetadata { name: layer_name, width: w, height: h, channel_groups, attributes: layer_items });
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

    Ok(ExrMetadata { path: path.to_path_buf(), file_size_bytes, groups, layers })
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

// --- Pomocnicze: grupowanie kanałów ---

#[derive(Default)]
struct GroupBuckets {
    rgb: Vec<String>,
    alpha: Vec<String>,
    depth: Vec<String>,
    cryptomatte: Vec<String>,
    normals: Vec<String>,
    motion: Vec<String>,
    other: Vec<String>,
}

impl GroupBuckets {
    fn new() -> Self { Self::default() }

    fn push(&mut self, short_name: String) {
        let upper = short_name.to_ascii_uppercase();
        let group = classify_channel_group(&upper);
        match group {
            ChannelGroup::Rgb => self.rgb.push(short_name),
            ChannelGroup::Alpha => self.alpha.push(short_name),
            ChannelGroup::Depth => self.depth.push(short_name),
            ChannelGroup::Cryptomatte => self.cryptomatte.push(short_name),
            ChannelGroup::Normals => self.normals.push(short_name),
            ChannelGroup::Motion => self.motion.push(short_name),
            ChannelGroup::Other => self.other.push(short_name),
        }
    }

    fn into_sorted_vec(mut self) -> Vec<LayerChannelsGroup> {
        // Posortuj kanały wewnątrz grup alfabetycznie (case-insensitive)
    let sort_ci = |v: &mut Vec<String>| v.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        sort_ci(&mut self.rgb);
        sort_ci(&mut self.alpha);
        sort_ci(&mut self.depth);
        sort_ci(&mut self.cryptomatte);
        sort_ci(&mut self.normals);
        sort_ci(&mut self.motion);
        sort_ci(&mut self.other);

        // Ustal kolejność logiczną grup
        let mut out = Vec::new();
        out.push(LayerChannelsGroup { group_name: "RGB".into(), channels: self.rgb });
        out.push(LayerChannelsGroup { group_name: "Alpha".into(), channels: self.alpha });
        out.push(LayerChannelsGroup { group_name: "Depth".into(), channels: self.depth });
        out.push(LayerChannelsGroup { group_name: "Cryptomatte".into(), channels: self.cryptomatte });
        out.push(LayerChannelsGroup { group_name: "Normals".into(), channels: self.normals });
        out.push(LayerChannelsGroup { group_name: "Motion".into(), channels: self.motion });
        out.push(LayerChannelsGroup { group_name: "Inne".into(), channels: self.other });
        out
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum ChannelGroup { Rgb, Alpha, Depth, Cryptomatte, Normals, Motion, Other }

fn classify_channel_group(upper_short: &str) -> ChannelGroup {
    // RGB
    if matches!(upper_short, "R" | "G" | "B") { return ChannelGroup::Rgb; }

    // Alpha
    if upper_short == "A" || upper_short.starts_with("ALPHA") { return ChannelGroup::Alpha; }

    // Depth / Z / Distance
    if upper_short == "Z" || upper_short.contains("DEPTH") || upper_short == "DISTANCE" {
        return ChannelGroup::Depth;
    }

    // Cryptomatte
    if upper_short.contains("CRYPT") || upper_short.contains("MATTE") {
        return ChannelGroup::Cryptomatte;
    }

    // Normals (N, NORMAL, N.x/y/z, itp.)
    if upper_short.starts_with('N') || upper_short.contains("NORMAL") {
        return ChannelGroup::Normals;
    }

    // Motion (VX/VY/VZ, MOTION, SPEED)
    if upper_short.ends_with("VX") || upper_short.ends_with("VY") || upper_short.ends_with("VZ")
        || upper_short.contains("MOTION") || upper_short.contains("SPEED") {
        return ChannelGroup::Motion;
    }

    ChannelGroup::Other
}

// Rozdziela "warstwa.kanał" na (warstwa, kanał_krótki).
// Jeżeli nagłówek warstwy zawiera atrybut `layer_name`, używa go jako nazwy warstwy,
// w przeciwnym razie rozcina po ostatniej kropce.
// split_layer_and_short oraz human_size przeniesione do utils

// (usunięto nieużywane formatery wartości dla UI)
