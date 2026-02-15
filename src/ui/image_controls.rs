use crate::io::image_cache::ImageCache;
use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{lock_or_recover, push_console, ConsoleModel};
use crate::AppWindow;
use slint::{ComponentHandle, Timer, TimerMode, Weak};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static LAST_PREVIEW_LOG: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Throttled update system for smooth parameter changes.
/// Uses SingleShot timer to avoid polling when UI is idle.
pub struct ThrottledUpdate {
    timer: Timer,
    pending_exposure: Arc<Mutex<Option<f32>>>,
    pending_gamma: Arc<Mutex<Option<f32>>>,
}

impl ThrottledUpdate {
    pub fn new<F>(mut callback: F) -> Self
    where
        F: FnMut(Option<f32>, Option<f32>) + 'static,
    {
        let pending_exposure = Arc::new(Mutex::new(None));
        let pending_gamma = Arc::new(Mutex::new(None));

        let pending_exp_clone = pending_exposure.clone();
        let pending_gamma_clone = pending_gamma.clone();

        let timer = Timer::default();
        // Deferred start — timer only starts on first update_exposure/update_gamma call
        let callback = std::cell::RefCell::new(Some(move || {
            let exp = lock_or_recover(&pending_exp_clone).take();
            let gamma = lock_or_recover(&pending_gamma_clone).take();
            if exp.is_some() || gamma.is_some() {
                callback(exp, gamma);
            }
        }));

        // Use a single-shot timer that only fires when restarted by update methods
        if let Some(cb) = callback.borrow_mut().take() {
            timer.start(TimerMode::SingleShot, Duration::from_millis(16), cb);
            timer.stop(); // Don't fire immediately — wait for first restart()
        }

        Self {
            timer,
            pending_exposure,
            pending_gamma,
        }
    }

    pub fn update_exposure(&self, value: f32) {
        *lock_or_recover(&self.pending_exposure) = Some(value);
        self.timer.restart();
    }

    pub fn update_gamma(&self, value: f32) {
        *lock_or_recover(&self.pending_gamma) = Some(value);
        self.timer.restart();
    }
}

/// Enhanced function for handling exposure and gamma changes with throttling
pub fn handle_parameter_changed_throttled(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    console: ConsoleModel,
    exposure: Option<f32>,
    gamma: Option<f32>,
) {
    if let Some(ui) = ui_handle.upgrade() {
        if let Ok(state) = app_state.read() {
            if let Some(ref cache) = state.image_cache {
                // Get current values if not passed
                let final_exposure = exposure.unwrap_or_else(|| ui.get_exposure_value());
                let final_gamma = gamma.unwrap_or_else(|| ui.get_gamma_value());

                let tonemap_mode = ui.get_tonemap_mode() as i32;
                let image = update_preview_image(
                    &ui,
                    cache,
                    final_exposure,
                    final_gamma,
                    tonemap_mode,
                    &console,
                );

                ui.set_exr_image(image);

                // Update status bar with changed parameter info
                if exposure.is_some() && gamma.is_some() {
                    ui.set_status_text(
                        format!(
                            "🔄 Exposure: {:.2} EV, Gamma: {:.2}",
                            final_exposure, final_gamma
                        )
                        .into(),
                    );
                } else if exposure.is_some() {
                    ui.set_status_text(format!("🔄 Exposure: {:.2} EV", final_exposure).into());
                } else if gamma.is_some() {
                    ui.set_status_text(format!("🔄 Gamma: {:.2}", final_gamma).into());
                }
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
    let container_ratio = if preview_h > 0.0 {
        preview_w / preview_h
    } else {
        1.0
    };
    let image_ratio = if img_h > 0.0 { img_w / img_h } else { 1.0 };
    // Longer side of image after fitting to container (contain)
    let display_long_side_logical = if container_ratio > image_ratio {
        preview_h * image_ratio
    } else {
        preview_w
    };
    let target = (display_long_side_logical * dpr).round().max(1.0) as u32;

    let image = cache.process_to_image(exposure, gamma, tonemap_mode);

    // Throttled log to console: at least 300ms interval
    let mut last = lock_or_recover(&LAST_PREVIEW_LOG);
    let now = Instant::now();
    if last
        .map(|t| now.duration_since(t).as_millis() >= 300)
        .unwrap_or(true)
    {
        push_console(ui, console,
            format!("[preview] exp={:.2}, gamma={:.2} | img={}x{} | view={}x{} @{:.1}x | target={} px",
                exposure, gamma,
                img_w as u32, img_h as u32,
                preview_w as u32, preview_h as u32, dpr,
                target));
        *last = Some(now);
    }

    image
}
