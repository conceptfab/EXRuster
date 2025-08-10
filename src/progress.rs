use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Import komponentów Slint
use crate::AppWindow;

pub trait ProgressSink: Send + Sync {
    fn start_indeterminate(&self, message: Option<&str>);
    fn set(&self, progress_0_1: f32, message: Option<&str>);
    fn finish(&self, message: Option<&str>);
    fn reset(&self);
}

pub struct UiProgress {
    ui: slint::Weak<AppWindow>,
    last_update: Arc<Mutex<Instant>>, // throttling
    min_interval: Duration,
}

impl UiProgress {
    pub fn new(ui: slint::Weak<AppWindow>) -> Self {
        Self { ui, last_update: Arc::new(Mutex::new(Instant::now() - Duration::from_millis(100))), min_interval: Duration::from_millis(80) }
    }

    fn maybe_update<F: FnOnce(&AppWindow)>(&self, f: F) {
        if let Some(ui) = self.ui.upgrade() {
            let mut last = self.last_update.lock().unwrap();
            let now = Instant::now();
            if now.duration_since(*last) >= self.min_interval {
                f(&ui);
                *last = now;
            }
        }
    }
}

impl ProgressSink for UiProgress {
    fn start_indeterminate(&self, message: Option<&str>) {
        // Natychmiastowa aktualizacja (bez throttlingu), aby użytkownik zobaczył pasek od razu
        if let Some(ui) = self.ui.upgrade() {
            ui.set_progress_value(-1.0);
            if let Some(m) = message { ui.set_status_text(m.into()); }
            *self.last_update.lock().unwrap() = Instant::now();
        }
    }

    fn set(&self, progress_0_1: f32, message: Option<&str>) {
        let clamped = progress_0_1.clamp(0.0, 1.0);
        // Jeżeli to duży skok lub jest komunikat – aktualizuj natychmiast, inaczej throttling
        let force = message.is_some() || clamped >= 0.99 || clamped <= 0.01;
        if force {
            if let Some(ui) = self.ui.upgrade() {
                ui.set_progress_value(clamped);
                if let Some(m) = message { ui.set_status_text(m.into()); }
                *self.last_update.lock().unwrap() = Instant::now();
            }
        } else {
            self.maybe_update(|ui| {
                ui.set_progress_value(clamped);
                if let Some(m) = message { ui.set_status_text(m.into()); }
            });
        }
    }

    fn finish(&self, message: Option<&str>) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_progress_value(1.0);
            if let Some(m) = message { ui.set_status_text(m.into()); }
            // krótki reset po 400ms
            let weak = self.ui.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(400), move || {
                if let Some(ui2) = weak.upgrade() {
                    ui2.set_progress_value(0.0);
                }
            });
        }
    }

    fn reset(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_progress_value(0.0);
        }
    }
}

pub struct NoopProgress;
impl ProgressSink for NoopProgress {
    fn start_indeterminate(&self, _message: Option<&str>) {}
    fn set(&self, _progress_0_1: f32, _message: Option<&str>) {}
    fn finish(&self, _message: Option<&str>) {}
    fn reset(&self) {}
}
