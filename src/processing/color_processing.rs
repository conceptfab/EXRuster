use std::path::{Path, PathBuf};
use ::exr::meta::attribute::AttributeValue;
use glam::{DMat3, DVec3, Mat3};
use std::sync::LazyLock;
use lru::LruCache;
use std::sync::RwLock;

// Global cache dla color matrices - persistent między sesjami
// RwLock allows multiple concurrent readers, improving performance for cache hits
static COLOR_MATRIX_CACHE: LazyLock<RwLock<LruCache<(PathBuf, String), Mat3>>> = 
    LazyLock::new(|| RwLock::new(LruCache::new(std::num::NonZeroUsize::new(100).unwrap())));

// Make the main function public
pub fn compute_rgb_to_srgb_matrix_from_file_for_layer(path: &Path, layer_name: &str) -> anyhow::Result<Mat3> {
    // Odczytaj wyłącznie nagłówki/atrybuty (bez danych pikseli)
    // Wczytaj tylko meta-dane (nagłówki) bez pikseli
    let meta = ::exr::meta::MetaData::read_from_file(path, /*pedantic=*/false)?;
    let wanted_lower = layer_name.to_lowercase();
    let mut primaries: Option<(f64, f64, f64, f64, f64, f64, f64, f64)> = None;

    // Najpierw spróbuj z warstwy/partu
    'outer: for header in meta.headers.iter() {
        let base_name: Option<String> = header.own_attributes.layer_name.as_ref().map(|t| t.to_string());
        let lname = base_name.unwrap_or_else(|| "".to_string());
        let lname_lower = lname.to_lowercase();
        let matches = (wanted_lower.is_empty() && lname_lower.is_empty()) || (!wanted_lower.is_empty() && lname_lower.contains(&wanted_lower));
        if matches {
            if let Some((_, AttributeValue::Chromaticities(ch))) = header
                .own_attributes
                .other
                .iter()
                .find(|(k, _)| k.to_string().eq_ignore_ascii_case("chromaticities"))
            {
                primaries = Some((
                    ch.red.x() as f64, ch.red.y() as f64,
                    ch.green.x() as f64, ch.green.y() as f64,
                    ch.blue.x() as f64, ch.blue.y() as f64,
                    ch.white.x() as f64, ch.white.y() as f64,
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
                    ch.red.x() as f64, ch.red.y() as f64,
                    ch.green.x() as f64, ch.green.y() as f64,
                    ch.blue.x() as f64, ch.blue.y() as f64,
                    ch.white.x() as f64, ch.white.y() as f64,
                ));
            }
        }
    }

    let (rx, ry, gx, gy, bx, by, wx, wy) = primaries
        .ok_or_else(|| anyhow::anyhow!("chromaticities attribute not found or incomplete"))?;

    let m_src = rgb_to_xyz_from_primaries(rx, ry, gx, gy, bx, by, wx, wy);
    // Adaptacja Bradford do D65
    let m_adapt = bradford_adaptation_matrix((wx, wy), (0.3127, 0.3290));
    let m_xyz_to_srgb = xyz_to_srgb_matrix();
    let m = m_xyz_to_srgb * (m_adapt * m_src);
    Ok(m)
}

fn rgb_to_xyz_from_primaries(rx: f64, ry: f64, gx: f64, gy: f64, bx: f64, by: f64, wx: f64, wy: f64) -> Mat3 {
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
        3.2404542, -0.9692660, 0.0556434,
       -1.5371385,  1.8760108, -0.2040259,
       -0.4985314,  0.0415560, 1.0572252,
    ])
}

fn bradford_adaptation_matrix(src_xy: (f64, f64), dst_xy: (f64, f64)) -> Mat3 {
    // Bradford cone response matrix and its inverse (f64)
    let m = DMat3::from_cols_array(&[
        0.8951_f64, -0.7502, 0.0389,
        0.2664,      1.7135, -0.0685,
       -0.1614,      0.0367, 1.0296,
    ]);
    let m_inv = DMat3::from_cols_array(&[
         0.9869929,  0.4323053, -0.0085287,
        -0.1470543,  0.5183603,  0.0400428,
         0.1599627,  0.0492912,  0.9684867,
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

pub fn compute_rgb_to_srgb_matrix_from_file_for_layer_cached(path: &Path, layer_name: &str) -> anyhow::Result<Mat3> {
    let key = (path.to_path_buf(), layer_name.to_string());
    
    // Fast path: Try read lock first for cache hit (allows concurrent reads)
    // Use peek() instead of get() to avoid needing mutable access for LRU update
    if let Ok(cache) = COLOR_MATRIX_CACHE.read() {
        if let Some(&matrix) = cache.peek(&key) {
            println!("Color matrix cache HIT for {}:{}", path.display(), layer_name);
            return Ok(matrix);
        }
    }
    
    // Cache miss - oblicz nową macierz
    println!("Color matrix cache MISS for {}:{}, computing...", path.display(), layer_name);
    
    let matrix = compute_rgb_to_srgb_matrix_from_file_for_layer(path, layer_name)?;
    
    // Write path: Use write lock only for cache update
    if let Ok(mut cache) = COLOR_MATRIX_CACHE.write() {
        cache.put(key, matrix);
    }
    
    Ok(matrix)
}

#[allow(dead_code)]
pub fn get_color_matrix_cache_stats() -> (u64, u64, f32) {
    // Updated for RwLock - use read lock for stats
    if let Ok(cache) = COLOR_MATRIX_CACHE.read() {
        let len = cache.len() as u64;
        let cap = cache.cap().get() as u64;
        let usage = if cap > 0 { len as f32 / cap as f32 } else { 0.0 };
        (len, cap, usage)
    } else {
        (0, 0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    
    #[test]
    fn test_cache_concurrent_performance() {
        // Test that RwLock allows concurrent reads, improving performance
        let test_file = std::env::current_dir().unwrap().join("test.exr");
        
        // Pre-populate cache with a test entry
        let _ = COLOR_MATRIX_CACHE.write().unwrap().put(
            (test_file.clone(), "test".to_string()),
            Mat3::IDENTITY
        );
        
        let start = std::time::Instant::now();
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let test_file = test_file.clone();
                thread::spawn(move || {
                    // Simulate concurrent cache reads
                    for _ in 0..100 {
                        if let Ok(cache) = COLOR_MATRIX_CACHE.read() {
                            let _ = cache.peek(&(test_file.clone(), "test".to_string()));
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
        assert!(duration.as_millis() < 100, "Cache reads too slow: {:?}", duration);
    }
    
    #[test]
    fn test_cache_stats() {
        let (_len, cap, usage) = get_color_matrix_cache_stats();
        assert!(cap > 0);
        assert!(usage >= 0.0 && usage <= 1.0);
    }
}