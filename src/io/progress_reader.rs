//! Reader wrapper that reports progress based on bytes read vs total file size.

use crate::ui::progress::ProgressSink;
use std::io::{Read, Result};
use std::time::{Duration, Instant};

/// Wraps a `Read` and reports progress to a `ProgressSink` based on bytes read.
pub struct ProgressReader<R> {
    inner: R,
    bytes_read: u64,
    total: u64,
    progress: Option<std::sync::Arc<dyn ProgressSink>>,
    last_report: Instant,
    min_interval: Duration,
    last_frac: f32,
}

impl<R: Read> ProgressReader<R> {
    pub fn new(
        inner: R,
        total: u64,
        progress: Option<std::sync::Arc<dyn ProgressSink>>,
    ) -> Self {
        Self {
            inner,
            bytes_read: 0,
            total: total.max(1),
            progress,
            last_report: Instant::now(),
            min_interval: Duration::from_millis(50),
            last_frac: 0.0,
        }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read += n as u64;

        if let Some(ref p) = self.progress {
            let frac = (self.bytes_read as f32 / self.total as f32).min(1.0);
            let now = Instant::now();
            let should_report = now.duration_since(self.last_report) >= self.min_interval
                || (frac - self.last_frac) >= 0.01
                || frac >= 1.0;

            if should_report {
                let msg = format!("Reading EXR... {:.0}%", frac * 100.0);
                p.set(frac, Some(msg.as_str()));
                self.last_report = now;
                self.last_frac = frac;
            }
        }

        Ok(n)
    }
}
