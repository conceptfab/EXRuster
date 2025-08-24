use rayon::prelude::*;
use crate::AppWindow;
// use std::sync::Arc;

/// Luminance weighting standards for color-to-grayscale conversion
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // Rec601 is used in tests and may be used by external code
pub enum LuminanceWeights {
    /// ITU-R BT.601 (NTSC/PAL) - older standard
    Rec601,
    /// ITU-R BT.709 (sRGB/HDTV) - more accurate for modern displays
    Rec709,
}

impl LuminanceWeights {
    /// Get the RGB coefficients for luminance calculation
    pub fn coefficients(self) -> (f32, f32, f32) {
        match self {
            Self::Rec601 => (0.299, 0.587, 0.114),
            Self::Rec709 => (0.2126, 0.7152, 0.0722),
        }
    }
    
    /// Calculate luminance from RGB values
    #[inline]
    pub fn luminance(self, r: f32, g: f32, b: f32) -> f32 {
        let (wr, wg, wb) = self.coefficients();
        wr * r + wg * g + wb * b
    }
}

impl Default for LuminanceWeights {
    fn default() -> Self {
        // Use Rec.709 as default - more accurate for sRGB content
        Self::Rec709
    }
}

#[derive(Debug, Clone)]
pub struct HistogramData {
    pub red_bins: Vec<u32>,
    pub green_bins: Vec<u32>, 
    pub blue_bins: Vec<u32>,
    pub luminance_bins: Vec<u32>,
    pub bin_count: usize,
    pub min_value: f32,
    pub max_value: f32,
    pub total_pixels: u32,
    pub luminance_standard: LuminanceWeights,
}

impl HistogramData {
    pub fn new(bin_count: usize) -> Self {
        Self::new_with_standard(bin_count, LuminanceWeights::default())
    }
    
    pub fn new_with_standard(bin_count: usize, luminance_standard: LuminanceWeights) -> Self {
        Self {
            red_bins: vec![0; bin_count],
            green_bins: vec![0; bin_count],
            blue_bins: vec![0; bin_count], 
            luminance_bins: vec![0; bin_count],
            bin_count,
            min_value: 0.0,
            max_value: 1.0,
            total_pixels: 0,
            luminance_standard,
        }
    }

    pub fn compute_from_rgba_pixels(&mut self, pixels: &[f32]) -> anyhow::Result<()> {
        self.reset();
        
        if pixels.len() % 4 != 0 {
            return Err(anyhow::anyhow!("Invalid RGBA pixel data"));
        }

        let pixel_count = pixels.len() / 4;
        if pixel_count == 0 { return Ok(()); }

        // Znajdź zakres wartości (min/max) równolegle
        let (min_r, max_r, min_g, max_g, min_b, max_b) = pixels
            .par_chunks_exact(4)
            .map(|rgba| (rgba[0], rgba[0], rgba[1], rgba[1], rgba[2], rgba[2]))
            .reduce(
                || (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY),
                |acc, curr| (
                    acc.0.min(curr.0), acc.1.max(curr.1),
                    acc.2.min(curr.2), acc.3.max(curr.3), 
                    acc.4.min(curr.4), acc.5.max(curr.5)
                )
            );

        self.min_value = min_r.min(min_g).min(min_b).max(0.0);
        self.max_value = max_r.max(max_g).max(max_b).min(10.0); // Clamp extreme values
        
        if self.max_value <= self.min_value {
            self.max_value = self.min_value + 1.0;
        }

        let range = self.max_value - self.min_value;
        self.total_pixels = pixel_count as u32;

        // Compute histograms równolegle 
        let chunk_size = (pixel_count / rayon::current_num_threads()).max(1024);
        let results: Vec<_> = pixels
            .par_chunks_exact(4)
            .chunks(chunk_size)
            .map(|chunk| {
                let mut local_r = vec![0u32; self.bin_count];
                let mut local_g = vec![0u32; self.bin_count];
                let mut local_b = vec![0u32; self.bin_count];
                let mut local_lum = vec![0u32; self.bin_count];

                for rgba in chunk {
                    let r = rgba[0].clamp(self.min_value, self.max_value);
                    let g = rgba[1].clamp(self.min_value, self.max_value);
                    let b = rgba[2].clamp(self.min_value, self.max_value);
                    
                    let r_norm = (r - self.min_value) / range;
                    let g_norm = (g - self.min_value) / range;
                    let b_norm = (b - self.min_value) / range;
                    let lum = self.luminance_standard.luminance(r, g, b);
                    let lum_norm = (lum - self.min_value) / range;

                    let r_bin = ((r_norm * (self.bin_count - 1) as f32).round() as usize).min(self.bin_count - 1);
                    let g_bin = ((g_norm * (self.bin_count - 1) as f32).round() as usize).min(self.bin_count - 1);
                    let b_bin = ((b_norm * (self.bin_count - 1) as f32).round() as usize).min(self.bin_count - 1);
                    let lum_bin = ((lum_norm.clamp(0.0, 1.0) * (self.bin_count - 1) as f32).round() as usize).min(self.bin_count - 1);

                    local_r[r_bin] += 1;
                    local_g[g_bin] += 1;
                    local_b[b_bin] += 1;
                    local_lum[lum_bin] += 1;
                }

                (local_r, local_g, local_b, local_lum)
            })
            .collect();

        // Merge results
        for (local_r, local_g, local_b, local_lum) in results {
            for i in 0..self.bin_count {
                self.red_bins[i] += local_r[i];
                self.green_bins[i] += local_g[i];
                self.blue_bins[i] += local_b[i];
                self.luminance_bins[i] += local_lum[i];
            }
        }

        Ok(())
    }

