#![allow(dead_code)]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicI32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::sync::Arc;

/// Metryki wydajności GPU do monitorowania i optymalizacji
#[derive(Debug)]
pub struct GpuMetrics {
    /// Czasy wykonania ostatnich operacji (frame times)
    pub frame_times: Arc<Mutex<VecDeque<Duration>>>,
    /// Aktualne użycie pamięci GPU w bajtach
    pub memory_usage: AtomicU64,
    /// Liczba trafień w cache pipeline'ów
    pub pipeline_cache_hits: AtomicU64,
    /// Wykorzystanie buffer pool (0.0 - 1.0) - przechowywane jako i32 * 1000
    pub buffer_pool_utilization: AtomicI32,
    /// Liczba operacji GPU wykonanych w ostatnim czasie
    pub operations_count: AtomicU64,
    /// Średni czas operacji GPU
    pub average_operation_time: Arc<Mutex<Duration>>,
    /// Maksymalny czas operacji GPU
    pub max_operation_time: Arc<Mutex<Duration>>,
    /// Liczba błędów GPU
    #[allow(dead_code)]
    pub error_count: AtomicU64,
    /// Timestamp ostatniej aktualizacji metryk
    pub last_update: Arc<Mutex<Instant>>,
}

impl Default for GpuMetrics {
    fn default() -> Self {
        Self {
            frame_times: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
            memory_usage: AtomicU64::new(0),
            pipeline_cache_hits: AtomicU64::new(0),
            buffer_pool_utilization: AtomicI32::new(0), // 0 = 0.0%
            operations_count: AtomicU64::new(0),
            average_operation_time: Arc::new(Mutex::new(Duration::ZERO)),
            max_operation_time: Arc::new(Mutex::new(Duration::ZERO)),
            error_count: AtomicU64::new(0),
            last_update: Arc::new(Mutex::new(Instant::now())),
        }
    }
}

impl GpuMetrics {
    /// Tworzy nową instancję metryk GPU
    pub fn new() -> Self {
        Self::default()
    }

    /// Rejestruje czas wykonania operacji GPU
    pub fn record_operation_time(&self, duration: Duration) {
        // Aktualizuj frame times
        if let Ok(mut frame_times) = self.frame_times.lock() {
            frame_times.push_back(duration);
            
            // Zachowaj tylko ostatnie 100 pomiarów
            if frame_times.len() > 100 {
                frame_times.pop_front();
            }
        }

        // Aktualizuj licznik operacji
        self.operations_count.fetch_add(1, Ordering::Relaxed);

        // Aktualizuj średni czas operacji
        if let Ok(mut avg_time) = self.average_operation_time.lock() {
            let current_count = self.operations_count.load(Ordering::Relaxed);
            if current_count > 0 {
                let total_time = avg_time.mul_f32((current_count - 1) as f32) + duration;
                *avg_time = total_time.div_f32(current_count as f32);
            }
        }

        // Aktualizuj maksymalny czas operacji
        if let Ok(mut max_time) = self.max_operation_time.lock() {
            if duration > *max_time {
                *max_time = duration;
            }
        }

        // Aktualizuj timestamp
        if let Ok(mut last_update) = self.last_update.lock() {
            *last_update = Instant::now();
        }
    }

