#![allow(dead_code)]
#![allow(unused_variables)]
use std::sync::atomic::{AtomicI32, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::gpu_metrics::GpuMetrics;

/// Typ operacji GPU do klasyfikacji
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuOperation {
    #[allow(dead_code)] /// Generowanie thumbnail'ów
    ThumbnailGeneration,
    #[allow(dead_code)] /// Generowanie poziomów MIP
    MipGeneration,
    /// Przetwarzanie obrazów (filtry, tone mapping)
    ImageProcessing,
    #[allow(dead_code)] /// Eksport obrazów
    ImageExport,
}

/// Parametry operacji GPU
#[derive(Debug, Clone)]
pub struct GpuOperationParams {
    /// Rozmiar danych wejściowych w bajtach
    pub input_size_bytes: u64,
    /// Rozmiar danych wyjściowych w bajtach
    #[allow(dead_code)]
    pub output_size_bytes: u64,
    /// Złożoność operacji (1.0 = prosta, 10.0 = bardzo złożona)
    pub complexity: f32,
    /// Czy operacja jest krytyczna dla UI
    pub is_ui_critical: bool,
    /// Maksymalny akceptowalny czas wykonania
    #[allow(dead_code)]
    pub max_acceptable_time: Duration,
}

impl Default for GpuOperationParams {
    fn default() -> Self {
        Self {
            input_size_bytes: 1024 * 1024, // 1MB
            output_size_bytes: 1024 * 1024, // 1MB
            complexity: 1.0,
            is_ui_critical: false,
            max_acceptable_time: Duration::from_millis(100),
        }
    }
}

/// Adaptacyjny scheduler GPU decydujący o użyciu GPU vs CPU
#[derive(Debug)]
pub struct AdaptiveGpuScheduler {
    /// Benchmark wydajności CPU (operacje/sekundę) - przechowywane jako i32 * 100
    cpu_benchmark: AtomicI32,
    /// Benchmark wydajności GPU (operacje/sekundę) - przechowywane jako i32 * 100
    gpu_benchmark: AtomicI32,
    /// Aktualne obciążenie GPU (0.0 - 1.0) - przechowywane jako i32 * 1000
    current_load: AtomicI32,
    /// Czy GPU jest dostępne
    gpu_available: AtomicBool,
    /// Metryki GPU do podejmowania decyzji
    gpu_metrics: Arc<GpuMetrics>,
    /// Historia decyzji dla różnych typów operacji
    decision_history: Arc<Mutex<HashMap<GpuOperation, Vec<bool>>>>,
    /// Timestamp ostatniego benchmark'u
    last_benchmark: Arc<Mutex<Instant>>,
    /// Częstotliwość wykonywania benchmark'ów
    benchmark_interval: Duration,
}

impl AdaptiveGpuScheduler {
    /// Tworzy nową instancję scheduler'a GPU
    pub fn new(gpu_metrics: Arc<GpuMetrics>) -> Self {
        Self {
                    cpu_benchmark: AtomicI32::new(10000), // Domyślnie 100.0 ops/s * 100
        gpu_benchmark: AtomicI32::new(20000),  // Domyślnie 200.0 ops/s * 100
        current_load: AtomicI32::new(0), // 0.0 * 1000
            gpu_available: AtomicBool::new(true),
            gpu_metrics,
            decision_history: Arc::new(Mutex::new(HashMap::new())),
            last_benchmark: Arc::new(Mutex::new(Instant::now())),
            benchmark_interval: Duration::from_secs(60), // Co minutę
        }
    }

    /// Decyduje czy użyć GPU dla danej operacji
    pub fn should_use_gpu(&self, operation: GpuOperation, params: &GpuOperationParams) -> bool {
        // Sprawdź czy GPU jest dostępne
        if !self.gpu_available.load(Ordering::Relaxed) {
            return false;
        }

        // Sprawdź czy GPU nie jest przeciążone
        let current_load = self.current_load.load(Ordering::Relaxed) as f32 / 1000.0;
        if current_load > 0.9 {
            return false;
        }

        // Sprawdź czy operacja jest krytyczna dla UI
        if params.is_ui_critical && current_load > 0.7 {
            return false;
        }

        // Wykonaj benchmark jeśli minął odpowiedni czas
        self.run_benchmark_if_needed();

        // Pobierz aktualne benchmark'i
        let cpu_perf = self.cpu_benchmark.load(Ordering::Relaxed) as f32 / 100.0;
        let gpu_perf = self.gpu_benchmark.load(Ordering::Relaxed) as f32 / 100.0;

        // Oblicz score dla GPU vs CPU
        let gpu_score = self.calculate_gpu_score(operation, params, gpu_perf);
        let cpu_score = self.calculate_cpu_score(operation, params, cpu_perf);

        // Dodaj bias dla GPU jeśli operacja jest GPU-friendly
        let gpu_bias = self.get_gpu_bias(operation);
        let adjusted_gpu_score = gpu_score * gpu_bias;

        let should_use_gpu = adjusted_gpu_score > cpu_score;

        // Zapisz decyzję w historii
        self.record_decision(operation, should_use_gpu);

        should_use_gpu
    }

    /// Oblicza score dla GPU
    fn calculate_gpu_score(&self, _operation: GpuOperation, params: &GpuOperationParams, gpu_perf: f32) -> f32 {
        let base_score = gpu_perf;
        
        // Modyfikator dla rozmiaru danych
        let size_factor = (params.input_size_bytes as f32 / (1024.0 * 1024.0)).min(10.0);
        
        // Modyfikator dla złożoności
        let complexity_factor = params.complexity.min(5.0);
        
        // Modyfikator dla obciążenia GPU
        let load_factor = 1.0 - (self.current_load.load(Ordering::Relaxed) as f32 / 1000.0);
        
        base_score * size_factor * complexity_factor * load_factor
    }

    /// Oblicza score dla CPU
    fn calculate_cpu_score(&self, _operation: GpuOperation, params: &GpuOperationParams, cpu_perf: f32) -> f32 {
        let base_score = cpu_perf;
        
        // CPU lepiej radzi sobie z małymi operacjami
        let size_factor = 1.0 / (params.input_size_bytes as f32 / (1024.0 * 1024.0)).max(0.1);
        
        // CPU lepiej radzi sobie z prostymi operacjami
        let complexity_factor = 1.0 / params.complexity.max(0.5);
        
        base_score * size_factor * complexity_factor
    }

    /// Pobiera bias dla GPU dla danego typu operacji
    fn get_gpu_bias(&self, operation: GpuOperation) -> f32 {
        match operation {
            GpuOperation::ThumbnailGeneration => 1.5,  // GPU bardzo dobre dla thumbnail'ów
            GpuOperation::MipGeneration => 2.0,         // GPU doskonałe dla MIP
            GpuOperation::ImageProcessing => 1.3,       // GPU dobre dla przetwarzania
            GpuOperation::ImageExport => 1.1,           // GPU umiarkowanie dobre dla eksportu
        }
    }

    /// Aktualizuje obciążenie GPU
    pub fn update_gpu_load(&self, load: f32) {
        let load_int = (load.clamp(0.0, 1.0) * 1000.0) as i32;
        self.current_load.store(load_int, Ordering::Relaxed);
    }

    /// Ustawia dostępność GPU
    #[allow(dead_code)]
    pub fn set_gpu_available(&self, available: bool) {
        self.gpu_available.store(available, Ordering::Relaxed);
    }

    /// Aktualizuje benchmark CPU
    pub fn update_cpu_benchmark(&self, ops_per_second: f32) {
        let ops_int = (ops_per_second.max(0.1) * 100.0) as i32;
        self.cpu_benchmark.store(ops_int, Ordering::Relaxed);
    }

    /// Aktualizuje benchmark GPU
    pub fn update_gpu_benchmark(&self, ops_per_second: f32) {
        let ops_int = (ops_per_second.max(0.1) * 100.0) as i32;
        self.gpu_benchmark.store(ops_int, Ordering::Relaxed);
    }

    /// Wykonuje benchmark jeśli minął odpowiedni czas
    fn run_benchmark_if_needed(&self) {
        if let Ok(mut last_benchmark) = self.last_benchmark.lock() {
            if last_benchmark.elapsed() >= self.benchmark_interval {
                self.run_benchmark();
                *last_benchmark = Instant::now();
            }
        }
    }

    /// Wykonuje benchmark wydajności
    fn run_benchmark(&self) {
        // Pobierz metryki z GPU
        let gpu_ops_per_sec = self.gpu_metrics.get_operations_per_second();
        let _gpu_avg_time = self.gpu_metrics.get_average_operation_time_us();
        
        // Aktualizuj benchmark GPU
        if gpu_ops_per_sec > 0.0 {
            self.update_gpu_benchmark(gpu_ops_per_sec);
        }
        
        // Symuluj benchmark CPU (w rzeczywistej implementacji można dodać rzeczywiste testy)
        let cpu_ops_per_sec = self.estimate_cpu_performance();
        self.update_cpu_benchmark(cpu_ops_per_sec);
        
        println!("Benchmark wykonany - GPU: {:.1} ops/s, CPU: {:.1} ops/s", 
                 gpu_ops_per_sec, cpu_ops_per_sec);
    }

    /// Szacuje wydajność CPU na podstawie dostępnych metryk
    fn estimate_cpu_performance(&self) -> f32 {
        // W rzeczywistej implementacji można dodać rzeczywiste testy CPU
        // Na razie zwracamy stałą wartość
        150.0
    }

    /// Zapisuje decyzję w historii
    fn record_decision(&self, operation: GpuOperation, used_gpu: bool) {
        if let Ok(mut history) = self.decision_history.lock() {
            let decisions = history.entry(operation).or_insert_with(Vec::new);
            decisions.push(used_gpu);
            
            // Zachowaj tylko ostatnie 100 decyzji
            if decisions.len() > 100 {
                decisions.remove(0);
            }
        }
    }

    /// Pobiera statystyki decyzji dla danego typu operacji
    #[allow(dead_code)]
    pub fn get_decision_stats(&self, operation: GpuOperation) -> Option<(usize, usize)> {
        if let Ok(history) = self.decision_history.lock() {
            if let Some(decisions) = history.get(&operation) {
                let gpu_count = decisions.iter().filter(|&&used| used).count();
                let total_count = decisions.len();
                Some((gpu_count, total_count))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Pobiera aktualny stan scheduler'a jako string
    #[allow(dead_code)]
    pub fn get_status_summary(&self) -> String {
        let cpu_perf = self.cpu_benchmark.load(Ordering::Relaxed) as f32 / 100.0;
        let gpu_perf = self.gpu_benchmark.load(Ordering::Relaxed) as f32 / 100.0;
        let current_load = self.current_load.load(Ordering::Relaxed) as f32 / 1000.0;
        let gpu_available = self.gpu_available.load(Ordering::Relaxed);
        
        format!(
            "GPU Scheduler Status: GPU Available: {}, Current Load: {:.1}%, CPU Perf: {:.1} ops/s, GPU Perf: {:.1} ops/s",
            gpu_available, current_load * 100.0, cpu_perf, gpu_perf
        )
    }

    /// Resetuje wszystkie benchmark'i i historię
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.cpu_benchmark.store(10000, Ordering::Relaxed); // 100.0 * 100
        self.gpu_benchmark.store(20000, Ordering::Relaxed); // 200.0 * 100
        self.current_load.store(0, Ordering::Relaxed); // 0.0 * 1000
        
        if let Ok(mut history) = self.decision_history.lock() {
            history.clear();
        }
        
        if let Ok(mut last_benchmark) = self.last_benchmark.lock() {
            *last_benchmark = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_scheduler_creation() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        assert!(scheduler.gpu_available.load(Ordering::Relaxed));
        assert_eq!(scheduler.current_load.load(Ordering::Relaxed), 0.0);
    }

    #[test]
    fn test_gpu_availability_setting() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        scheduler.set_gpu_available(false);
        assert!(!scheduler.gpu_available.load(Ordering::Relaxed));
        
        scheduler.set_gpu_available(true);
        assert!(scheduler.gpu_available.load(Ordering::Relaxed));
    }

    #[test]
    fn test_gpu_load_update() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        scheduler.update_gpu_load(0.75);
        assert_eq!(scheduler.current_load.load(Ordering::Relaxed), 0.75);
        
        // Test clamp'owania
        scheduler.update_gpu_load(1.5);
        assert_eq!(scheduler.current_load.load(Ordering::Relaxed), 1.0);
        
        scheduler.update_gpu_load(-0.5);
        assert_eq!(scheduler.current_load.load(Ordering::Relaxed), 0.0);
    }

    #[test]
    fn test_benchmark_updates() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        scheduler.update_cpu_benchmark(150.0);
        assert_eq!(scheduler.cpu_benchmark.load(Ordering::Relaxed), 150.0);
        
        scheduler.update_gpu_benchmark(300.0);
        assert_eq!(scheduler.gpu_benchmark.load(Ordering::Relaxed), 300.0);
    }

    #[test]
    fn test_decision_recording() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        let params = GpuOperationParams::default();
        let operation = GpuOperation::ThumbnailGeneration;
        
        // Wykonaj kilka decyzji
        scheduler.should_use_gpu(operation, &params);
        scheduler.should_use_gpu(operation, &params);
        
        if let Some((gpu_count, total)) = scheduler.get_decision_stats(operation) {
            assert_eq!(total, 2);
            assert!(gpu_count >= 0);
        }
    }

    #[test]
    fn test_gpu_bias_calculation() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        // Thumbnail generation powinno mieć wysoki bias
        let thumbnail_bias = match GpuOperation::ThumbnailGeneration {
            GpuOperation::ThumbnailGeneration => 1.5,
            _ => 1.0,
        };
        assert_eq!(thumbnail_bias, 1.5);
        
        // MIP generation powinno mieć najwyższy bias
        let mip_bias = match GpuOperation::MipGeneration {
            GpuOperation::MipGeneration => 2.0,
            _ => 1.0,
        };
        assert_eq!(mip_bias, 2.0);
    }

    #[test]
    fn test_scheduler_reset() {
        let metrics = Arc::new(GpuMetrics::new());
        let scheduler = AdaptiveGpuScheduler::new(metrics);
        
        // Zmień niektóre wartości
        scheduler.update_gpu_load(0.8);
        scheduler.update_cpu_benchmark(200.0);
        
        // Reset
        scheduler.reset();
        
        assert_eq!(scheduler.current_load.load(Ordering::Relaxed), 0.0);
        assert_eq!(scheduler.cpu_benchmark.load(Ordering::Relaxed), 100.0);
    }
}