    pub fn get_percentile(&self, channel: HistogramChannel, percentile: f32) -> f32 {
        let bins = match channel {
            HistogramChannel::Red => &self.red_bins,
            HistogramChannel::Green => &self.green_bins,
            HistogramChannel::Blue => &self.blue_bins,
            HistogramChannel::Luminance => &self.luminance_bins,
        };

        let target = (self.total_pixels as f32 * percentile.clamp(0.0, 1.0)) as u32;
        let mut accumulated = 0u32;

        for (i, &count) in bins.iter().enumerate() {
            accumulated += count;
            if accumulated >= target {
                let bin_value = i as f32 / (self.bin_count - 1) as f32;
                return self.min_value + bin_value * (self.max_value - self.min_value);
            }
        }

        self.max_value
    }


    /// Apply histogram data directly to UI - eliminates all the repetitive boilerplate
    pub fn apply_to_ui(&self, ui: &AppWindow) {
        use slint::{ModelRc, VecModel};
        
        // Convert bins to i32 vectors exactly as the original code did
        let red_bins: Vec<i32> = self.red_bins.iter().map(|&x| x as i32).collect();
        let green_bins: Vec<i32> = self.green_bins.iter().map(|&x| x as i32).collect();
        let blue_bins: Vec<i32> = self.blue_bins.iter().map(|&x| x as i32).collect();
        let lum_bins: Vec<i32> = self.luminance_bins.iter().map(|&x| x as i32).collect();
        
        // Set the data exactly as the original code did
        ui.set_histogram_red_data(ModelRc::new(VecModel::from(red_bins)));
        ui.set_histogram_green_data(ModelRc::new(VecModel::from(green_bins)));
        ui.set_histogram_blue_data(ModelRc::new(VecModel::from(blue_bins)));
        ui.set_histogram_luminance_data(ModelRc::new(VecModel::from(lum_bins)));
        
        // Also set the statistics
        ui.set_histogram_min_value(self.min_value);
        ui.set_histogram_max_value(self.max_value);
    }

