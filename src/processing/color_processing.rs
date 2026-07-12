use crate::{log_info, log_warn};
use ::exr::meta::attribute::AttributeValue;
use glam::{DMat3, DVec3, Mat3};
use lru::LruCache;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::RwLock;

// Global cache dla color matrices - persistent między sesjami
// RwLock allows multiple concurrent readers, improving performance for cache hits
//
// Klucz zawiera mtime pliku: dzięki temu ponowny zapis pliku "w miejscu" (typowy
// workflow artysty) unieważnia wpis, zamiast zwracać nieaktualny wynik — również
// nieaktualne `None` ("brak chromaticities").
type ColorMatrixCacheKey = (PathBuf, String, u64);
type ColorMatrixCache = LruCache<ColorMatrixCacheKey, Option<Mat3>>;

static COLOR_MATRIX_CACHE: LazyLock<RwLock<ColorMatrixCache>> =
    LazyLock::new(|| RwLock::new(LruCache::new(std::num::NonZeroUsize::new(100).unwrap())));

/// Mtime pliku w sekundach od epoki; 0 gdy nie da się go odczytać.
/// (Lokalny odpowiednik `file_mtime_u64` z `io::thumbnails` — celowo nie
/// współdzielony, by nie wiązać `processing` z `io::thumbnails`.)
fn file_mtime_u64(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn color_matrix_cache_key(path: &Path, layer_name: &str) -> ColorMatrixCacheKey {
    (
        path.to_path_buf(),
        layer_name.to_string(),
        file_mtime_u64(path),
    )
}

// Make the main function public
pub fn compute_rgb_to_srgb_matrix_from_file_for_layer(
    path: &Path,
    layer_name: &str,
) -> anyhow::Result<Option<Mat3>> {
    // Odczytaj wyłącznie nagłówki/atrybuty (bez danych pikseli)
    // Wczytaj tylko meta-dane (nagłówki) bez pikseli
    let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/ false)?;
    let wanted_lower = layer_name.to_lowercase();
    let mut primaries: Option<(f64, f64, f64, f64, f64, f64, f64, f64)> = None;

    // Najpierw spróbuj z warstwy/partu
    'outer: for header in meta.headers.iter() {
        let base_name: Option<String> = header
            .own_attributes
            .layer_name
            .as_ref()
            .map(|t| t.to_string());
        let lname = base_name.unwrap_or_default();
        let lname_lower = lname.to_lowercase();
        let matches = (wanted_lower.is_empty() && lname_lower.is_empty())
            || (!wanted_lower.is_empty() && lname_lower.contains(&wanted_lower));
        if matches {
            if let Some((_, AttributeValue::Chromaticities(ch))) = header
                .own_attributes
                .other
                .iter()
                .find(|(k, _)| k.to_string().eq_ignore_ascii_case("chromaticities"))
            {
                primaries = Some((
                    ch.red.x() as f64,
                    ch.red.y() as f64,
                    ch.green.x() as f64,
                    ch.green.y() as f64,
                    ch.blue.x() as f64,
                    ch.blue.y() as f64,
                    ch.white.x() as f64,
                    ch.white.y() as f64,
                ));
                break 'outer;
            }
        }
    }

    if primaries.is_none() {
        if let Some(first_header) = meta.headers.first() {
            if let Some((_, AttributeValue::Chromaticities(ch))) = first_header
                .shared_attributes
                .other
                .iter()
                .find(|(k, _)| k.to_string().eq_ignore_ascii_case("chromaticities"))
            {
                primaries = Some((
                    ch.red.x() as f64,
                    ch.red.y() as f64,
                    ch.green.x() as f64,
                    ch.green.y() as f64,
                    ch.blue.x() as f64,
                    ch.blue.y() as f64,
                    ch.white.x() as f64,
                    ch.white.y() as f64,
                ));
            }
        }
    }

    let Some((rx, ry, gx, gy, bx, by, wx, wy)) = primaries else {
        // No chromaticities attribute — a valid, cacheable outcome (assume sRGB).
        return Ok(None);
    };

    let m_src = rgb_to_xyz_from_primaries(rx, ry, gx, gy, bx, by, wx, wy);
    // Adaptacja Bradford do D65
    let m_adapt = bradford_adaptation_matrix((wx, wy), (0.3127, 0.3290));
    let m_xyz_to_srgb = xyz_to_srgb_matrix();
    Ok(Some(m_xyz_to_srgb * (m_adapt * m_src)))
}

