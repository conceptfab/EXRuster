use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use slint::invoke_from_event_loop;

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

    fn do_update(&self, progress: f32, message: Option<String>) {
        let ui_weak = self.ui.clone();
        let _ = invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_progress_value(progress);
                if let Some(m) = message {
                    ui.set_status_text(m.into());
                }
            }
        });
    }

    fn maybe_update(&self, progress: f32, message: Option<String>) {
        let mut last = self.last_update.lock().unwrap();
        let now = Instant::now();
        if now.duration_since(*last) >= self.min_interval {
            self.do_update(progress, message);
            *last = now;
        }
    }
}

impl ProgressSink for UiProgress {
    fn start_indeterminate(&self, message: Option<&str>) {
        // Natychmiastowa aktualizacja (bez throttlingu), aby użytkownik zobaczył pasek od razu
        self.do_update(-1.0, message.map(|s| s.to_string()));
        *self.last_update.lock().unwrap() = Instant::now();
    }

    fn set(&self, progress_0_1: f32, message: Option<&str>) {
        let clamped = progress_0_1.clamp(0.0, 1.0);
        let msg = message.map(|s| s.to_string());
        // Jeżeli to duży skok lub jest komunikat – aktualizuj natychmiast, inaczej throttling
        let force = msg.is_some() || clamped >= 0.99 || clamped <= 0.01;
        if force {
            self.do_update(clamped, msg.clone());
            *self.last_update.lock().unwrap() = Instant::now();
        } else {
            self.maybe_update(clamped, msg);
        }
    }

    fn finish(&self, message: Option<&str>) {
        let msg = message.map(|s| s.to_string());
        self.do_update(1.0, msg);
        // krótki reset po 400ms
        let weak = self.ui.clone();
        let _ = invoke_from_event_loop(move || {
            slint::Timer::single_shot(std::time::Duration::from_millis(400), move || {
                if let Some(ui2) = weak.upgrade() {
                    ui2.set_progress_value(0.0);
                }
            });
        });
    }

    fn reset(&self) {
        self.do_update(0.0, None);
    }
}
