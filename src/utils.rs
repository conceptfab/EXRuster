// Wspólne funkcje pomocnicze używane w wielu modułach

#[inline]
pub(crate) fn split_layer_and_short(full: &str, base_attr: Option<&str>) -> (String, String) {
    if let Some(base) = base_attr {
        let short = full.rsplit('.').next().unwrap_or(full).to_string();
        (base.to_string(), short)
    } else if let Some(p) = full.rfind('.') {
        (full[..p].to_string(), full[p + 1..].to_string())
    } else {
        ("".to_string(), full.to_string())
    }
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
