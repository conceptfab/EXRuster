use slint::{Weak, Timer, TimerMode, ComponentHandle};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::io::image_cache::ImageCache;
use crate::ui::ui_handlers::{lock_or_recover, push_console, ConsoleModel};
use crate::AppWindow;

static LAST_PREVIEW_LOG: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Throttled update system for smooth parameter changes
pub struct ThrottledUpdate {
    _timer: Timer,
    pending_exposure: Arc<Mutex<Option<f32>>>,
    pending_gamma: Arc<Mutex<Option<f32>>>,
}

impl ThrottledUpdate {
    pub fn new<F>(mut callback: F) -> Self 
    where 
        F: FnMut(Option<f32>, Option<f32>) + 'static
    {
        let pending_exposure = Arc::new(Mutex::new(None));
        let pending_gamma = Arc::new(Mutex::new(None));
        
        let pending_exp_clone = pending_exposure.clone();
        let pending_gamma_clone = pending_gamma.clone();
        
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let exp = lock_or_recover(&pending_exp_clone).take();
            let gamma = lock_or_recover(&pending_gamma_clone).take();
            
            // Call callback even if only one parameter changed
            if exp.is_some() || gamma.is_some() {
                callback(exp, gamma);
            }
        });
        
        Self { _timer: timer, pending_exposure, pending_gamma }
    }
    
    pub fn update_exposure(&self, value: f32) {
        *lock_or_recover(&self.pending_exposure) = Some(value);
    }
    
    pub fn update_gamma(&self, value: f32) {
        *lock_or_recover(&self.pending_gamma) = Some(value);
    }
}

/// Enhanced function for handling exposure and gamma changes with throttling
pub fn handle_parameter_changed_throttled(
    ui_handle: Weak<AppWindow>,
    image_cache: Arc<Mutex<Option<ImageCache>>>,
    console: ConsoleModel,
    exposure: Option<f32>,
    gamma: Option<f32>,
) {
    if let Some(ui) = ui_handle.upgrade() {
        let cache_guard = lock_or_recover(&image_cache);
        if let Some(ref cache) = *cache_guard {
            // Get current values if not passed
            let final_exposure = exposure.unwrap_or_else(|| ui.get_exposure_value());
            let final_gamma = gamma.unwrap_or_else(|| ui.get_gamma_value());
            
            let tonemap_mode = ui.get_tonemap_mode() as i32;
            let image = update_preview_image(&ui, cache, final_exposure, final_gamma, tonemap_mode, &console);
            
            ui.set_exr_image(image);
            
            // Update status bar with changed parameter info
            if exposure.is_some() && gamma.is_some() {
                ui.set_status_text(format!("🔄 Exposure: {:.2} EV, Gamma: {:.2}", final_exposure, final_gamma).into());
            } else if exposure.is_some() {
                ui.set_status_text(format!("🔄 Exposure: {:.2} EV", final_exposure).into());
            } else if gamma.is_some() {
                ui.set_status_text(format!("🔄 Gamma: {:.2}", final_gamma).into());
            }
        }
    }
}

/// Updates preview image based on current UI parameters
pub fn update_preview_image(
    ui: &AppWindow,
    cache: &ImageCache,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
    console: &ConsoleModel,
) -> slint::Image {
    // Use thumbnail for real-time preview if image is large, but don't go below 1:1 relative to widget
    // Consider HiDPI and image-fit: contain (aspect fitting)
    let preview_w = ui.get_preview_area_width() as f32;
    let preview_h = ui.get_preview_area_height() as f32;
    let dpr = ui.window().scale_factor() as f32;
    let img_w = cache.width as f32;
    let img_h = cache.height as f32;
    let container_ratio = if preview_h > 0.0 { preview_w / preview_h } else { 1.0 };
    let image_ratio = if img_h > 0.0 { img_w / img_h } else { 1.0 };
    // Longer side of image after fitting to container (contain)
    let display_long_side_logical = if container_ratio > image_ratio { preview_h * image_ratio } else { preview_w };
    let target = (display_long_side_logical * dpr).round().max(1.0) as u32;
    
    let image = if cache.raw_pixels.len() > 2_000_000 {
        cache.process_to_thumbnail(exposure, gamma, tonemap_mode, target)
    } else {
        cache.process_to_image(exposure, gamma, tonemap_mode)
    };
    
    // Throttled log to console: at least 300ms interval, with DPI and fitting diagnostics
    let mut last = lock_or_recover(&LAST_PREVIEW_LOG);
    let now = Instant::now();
    if last.map(|t| now.duration_since(t).as_millis() >= 300).unwrap_or(true) {
        let display_w_logical = if container_ratio > image_ratio { preview_h * image_ratio } else { preview_w };
        let display_h_logical = if container_ratio > image_ratio { preview_h } else { preview_w / image_ratio };
        let win_w = ui.get_window_width() as u32;
        let win_h = ui.get_window_height() as u32;
        let win_w_px = (win_w as f32 * dpr).round() as u32;
        let win_h_px = (win_h as f32 * dpr).round() as u32;
        push_console(ui, console,
            format!("[preview] params: exp={:.2}, gamma={:.2} | window={}x{} (≈{}x{} px @{}x) | view={}x{} @{}x | img={}x{} | display≈{}x{} px target={} px",
                exposure, gamma,
                win_w, win_h, win_w_px, win_h_px, dpr,
                preview_w as u32, preview_h as u32, dpr,
                img_w as u32, img_h as u32,
                (display_w_logical * dpr).round() as u32,
                (display_h_logical * dpr).round() as u32,
                target));
        *last = Some(now);
    }
    
    image
}