    /// Rejestruje trafienie w cache pipeline'ów
    pub fn record_pipeline_cache_hit(&self) {
        self.pipeline_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Aktualizuje użycie pamięci GPU
    pub fn update_memory_usage(&self, bytes: u64) {
        self.memory_usage.store(bytes, Ordering::Relaxed);
    }

    /// Aktualizuje wykorzystanie buffer pool
    pub fn update_buffer_pool_utilization(&self, utilization: f32) {
        let utilization_int = (utilization.clamp(0.0, 1.0) * 1000.0) as i32;
        self.buffer_pool_utilization.store(utilization_int, Ordering::Relaxed);
    }

    /// Rejestruje błąd GPU
    #[allow(dead_code)]
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Pobiera aktualne metryki jako string do logowania
    #[allow(dead_code)]
    pub fn get_metrics_summary(&self) -> String {
        let memory_mb = self.memory_usage.load(Ordering::Relaxed) as f64 / 1024.0 / 1024.0;
        let cache_hits = self.pipeline_cache_hits.load(Ordering::Relaxed);
        let operations = self.operations_count.load(Ordering::Relaxed);
        let errors = self.error_count.load(Ordering::Relaxed);
        let buffer_util = self.buffer_pool_utilization.load(Ordering::Relaxed) as f32 / 1000.0;
        
        let avg_time = if let Ok(avg_time) = self.average_operation_time.lock() {
            format!("{:.2}ms", avg_time.as_micros() as f64 / 1000.0)
        } else {
            "N/A".to_string()
        };

        let max_time = if let Ok(max_time) = self.max_operation_time.lock() {
            format!("{:.2}ms", max_time.as_micros() as f64 / 1000.0)
        } else {
            "N/A".to_string()
        };

        format!(
            "GPU Metrics: Memory: {:.1}MB, Cache Hits: {}, Operations: {}, Errors: {}, Buffer Util: {:.1}%, Avg Time: {}, Max Time: {}",
            memory_mb, cache_hits, operations, errors, buffer_util * 100.0, avg_time, max_time
        )
    }

    /// Pobiera średni czas operacji w mikrosekundach
    pub fn get_average_operation_time_us(&self) -> u64 {
        if let Ok(avg_time) = self.average_operation_time.lock() {
            avg_time.as_micros() as u64
        } else {
            0
        }
    }

    /// Pobiera liczbę operacji na sekundę
    pub fn get_operations_per_second(&self) -> f32 {
        let operations = self.operations_count.load(Ordering::Relaxed);
        if let Ok(last_update) = self.last_update.lock() {
            let elapsed = last_update.elapsed();
            if elapsed.as_secs() > 0 {
                operations as f32 / elapsed.as_secs() as f32
            } else {
                0.0
            }
        } else {
            0.0
        }
    }

    /// Resetuje wszystkie metryki
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.memory_usage.store(0, Ordering::Relaxed);
        self.pipeline_cache_hits.store(0, Ordering::Relaxed);
        self.buffer_pool_utilization.store(0, Ordering::Relaxed);
        self.operations_count.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        
        if let Ok(mut frame_times) = self.frame_times.lock() {
            frame_times.clear();
        }
        
        if let Ok(mut avg_time) = self.average_operation_time.lock() {
            *avg_time = Duration::ZERO;
        }
        
        if let Ok(mut max_time) = self.max_operation_time.lock() {
            *max_time = Duration::ZERO;
        }
        
        if let Ok(mut last_update) = self.last_update.lock() {
            *last_update = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_gpu_metrics_creation() {
        let metrics = GpuMetrics::new();
        assert_eq!(metrics.memory_usage.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.operations_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_operation_time_recording() {
        let metrics = GpuMetrics::new();
        
        // Symuluj operację GPU
        let operation_time = Duration::from_millis(50);
        metrics.record_operation_time(operation_time);
        
        assert_eq!(metrics.operations_count.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.get_average_operation_time_us(), 50000);
    }

    #[test]
    fn test_pipeline_cache_hit_recording() {
        let metrics = GpuMetrics::new();
        
        metrics.record_pipeline_cache_hit();
        metrics.record_pipeline_cache_hit();
        
        assert_eq!(metrics.pipeline_cache_hits.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_memory_usage_update() {
        let metrics = GpuMetrics::new();
        
        metrics.update_memory_usage(1024 * 1024); // 1MB
        
        assert_eq!(metrics.memory_usage.load(Ordering::Relaxed), 1024 * 1024);
    }

    #[test]
    fn test_buffer_pool_utilization_update() {
        let metrics = GpuMetrics::new();
        
        metrics.update_buffer_pool_utilization(0.75);
        
        assert_eq!(metrics.buffer_pool_utilization.load(Ordering::Relaxed), 0.75);
    }

    #[test]
    fn test_error_recording() {
        let metrics = GpuMetrics::new();
        
        metrics.record_error();
        metrics.record_error();
        
        assert_eq!(metrics.error_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = GpuMetrics::new();
        
        // Dodaj jakieś dane
        metrics.record_operation_time(Duration::from_millis(100));
        metrics.record_pipeline_cache_hit();
        metrics.update_memory_usage(1024);
        
        // Reset
        metrics.reset();
        
        assert_eq!(metrics.operations_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.pipeline_cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.memory_usage.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_operations_per_second() {
        let metrics = GpuMetrics::new();
        
        // Symuluj kilka operacji
        for _ in 0..5 {
            metrics.record_operation_time(Duration::from_millis(10));
        }
        
        // Poczekaj chwilę
        thread::sleep(Duration::from_millis(100));
        
        let ops_per_sec = metrics.get_operations_per_second();
        assert!(ops_per_sec > 0.0);
    }
}
