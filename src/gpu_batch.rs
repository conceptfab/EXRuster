use std::sync::Arc;
use anyhow::Result;
use crate::gpu_context::GpuContext;
use crate::gpu_thumbnails::generate_thumbnail_from_pixels_gpu;

use glam::Mat3;
use rayon::prelude::*;
use std::collections::HashMap;

/// Struktura reprezentująca zadanie batch processing
#[derive(Clone, Debug)]
pub struct BatchJob {
    pub id: String,
    pub job_type: BatchJobType,
}

/// Typy zadań batch processing
#[derive(Clone, Debug)]
pub enum BatchJobType {
    /// Generacja miniaturek
    ThumbnailGeneration {
        pixels: Vec<f32>,
        src_width: u32,
        src_height: u32,
        target_height: u32,
        exposure: f32,
        gamma: f32,
        tonemap_mode: i32,
        color_matrix: Option<Mat3>,
    },
}

/// Wynik zadania batch processing
#[derive(Clone, Debug)]
pub enum BatchJobResult {
    Thumbnail {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
    },
}

/// GPU Batch Processor
pub struct GpuBatchProcessor {
    gpu_context: Arc<GpuContext>,
    max_concurrent_jobs: usize,
}

impl GpuBatchProcessor {
    /// Tworzy nowy batch processor
    pub fn new(gpu_context: Arc<GpuContext>) -> Self {
        Self {
            gpu_context,
            max_concurrent_jobs: 4, // Domyślnie 4 równoległe zadania GPU
        }
    }



    /// Przetwarzanie pojedynczego zadania
    pub fn process_job(&self, job: BatchJob) -> Result<(String, BatchJobResult)> {
        let job_id = job.id.clone();
        let result = match job.job_type {
            BatchJobType::ThumbnailGeneration {
                pixels,
                src_width,
                src_height,
                target_height,
                exposure,
                gamma,
                tonemap_mode,
                color_matrix,
            } => {
                let (bytes, width, height) = generate_thumbnail_from_pixels_gpu(
                    &self.gpu_context,
                    &pixels,
                    src_width,
                    src_height,
                    target_height,
                    exposure,
                    gamma,
                    tonemap_mode,
                    color_matrix,
                )?;
                
                BatchJobResult::Thumbnail {
                    bytes,
                    width,
                    height,
                }
            }
        };
        
        Ok((job_id, result))
    }

    /// Przetwarzanie listy zadań w batch'ach
    pub fn process_batch(&self, jobs: Vec<BatchJob>) -> Vec<Result<(String, BatchJobResult)>> {
        // Podziel zadania na chunki o rozmiarze max_concurrent_jobs
        jobs.par_chunks(self.max_concurrent_jobs)
            .map(|chunk| {
                // Przetwórz chunk równolegle
                chunk
                    .par_iter()
                    .map(|job| self.process_job(job.clone()))
                    .collect::<Vec<_>>()
            })
            .flatten()
            .collect()
    }

    /// Pomocnicza funkcja do tworzenia zadań thumbnail
    pub fn create_thumbnail_job(
        id: String,
        pixels: Vec<f32>,
        src_width: u32,
        src_height: u32,
        target_height: u32,
        exposure: f32,
        gamma: f32,
        tonemap_mode: i32,
        color_matrix: Option<Mat3>,
    ) -> BatchJob {
        BatchJob {
            id,
            job_type: BatchJobType::ThumbnailGeneration {
                pixels,
                src_width,
                src_height,
                target_height,
                exposure,
                gamma,
                tonemap_mode,
                color_matrix,
            },
        }
    }


}

/// High-level batch processing functions
/// Przetwarza listę plików EXR do miniaturek
pub fn batch_process_thumbnails_gpu(
    gpu_context: Arc<GpuContext>,
    image_data: Vec<(String, Vec<f32>, u32, u32)>, // (id, pixels, width, height)
    target_height: u32,
    exposure: f32,
    gamma: f32,
    tonemap_mode: i32,
) -> HashMap<String, (Vec<u8>, u32, u32)> {
    let processor = GpuBatchProcessor::new(gpu_context);
    
    let jobs: Vec<BatchJob> = image_data
        .into_iter()
        .map(|(id, pixels, width, height)| {
            GpuBatchProcessor::create_thumbnail_job(
                id,
                pixels,
                width,
                height,
                target_height,
                exposure,
                gamma,
                tonemap_mode,
                None,
            )
        })
        .collect();
    
    let results = processor.process_batch(jobs);
    let mut thumbnails = HashMap::new();
    
    for result in results {
        if let Ok((id, BatchJobResult::Thumbnail { bytes, width, height })) = result {
            thumbnails.insert(id, (bytes, width, height));
        }
    }
    
    thumbnails
}

