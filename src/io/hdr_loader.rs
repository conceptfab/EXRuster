use anyhow::{Context, Result};
use image::codecs::hdr::HdrDecoder;
use image::DynamicImage;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Decoded Radiance HDR image: interleaved RGBA f32 (alpha = 1.0).
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
}

pub fn load_hdr(path: &Path) -> Result<HdrImage> {
    let file = File::open(path)
        .with_context(|| format!("Cannot open HDR file: {}", path.display()))?;
    let decoder = HdrDecoder::new(BufReader::new(file))
        .with_context(|| format!("Failed to decode HDR: {}", path.display()))?;
    let dyn_img = DynamicImage::from_decoder(decoder)
        .with_context(|| format!("Failed to read HDR pixels: {}", path.display()))?;
    let rgb32 = dyn_img.into_rgb32f();
    let width = rgb32.width();
    let height = rgb32.height();

    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for px in rgb32.pixels() {
        rgba.push(px.0[0]);
        rgba.push(px.0[1]);
        rgba.push(px.0[2]);
        rgba.push(1.0);
    }
    Ok(HdrImage { width, height, rgba })
}
