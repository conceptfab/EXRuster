use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Context;
use exr::prelude as exr;

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

    // Pełne dane o warstwach i kanałach (bez potrzeby czytania pikseli)
    let image = exr::read_all_data_from_file(path)
        .with_context(|| format!("Błąd odczytu EXR (nagłówki): {}", path.display()))?;

    // Grupa ogólna (do UI): podstawowe informacje o pliku i obrazie
    let mut general_items: Vec<(String, String)> = Vec::new();
    general_items.push(("Ścieżka".into(), path.display().to_string()));
    general_items.push(("Rozmiar pliku".into(), human_size(file_size_bytes)));
    general_items.push(("Warstwy".into(), image.layer_data.len().to_string()));

    // Zbierz nagłówek pliku jako key→value (parsowanie z Debug → spłaszczone do "klucz: wartość")
    let image_header_debug = format!("{:#?}", image.attributes);
    let header_items = parse_attributes_debug_flat(&image_header_debug);
    let mut groups: Vec<MetadataGroup> = Vec::new();
    groups.push(MetadataGroup { name: "Ogólne".into(), items: general_items });
    groups.push(MetadataGroup { name: "Nagłówek".into(), items: header_items });

    // Buduj warstwy i ich grupy kanałów
    let mut layers: Vec<LayerMetadata> = Vec::with_capacity(image.layer_data.len());
    for layer in image.layer_data.iter() {
        let base_layer_name: Option<String> = layer
            .attributes
            .layer_name
            .as_ref()
            .map(|s| s.to_string());

        let w = layer.size.width() as u32;
        let h = layer.size.height() as u32;

        // Grupowanie kanałów według logiki do UI
        let mut groups: GroupBuckets = GroupBuckets::new();
        for ch in &layer.channel_data.list {
            let full = ch.name.to_string();
            let (lname, short) = split_layer_and_short(&full, base_layer_name.as_deref());
            let _ = lname; // lname nieużywane dalej, ale poprawne dla dopasowania
            groups.push(short);
        }

        let channel_groups: Vec<LayerChannelsGroup> = groups.into_sorted_vec();

        // Nazwa warstwy (pusta dla warstwy bazowej)
        let layer_name = base_layer_name.unwrap_or_else(|| "".to_string());
        // Atrybuty warstwy (Debug → key/value)
        let layer_header_debug = format!("{:#?}", layer.attributes);
        let layer_items = parse_attributes_debug_flat(&layer_header_debug);
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

    // Sekcja: Nagłówek (wybrane i sformatowane klucze)
    rows.push(("Nagłówek".into(), "".into()));
    for g in &meta.groups {
        if g.name == "Nagłówek" {
            for (k, v) in &g.items {
                let key = k.trim();
                // Normalizacje popularnych pól, resztę przepuszczamy jak jest
                let (out_k, out_v) = if key.eq_ignore_ascii_case("display_window") {
                    ("display_window".to_string(), pretty_display_window(v))
                } else if key.eq_ignore_ascii_case("pixel_aspect") || key.eq_ignore_ascii_case("pixel_aspect_ratio") {
                    ("pixel_aspect".to_string(), pretty_number(v, 3))
                } else if key.eq_ignore_ascii_case("chromaticities") {
                    ("chromaticities".to_string(), pretty_chromaticities(v))
                } else if key.eq_ignore_ascii_case("time_code") {
                    ("time_code".to_string(), v.replace('{', "").replace('}', "").replace(',', " "))
                } else {
                    (key.to_string(), v.clone())
                };
                rows.push((out_k, out_v));
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
fn split_layer_and_short(full: &str, base_attr: Option<&str>) -> (String, String) {
    if let Some(base) = base_attr {
        let short = full.rsplit('.').next().unwrap_or(full).to_string();
        (base.to_string(), short)
    } else if let Some(p) = full.rfind('.') {
        (full[..p].to_string(), full[p + 1..].to_string())
    } else {
        ("".to_string(), full.to_string())
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"]; 
    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{} {}", bytes, UNITS[unit]) } else { format!("{:.2} {}", size, UNITS[unit]) }
}

fn parse_attributes_debug_flat(debug_str: &str) -> Vec<(String, String)> {
    // Heurystyczne spłaszczenie pretty-printed Debug do par "klucz: wartość".
    // Działa dla:
    // - key: SimpleValue
    // - key: Type( ...scalar lines... )
    // - key: Type { ...scalar lines... }
    // Zagnieżdżenia łączymy w jedną linię wartości (po spacji), pomijając nawiasy i przecinki.
    let lines: Vec<&str> = debug_str.lines().collect();
    let mut i = 0usize;
    let mut out: Vec<(String, String)> = Vec::new();

    // Pomiń nagłówki typu "ImageAttributes {"/"LayerAttributes ...{" itp.
    while i < lines.len() {
        let t = lines[i].trim();
        if t.is_empty() { i += 1; continue; }

        // Zamkniecia bloków – ignoruj
        if t == "}" || t == ")," || t == ")" || t == "}," { i += 1; continue; }

        // Jeśli linia ma postać key: value... (bez otwierania nowego bloku) -> emituj
        if let Some(pos) = t.find(':') {
            let key = t[..pos].trim().trim_matches('"').to_string();
            let rest = t[pos+1..].trim();

            // Jeżeli rest kończy się '{' lub '(' → zbieraj do zamknięcia jako jedna wartość
            let opens_block = rest.ends_with('{') || rest.ends_with('(');
            if opens_block {
                // Ustal jaki nawias kończy blok
                let _closing = if rest.ends_with('{') { '}' } else { ')' };
                let mut depth: i32 = 1; // mamy już jeden otwarty poziom
                let mut acc: Vec<String> = Vec::new();
                i += 1;
                while i < lines.len() && depth > 0 {
                    let s = lines[i].trim();
                    // zliczaj zagnieżdżenia
                    if s.ends_with('{') { depth += 1; }
                    if s.ends_with('(') { depth += 1; }
                    if s.starts_with('}') || s.starts_with(')') { depth -= 1; i += 1; continue; }
                    if s == "{" || s == "}" || s == "(" || s == ")" { i += 1; continue; }
                    if depth > 0 {
                        // wyczyść przecinki i cudzysłowy brzegowe
                        let mut v = s.trim_end_matches(',').to_string();
                        // wyciągnij zawartość w cudzysłowie jeśli jest
                        if v.matches('"').count() >= 2 {
                            if let Some(first) = v.find('"') {
                                if let Some(last) = v.rfind('"') { if last > first { v = v[first+1..last].to_string(); } }
                            }
                        }
                        if !v.is_empty() { acc.push(v); }
                    }
                    i += 1;
                }
                let value = acc.join(" ");
                if !key.is_empty() && !value.is_empty() { out.push((key, value)); }
                continue; // już przesunęliśmy i
            } else {
                // Prosta wartość w tej samej linii
                let mut value = rest.trim_end_matches(',').to_string();
                // Usuń zewnętrzne cudzysłowy
                if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                    value = value[1..value.len()-1].to_string();
                }
                out.push((key, value));
                i += 1;
                continue;
            }
        }

        // Linia bez ':' – pomiń
        i += 1;
    }

    out
}

// --- Proste formatery wartości dla UI ---

fn pretty_number(s: &str, decimals: usize) -> String {
    let x: Option<f64> = s.trim().parse().ok();
    match (x, decimals) {
        (Some(v), 0) => format!("{:.0}", v),
        (Some(v), 1) => format!("{:.1}", v),
        (Some(v), 2) => format!("{:.2}", v),
        (Some(v), 3) => format!("{:.3}", v),
        (Some(v), _) => format!("{:.3}", v),
        _ => s.to_string(),
    }
}

fn pretty_vec2_tuple(s: &str) -> Option<(i64, i64)> {
    // próba wyłuskania dwóch liczb z tekstu typu "Vec2( 0 0 size: Vec2( 2200 1237)"
    let mut nums: Vec<i64> = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '-' { cur.push(c); }
        else {
            if !cur.is_empty() { if let Ok(v) = cur.parse() { nums.push(v); } cur.clear(); }
        }
    }
    if !cur.is_empty() { if let Ok(v) = cur.parse() { nums.push(v); } }
    if nums.len() >= 2 { Some((nums[0], nums[1])) } else { None }
}

fn pretty_display_window(v: &str) -> String {
    // Oczekiwany wzorzec: pozycja (x,y) i rozmiar (w,h) gdzie w/h zwykle pojawiają się dalej w ciągu
    let pos = pretty_vec2_tuple(v).unwrap_or((0, 0));
    // spróbuj też wyłuskać kolejne dwie liczby jako rozmiar
    let mut nums: Vec<i64> = Vec::new();
    let mut cur = String::new();
    for c in v.chars() {
        if c.is_ascii_digit() || c == '-' { cur.push(c); }
        else { if !cur.is_empty() { if let Ok(n) = cur.parse() { nums.push(n); } cur.clear(); }
        }
    }
    if !cur.is_empty() { if let Ok(n) = cur.parse() { nums.push(n); } }
    let size = if nums.len() >= 4 { (nums[2], nums[3]) } else { (0, 0) };
    format!("position: ({}, {}); size: {}x{}", pos.0, pos.1, size.0, size.1)
}

fn pretty_chromaticities(v: &str) -> String {
    // heurystyczny parser: wyciągnij pary (x,y) w kolejności R,G,B,W
    let mut nums: Vec<f64> = Vec::new();
    let mut cur = String::new();
    for c in v.chars() {
        if c.is_ascii_digit() || c == '.' || c == '-' { cur.push(c); }
        else { if !cur.is_empty() { if let Ok(n) = cur.parse() { nums.push(n); } cur.clear(); }
        }
    }
    if !cur.is_empty() { if let Ok(n) = cur.parse() { nums.push(n); } }
    let mut xy = vec![(0.0,0.0); 4];
    for i in 0..4 { if nums.len() >= (i*2+2) { xy[i] = (nums[i*2], nums[i*2+1]); } }
    format!(
        "R: ({:.3},{:.3})  G: ({:.3},{:.3})  B: ({:.3},{:.3})  W: ({:.3},{:.3})",
        xy[0].0, xy[0].1, xy[1].0, xy[1].1, xy[2].0, xy[2].1, xy[3].0, xy[3].1
    )
}
