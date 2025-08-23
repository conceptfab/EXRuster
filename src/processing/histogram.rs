use rayon::prelude::*;
use crate::AppWindow;
// use std::sync::Arc;

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
}

impl HistogramData {
    pub fn new(bin_count: usize) -> Self {
        Self {
            red_bins: vec![0; bin_count],
            green_bins: vec![0; bin_count],
            blue_bins: vec![0; bin_count], 
            luminance_bins: vec![0; bin_count],
            bin_count,
            min_value: 0.0,
            max_value: 1.0,
            total_pixels: 0,
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
                    let lum_norm = (0.299 * r + 0.587 * g + 0.114 * b - self.min_value) / range;

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
}
