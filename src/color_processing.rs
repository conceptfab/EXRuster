use std::path::Path;
use ::exr::meta::attribute::AttributeValue;

// Make the main function public
pub fn compute_rgb_to_srgb_matrix_from_file_for_layer(path: &Path, layer_name: &str) -> anyhow::Result<[[f32; 3]; 3]> {
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

    // Fallback: globalny nagłówek
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
    let m = mul3x3(m_xyz_to_srgb, mul3x3(m_adapt, m_src));
    Ok(m)
}

fn rgb_to_xyz_from_primaries(rx: f64, ry: f64, gx: f64, gy: f64, bx: f64, by: f64, wx: f64, wy: f64) -> [[f32; 3]; 3] {
    // Zbuduj macierz kolumnami XYZ primaries, znormalizowaną tak, by biel dawała Y=1
    let rz = 1.0 - rx - ry; let gz = 1.0 - gx - gy; let bz = 1.0 - bx - by;
    let r = [rx/ry, 1.0, rz/ry];
    let g = [gx/gy, 1.0, gz/gy];
    let b = [bx/by, 1.0, bz/by];
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];

    // White point XYZ (Y=1)
    let wz = 1.0 - wx - wy;
    let w = [wx/wy, 1.0, wz/wy];

    // Solve M * s = w for s (scale factors)
    let s = solve3(m, w);
    let m_scaled = [
        [ (m[0][0]*s[0]) as f32, (m[0][1]*s[1]) as f32, (m[0][2]*s[2]) as f32 ],
        [ (m[1][0]*s[0]) as f32, (m[1][1]*s[1]) as f32, (m[1][2]*s[2]) as f32 ],
        [ (m[2][0]*s[0]) as f32, (m[2][1]*s[1]) as f32, (m[2][2]*s[2]) as f32 ],
    ];
    m_scaled
}

fn xyz_to_srgb_matrix() -> [[f32; 3]; 3] {
    // Stała macierz XYZ→sRGB (D65)
    [
        [ 3.2404542, -1.5371385, -0.4985314 ],
        [ -0.9692660, 1.8760108, 0.0415560 ],
        [ 0.0556434, -0.2040259, 1.0572252 ],
    ]
}

fn bradford_adaptation_matrix(src_xy: (f64, f64), dst_xy: (f64, f64)) -> [[f32;3];3] {
    // Bradford cone response matrix and its inverse
    let m = [
        [ 0.8951_f64,  0.2664, -0.1614],
        [-0.7502,      1.7135,  0.0367],
        [ 0.0389,     -0.0685,  1.0296],
    ];
    let m_inv = [
        [ 0.9869929, -0.1470543, 0.1599627],
        [ 0.4323053,  0.5183603, 0.0492912],
        [-0.0085287,  0.0400428, 0.9684867],
    ];

    let src_xyz = xy_to_xyz(src_xy.0, src_xy.1);
    let dst_xyz = xy_to_xyz(dst_xy.0, dst_xy.1);
    // Compute cone response for source and destination whites
    let src_lms = mul3x1(m, src_xyz);
    let dst_lms = mul3x1(m, dst_xyz);
    let scale = [dst_lms[0]/src_lms[0], dst_lms[1]/src_lms[1], dst_lms[2]/src_lms[2]];
    // Build adaptation matrix: M_inv * S * M
    let s = [
        [scale[0], 0.0, 0.0],
        [0.0, scale[1], 0.0],
        [0.0, 0.0, scale[2]],
    ];
    let ms = mul3x3_f64(s, m);
    let tmp = mul3x3_f64(m_inv, ms);
    // return as f32
    [
        [tmp[0][0] as f32, tmp[0][1] as f32, tmp[0][2] as f32],
        [tmp[1][0] as f32, tmp[1][1] as f32, tmp[1][2] as f32],
        [tmp[2][0] as f32, tmp[2][1] as f32, tmp[2][2] as f32],
    ]
}

fn xy_to_xyz(x: f64, y: f64) -> [f64;3] {
    let z = 1.0 - x - y;
    [x/y, 1.0, z/y]
}

fn mul3x3(a: [[f32;3];3], b: [[f32;3];3]) -> [[f32;3];3] {
    let mut m = [[0.0f32;3];3];
    for i in 0..3 { for j in 0..3 { m[i][j] = a[i][0]*b[0][j] + a[i][1]*b[1][j] + a[i][2]*b[2][j]; } }
    m
}

fn mul3x3_f64(a: [[f64;3];3], b: [[f64;3];3]) -> [[f64;3];3] {
    let mut m = [[0.0f64;3];3];
    for i in 0..3 { for j in 0..3 { m[i][j] = a[i][0]*b[0][j] + a[i][1]*b[1][j] + a[i][2]*b[2][j]; } }
    m
}

fn mul3x1(a: [[f64;3];3], v: [f64;3]) -> [f64;3] {
    [
        a[0][0]*v[0] + a[0][1]*v[1] + a[0][2]*v[2],
        a[1][0]*v[0] + a[1][1]*v[1] + a[1][2]*v[2],
        a[2][0]*v[0] + a[2][1]*v[1] + a[2][2]*v[2],
    ]
}

fn solve3(m: [[f64;3];3], w: [f64;3]) -> [f64;3] {
    // Proste rozwiązanie układu liniowego 3x3 metodą Cramera
    let det =
        m[0][0]*(m[1][1]*m[2][2]-m[1][2]*m[2][1]) -
        m[0][1]*(m[1][0]*m[2][2]-m[1][2]*m[2][0]) +
        m[0][2]*(m[1][0]*m[2][1]-m[1][1]*m[2][0]);
    if det.abs() < 1e-12 { return [1.0,1.0,1.0]; }
    let inv_det = 1.0/det;
    let inv = [
        [ (m[1][1]*m[2][2]-m[1][2]*m[2][1])*inv_det, (m[0][2]*m[2][1]-m[0][1]*m[2][2])*inv_det, (m[0][1]*m[1][2]-m[0][2]*m[1][1])*inv_det ],
        [ (m[1][2]*m[2][0]-m[1][0]*m[2][2])*inv_det, (m[0][0]*m[2][2]-m[0][2]*m[2][0])*inv_det, (m[0][2]*m[1][0]-m[0][0]*m[1][2])*inv_det ],
        [ (m[1][0]*m[2][1]-m[1][1]*m[2][0])*inv_det, (m[0][1]*m[2][0]-m[0][0]*m[2][1])*inv_det, (m[0][0]*m[1][1]-m[0][1]*m[1][0])*inv_det ],
    ];
    let s0 = inv[0][0]*w[0] + inv[0][1]*w[1] + inv[0][2]*w[2];
    let s1 = inv[1][0]*w[0] + inv[1][1]*w[1] + inv[1][2]*w[2];
    let s2 = inv[2][0]*w[0] + inv[2][1]*w[1] + inv[2][2]*w[2];
    [s0,s1,s2]
}