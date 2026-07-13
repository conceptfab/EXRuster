use crate::ui::state::SharedAppState;
use crate::ui::ui_handlers::{lock_or_recover, ConsoleModel};
use crate::AppWindow;
use slint::{ComponentHandle, Timer, TimerMode, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Monotonic render generation. Every render request takes the next number;
/// the event loop accepts a finished frame only if it is still the newest one.
/// Without this, a slow full-res render could land after a newer, faster one.
static RENDER_GENERATION: AtomicU64 = AtomicU64::new(0);

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

/// Debounced geometry change handler — delays re-render until resize settles (200ms).
/// Avoids full process_to_image on every resize tick.
pub struct DebouncedGeometry {
    timer: Timer,
}

impl DebouncedGeometry {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn() + 'static,
    {
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, Duration::from_millis(200), callback);
        timer.stop(); // Don't fire until first trigger()
        Self { timer }
    }

    pub fn trigger(&self) {
        self.timer.restart();
    }
}

/// Enhanced function for handling exposure and gamma changes with throttling
pub fn handle_parameter_changed_throttled(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    _console: ConsoleModel,
    exposure: Option<f32>,
    gamma: Option<f32>,
) {
    let Some(ui) = ui_handle.upgrade() else {
        return;
    };

    let final_exposure = exposure.unwrap_or_else(|| ui.get_exposure_value());
    let final_gamma = gamma.unwrap_or_else(|| ui.get_gamma_value());
    let tonemap_mode = ui.get_tonemap_mode();

    spawn_preview_render(
        ui.as_weak(),
        app_state,
        final_exposure,
        final_gamma,
        tonemap_mode,
    );

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

/// Longer side of the image as actually displayed (contain-fit, DPI-aware).
fn compute_display_target(ui: &AppWindow, img_w: u32, img_h: u32) -> u32 {
    let preview_w = ui.get_preview_area_width();
    let preview_h = ui.get_preview_area_height();
    let dpr = ui.window().scale_factor();

    let container_ratio = if preview_h > 0.0 {
        preview_w / preview_h
    } else {
        1.0
    };
    let image_ratio = if img_h > 0 {
        img_w as f32 / img_h as f32
    } else {
        1.0
    };
    let display_long_side_logical = if container_ratio > image_ratio {
        preview_h * image_ratio
    } else {
        preview_w
    };
    (display_long_side_logical * dpr).round().max(1.0) as u32
}

/// Stride-decimate RGBA pixels down to roughly display size.
///
/// Nearest-neighbour on purpose: this feeds the throw-away interactive frame
/// during a slider drag, and the full-resolution pass replaces it milliseconds
/// later. Anything fancier would cost more than the frame is worth. Returns the
/// input untouched when the image is already near display size.
fn decimate_for_display(
    pixels: &Arc<Vec<f32>>,
    width: u32,
    height: u32,
    target_long_side: u32,
) -> (Arc<Vec<f32>>, u32, u32) {
    let long_side = width.max(height);
    if target_long_side == 0 || long_side <= target_long_side.saturating_mul(3) / 2 {
        return (Arc::clone(pixels), width, height);
    }

    let step = (long_side / target_long_side).max(1) as usize;
    let (w, h) = (width as usize, height as usize);
    let new_w = w.div_ceil(step);
    let new_h = h.div_ceil(step);

    let mut out = Vec::with_capacity(new_w * new_h * 4);
    for y in (0..h).step_by(step) {
        let row = y * w;
        for x in (0..w).step_by(step) {
            let i = (row + x) * 4;
            out.extend_from_slice(&pixels[i..i + 4]);
        }
    }
    (Arc::new(out), new_w as u32, new_h as u32)
}

/// Render the preview on the rayon pool instead of the UI thread.
///
/// Two passes: a decimated frame for instant feedback, then full resolution —
/// but only if no newer request superseded this one in the meantime. The event
/// loop does nothing but wrap the finished buffer in a `slint::Image`.
pub fn spawn_preview_render(
    ui_handle: Weak<AppWindow>,
    app_state: SharedAppState,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) {
    let (snapshot, target) = {
        let Some(ui) = ui_handle.upgrade() else {
            return;
        };
        let Ok(state) = app_state.read() else {
            return;
        };
        let Some(cache) = state.image_cache.as_ref() else {
            return; // nothing loaded yet
        };
        (
            cache.snapshot(),
            compute_display_target(&ui, cache.width, cache.height),
        )
    };

    let generation = RENDER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    rayon::spawn(move || {
        let post_frame = |buffer: slint::SharedPixelBuffer<slint::Rgba8Pixel>| {
            let ui_handle = ui_handle.clone();
            let _ = slint::invoke_from_event_loop(move || {
                // Drop the frame if a newer render has since been requested.
                if RENDER_GENERATION.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Some(ui) = ui_handle.upgrade() {
                    ui.set_exr_image(slint::Image::from_rgba8(buffer));
                }
            });
        };

        let (small, sw, sh) =
            decimate_for_display(&snapshot.pixels, snapshot.width, snapshot.height, target);
        let was_decimated = sw != snapshot.width || sh != snapshot.height;

        post_frame(crate::io::image_cache::render_to_buffer(
            &small,
            sw,
            sh,
            exposure,
            gamma,
            tonemap_mode,
            snapshot.color_matrix,
        ));

        // Full-res pass, unless we were already superseded.
        if was_decimated && RENDER_GENERATION.load(Ordering::SeqCst) == generation {
            post_frame(crate::io::image_cache::render_to_buffer(
                &snapshot.pixels,
                snapshot.width,
                snapshot.height,
                exposure,
                gamma,
                tonemap_mode,
                snapshot.color_matrix,
            ));
        }
    });
}