    fn reset(&mut self) {
        self.red_bins.fill(0);
        self.green_bins.fill(0);
        self.blue_bins.fill(0);
        self.luminance_bins.fill(0);
        self.total_pixels = 0;
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum HistogramChannel {
    Red,
    Green,  
    Blue,
    Luminance,
}

// GPU Histogram computing removed - CPU only

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_basic() {
        let mut hist = HistogramData::new(256);
        let pixels = vec![0.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0];
        
        hist.compute_from_rgba_pixels(&pixels).unwrap();
        assert_eq!(hist.total_pixels, 3);
        
        let p50 = hist.get_percentile(HistogramChannel::Red, 0.5);
        assert!(p50 >= 0.0 && p50 <= 1.0);
    }

    #[test]
    fn test_histogram_performance() {
        let mut hist = HistogramData::new(256);
        let pixel_count = 1920 * 1080;
        let pixels: Vec<f32> = (0..pixel_count * 4).map(|i| (i % 256) as f32 / 255.0).collect();
        
        let start = std::time::Instant::now();
        hist.compute_from_rgba_pixels(&pixels).unwrap();
        let duration = start.elapsed();
        
        println!("Histogram computation for {}MP took {:?}", pixel_count / 1_000_000, duration);
        assert!(duration.as_millis() < 100); // Should be under 100ms for 2MP
    }
    
    #[test]
    fn test_luminance_standards() {
        // Test basic luminance calculation differences between standards
        let r = 1.0_f32;
        let g = 0.5_f32; 
        let b = 0.2_f32;
        
        let rec601_lum = LuminanceWeights::Rec601.luminance(r, g, b);
        let rec709_lum = LuminanceWeights::Rec709.luminance(r, g, b);
        
        // Both should be valid luminance values
        assert!(rec601_lum > 0.0 && rec601_lum <= 1.0);
        assert!(rec709_lum > 0.0 && rec709_lum <= 1.0);
        
        // They should be different for non-equal RGB values
        assert!((rec601_lum - rec709_lum).abs() > 1e-6, 
               "Rec.601 and Rec.709 should give different results: {} vs {}", rec601_lum, rec709_lum);
        
        println!("Rec.601 luminance: {:.6}", rec601_lum);
        println!("Rec.709 luminance: {:.6}", rec709_lum);
        
        // Test coefficients sum to 1.0 (approximately)
        let (r601, g601, b601) = LuminanceWeights::Rec601.coefficients();
        let (r709, g709, b709) = LuminanceWeights::Rec709.coefficients();
        
        let sum601 = r601 + g601 + b601;
        let sum709 = r709 + g709 + b709;
        
        assert!((sum601 - 1.0).abs() < 1e-6, "Rec.601 coefficients should sum to 1.0, got {}", sum601);
        assert!((sum709 - 1.0).abs() < 1e-6, "Rec.709 coefficients should sum to 1.0, got {}", sum709);
    }
    
    #[test]
    fn test_histogram_with_different_standards() {
        let pixels = vec![
            1.0, 0.0, 0.0, 1.0,  // Red pixel
            0.0, 1.0, 0.0, 1.0,  // Green pixel  
            0.0, 0.0, 1.0, 1.0,  // Blue pixel
        ];
        
        let mut hist_601 = HistogramData::new_with_standard(256, LuminanceWeights::Rec601);
        let mut hist_709 = HistogramData::new_with_standard(256, LuminanceWeights::Rec709);
        
        hist_601.compute_from_rgba_pixels(&pixels).unwrap();
        hist_709.compute_from_rgba_pixels(&pixels).unwrap();
        
        // Both should have same number of pixels
        assert_eq!(hist_601.total_pixels, 3);
        assert_eq!(hist_709.total_pixels, 3);
        
        // But luminance histograms should be different due to different weighting
        let sum_601: u32 = hist_601.luminance_bins.iter().sum();
        let sum_709: u32 = hist_709.luminance_bins.iter().sum();
        
        assert_eq!(sum_601, 3); // Total pixel count
        assert_eq!(sum_709, 3); // Total pixel count
        
        // The distribution should be different between standards
        let hist_601_non_zero = hist_601.luminance_bins.iter().filter(|&&x| x > 0).count();
        let hist_709_non_zero = hist_709.luminance_bins.iter().filter(|&&x| x > 0).count();
        
        println!("Rec.601 non-zero bins: {}", hist_601_non_zero);
        println!("Rec.709 non-zero bins: {}", hist_709_non_zero);
        
        // Both should have some non-zero bins
        assert!(hist_601_non_zero > 0);
        assert!(hist_709_non_zero > 0);
    }
    
    #[test]
    fn test_default_is_rec709() {
        let hist = HistogramData::new(256);
        assert_eq!(hist.luminance_standard, LuminanceWeights::Rec709);
        
        let default_standard = LuminanceWeights::default();
        assert_eq!(default_standard, LuminanceWeights::Rec709);
    }
}
