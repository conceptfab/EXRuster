// Wspólne funkcje pomocnicze używane w wielu modułach
use slint::Color;
use crate::AppWindow;

#[inline]
pub(crate) fn split_layer_and_short(full: &str, base_attr: Option<&str>) -> (String, String) {
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
pub(crate) fn human_size(bytes: u64) -> String {
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
        "G" | "GREEN" => (ui.get_layers_color_g(), "🟢".to_string(), "Green".to_string()),
        "B" | "BLUE" => (ui.get_layers_color_b(), "🔵".to_string(), "Blue".to_string()),
        "A" | "ALPHA" => (ui.get_layers_color_default(), "⚪".to_string(), "Alpha".to_string()),
        _ => (ui.get_layers_color_default(), "•".to_string(), channel.to_string()),
    }
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