#[allow(clippy::too_many_arguments)]
fn rgb_to_xyz_from_primaries(
    rx: f64,
    ry: f64,
    gx: f64,
    gy: f64,
    bx: f64,
    by: f64,
    wx: f64,
    wy: f64,
) -> Mat3 {
    // Zbuduj macierz kolumnami XYZ primaries, znormalizowaną tak, by biel dawała Y=1
    let rz = 1.0 - rx - ry;
    let gz = 1.0 - gx - gy;
    let bz = 1.0 - bx - by;

    let r = DVec3::new(rx / ry, 1.0, rz / ry);
    let g = DVec3::new(gx / gy, 1.0, gz / gy);
    let b = DVec3::new(bx / by, 1.0, bz / by);

    let m = DMat3::from_cols(r, g, b);

    // White point XYZ (Y=1)
    let wz = 1.0 - wx - wy;
    let w = DVec3::new(wx / wy, 1.0, wz / wy);

    // Rozwiąż M * s = w (scale factors)
    let s = m.inverse() * w;
    let scaled = DMat3::from_cols(r * s.x, g * s.y, b * s.z);
    // Konwersja do f32
    scaled.as_mat3()
}

fn xyz_to_srgb_matrix() -> Mat3 {
    // Stała macierz XYZ→sRGB (D65)
    Mat3::from_cols_array(&[
        3.2404542, -0.969_266, 0.0556434, -1.5371385, 1.8760108, -0.2040259, -0.4985314, 0.0415560,
        1.0572252,
    ])
}

fn bradford_adaptation_matrix(src_xy: (f64, f64), dst_xy: (f64, f64)) -> Mat3 {
    // Bradford cone response matrix and its inverse (f64)
    let m = DMat3::from_cols_array(&[
        0.8951_f64, -0.7502, 0.0389, 0.2664, 1.7135, -0.0685, -0.1614, 0.0367, 1.0296,
    ]);
    let m_inv = DMat3::from_cols_array(&[
        0.9869929, 0.4323053, -0.0085287, -0.1470543, 0.5183603, 0.0400428, 0.1599627, 0.0492912,
        0.9684867,
    ]);

    let src_xyz = xy_to_xyz(src_xy.0, src_xy.1);
    let dst_xyz = xy_to_xyz(dst_xy.0, dst_xy.1);

    // Compute cone response for source and destination whites
    let src_lms = m * src_xyz;
    let dst_lms = m * dst_xyz;
    let scale = DMat3::from_diagonal(DVec3::new(
        dst_lms.x / src_lms.x,
        dst_lms.y / src_lms.y,
        dst_lms.z / src_lms.z,
    ));

    // Build adaptation matrix: M_inv * S * M
    let tmp = m_inv * (scale * m);

    // Convert to f32 Mat3
    tmp.as_mat3()
}

fn xy_to_xyz(x: f64, y: f64) -> DVec3 {
    let z = 1.0 - x - y;
    DVec3::new(x / y, 1.0, z / y)
}

