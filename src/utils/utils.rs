// Wspólne funkcje pomocnicze używane w wielu modułach
use crate::AppWindow;
use slint::Color;

#[inline]
pub fn split_layer_and_short(full: &str, base_attr: Option<&str>) -> (String, String) {
    let result = if let Some(base) = base_attr {
        let short = full.rsplit('.').next().unwrap_or(full).to_string();
        (base.to_string(), short)
    } else if let Some(p) = full.rfind('.') {
        (full[..p].to_string(), full[p + 1..].to_string())
    } else {
        ("".to_string(), full.to_string())
    };

    result
}

#[inline]
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", size, UNITS[unit])
    }
}

/// Wspólna funkcja mapowania kanałów na kolor, emoji i przyjazną nazwę do wyświetlenia
#[inline]
pub fn get_channel_info(channel: &str, ui: &AppWindow) -> (Color, String, String) {
    let upper = channel.trim().to_ascii_uppercase();
    match upper.as_str() {
        "R" | "RED" => (ui.get_layers_color_r(), "🔴".to_string(), "Red".to_string()),
        "G" | "GREEN" => (
            ui.get_layers_color_g(),
            "🟢".to_string(),
            "Green".to_string(),
        ),
        "B" | "BLUE" => (
            ui.get_layers_color_b(),
            "🔵".to_string(),
            "Blue".to_string(),
        ),
        "A" | "ALPHA" => (
            ui.get_layers_color_default(),
            "⚪".to_string(),
            "Alpha".to_string(),
        ),
        _ => (
            ui.get_layers_color_default(),
            "•".to_string(),
            channel.to_string(),
        ),
    }
}

/// Normalizuje nazwę do wyświetlenia: mapuje popularne warianty Unicode
/// (pełnoszerokościowe, typograficzne, matematyczne) na odpowiedniki ASCII,
/// żeby font UI (Geist) nie renderował ich jako „tofu" (□).
/// Nie dotyka oryginalnej ścieżki pliku — tylko stringa widocznego w UI.
pub fn normalize_display_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let replacement = match c {
            // pełnoszerokościowe nawiasy i znaki
            '\u{FF08}' => Some('('),
            '\u{FF09}' => Some(')'),
            '\u{FF3B}' => Some('['),
            '\u{FF3D}' => Some(']'),
            '\u{FF5B}' => Some('{'),
            '\u{FF5D}' => Some('}'),
            '\u{FF0D}' => Some('-'),
            '\u{FF0E}' => Some('.'),
            '\u{FF0F}' => Some('/'),
            '\u{FF3F}' => Some('_'),
            '\u{FF3C}' => Some('\\'),
            // nawiasy specjalne
            '\u{2768}' | '\u{276A}' | '\u{27EE}' | '\u{2985}' => Some('('),
            '\u{2769}' | '\u{276B}' | '\u{27EF}' | '\u{2986}' => Some(')'),
            '\u{276C}' | '\u{2770}' | '\u{27E8}' => Some('<'),
            '\u{276D}' | '\u{2771}' | '\u{27E9}' => Some('>'),
            '\u{3010}' | '\u{3014}' | '\u{27E6}' => Some('['),
            '\u{3011}' | '\u{3015}' | '\u{27E7}' => Some(']'),
            // typograficzne myślniki i cudzysłowy
            '\u{2013}' | '\u{2014}' | '\u{2212}' => Some('-'),
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => Some('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => Some('"'),
            _ => None,
        };
        match replacement {
            Some(a) => out.push(a),
            None => out.push(c),
        }
    }
    out
}

/// Normalizacja nazw kanałów do standardowych skrótów R/G/B/A
#[inline]
pub fn normalize_channel_name(channel: &str) -> String {
    let upper = channel.trim().to_ascii_uppercase();
    match upper.as_str() {
        "RED" => "R".to_string(),
        "GREEN" => "G".to_string(),
        "BLUE" => "B".to_string(),
        "ALPHA" => "A".to_string(),
        _ => channel.to_string(),
    }
}