pub fn compute_rgb_to_srgb_matrix_from_file_for_layer_cached(
    path: &Path,
    layer_name: &str,
) -> Option<Mat3> {
    let key = color_matrix_cache_key(path, layer_name);

    // Fast path: Try read lock first for cache hit (allows concurrent reads)
    // Use peek() instead of get() to avoid needing mutable access for LRU update
    if let Ok(cache) = COLOR_MATRIX_CACHE.read() {
        if let Some(&cached) = cache.peek(&key) {
            log_info!(
                "Color matrix cache HIT for {}:{}",
                path.display(),
                layer_name
            );
            return cached;
        }
    }

    // Cache miss - oblicz nową macierz
    log_info!(
        "Color matrix cache MISS for {}:{}, computing...",
        path.display(),
        layer_name
    );

    match compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name) {
        Ok(result) => {
            // Cache both Some(matrix) and None ("no chromaticities") outcomes.
            if let Ok(mut cache) = COLOR_MATRIX_CACHE.write() {
                cache.put(key, result);
            }
            result
        }
        // I/O errors are not cached — the file may appear or change later.
        // Log them, otherwise a corrupt/unreadable header is indistinguishable
        // from a file that legitimately has no chromaticities attribute.
        Err(e) => {
            log_warn!(
                "Failed to read color matrix from {}:{} ({e}); assuming sRGB",
                path.display(),
                layer_name
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// Unikalna ścieżka w katalogu tymczasowym (brak zależności `tempfile` w projekcie).
    fn unique_temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exruster_{tag}_{}_{nanos}.exr", std::process::id()))
    }

    /// Zapisuje minimalny, poprawny plik EXR — bez atrybutu `chromaticities`.
    fn write_exr_without_chromaticities(path: &Path) {
        ::exr::prelude::write_rgba_file(path, 2, 2, |_x, _y| (0.5f32, 0.5f32, 0.5f32, 1.0f32))
            .expect("failed to write test exr");
    }

    #[test]
    fn none_result_is_cached_for_file_without_chromaticities() {
        let path = unique_temp_path("no_chroma");
        write_exr_without_chromaticities(&path);

        // Plik bez chromaticities → brak macierzy (traktujemy jak sRGB).
        let result = compute_rgb_to_srgb_matrix_from_file_for_layer_cached(&path, "");
        assert!(
            result.is_none(),
            "file has no chromaticities → expected None"
        );

        // Sedno zmiany: negatywny wynik JEST w cache'u (wpis istnieje, wartość = None),
        // więc kolejne kliknięcia warstwy nie czytają nagłówka z dysku ponownie.
        let key = color_matrix_cache_key(&path, "");
        let cached = COLOR_MATRIX_CACHE.read().unwrap().peek(&key).copied();
        assert_eq!(
            cached,
            Some(None),
            "negative (None) outcome must be cached, not recomputed on every call"
        );

        // Powtórne wywołanie nadal zwraca None (tym razem z cache'u).
        assert!(compute_rgb_to_srgb_matrix_from_file_for_layer_cached(&path, "").is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn io_error_is_not_cached() {
        let path = unique_temp_path("missing");
        assert!(!path.exists(), "test path must not exist");

        // Nieistniejący plik → błąd I/O → None, ale NIC nie trafia do cache'u
        // (plik może się pojawić później).
        let result = compute_rgb_to_srgb_matrix_from_file_for_layer_cached(&path, "");
        assert!(result.is_none(), "I/O error surfaces as None");

        let key = color_matrix_cache_key(&path, "");
        let cached = COLOR_MATRIX_CACHE.read().unwrap().peek(&key).copied();
        assert_eq!(cached, None, "I/O errors must NOT create a cache entry");
    }

    #[test]
    fn cache_key_includes_file_mtime() {
        // Klucz zawiera mtime pliku — zapis pliku "w miejscu" daje inny klucz,
        // więc stary (również negatywny) wynik nie jest zwracany jako aktualny.
        let path = unique_temp_path("mtime");
        write_exr_without_chromaticities(&path);

        let key = color_matrix_cache_key(&path, "");
        assert_eq!(key.0, path);
        assert_ne!(
            key.2, 0,
            "existing file must have a non-zero mtime in the key"
        );

        // Ten sam plik z inną mtime → inny klucz (a więc cache miss).
        let stale_key = (key.0.clone(), key.1.clone(), key.2 - 1);
        assert_ne!(key, stale_key);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_cache_concurrent_performance() {
        // Test that RwLock allows concurrent reads, improving performance
        let test_file = std::env::current_dir().unwrap().join("test.exr");

        // Pre-populate cache with a test entry
        let _ = COLOR_MATRIX_CACHE.write().unwrap().put(
            (test_file.clone(), "test".to_string(), 0),
            Some(Mat3::IDENTITY),
        );

        let start = std::time::Instant::now();
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let test_file = test_file.clone();
                thread::spawn(move || {
                    // Simulate concurrent cache reads
                    for _ in 0..100 {
                        if let Ok(cache) = COLOR_MATRIX_CACHE.read() {
                            let _ = cache.peek(&(test_file.clone(), "test".to_string(), 0));
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let duration = start.elapsed();
        println!("Concurrent cache read test took: {:?}", duration);

        // With RwLock, this should be much faster than with Mutex
        // because reads can happen concurrently
        assert!(
            duration.as_millis() < 100,
            "Cache reads too slow: {:?}",
            duration
        );
    }
}
