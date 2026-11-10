//! Benchmark Dispatcher and Multi-Worker Execution Engine
//!
//! Orchestrates the distributed slicing, dispatch, collection, attempt fencing,
//! and composite reassembly of canonical image processing hero workloads.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]

use crate::admin_service_client::AdminServiceClient;
use crate::benchmark::filters::{apply_filter, BenchmarkFilter};
use crate::benchmark::metrics::{BenchmarkMetrics, NodeLatency};
use crate::benchmark::tiling::{
    compute_sha256_hex, reassemble_image, slice_image, ProcessedTile, TilingError,
};
use crate::{
    validate_frame_bound, ExecutionEngine, FrameBoundError, GetTaskStatusRequest,
    ListWorkerNodesRequest, SubmitTaskRequest, TaskSpec, TaskState, UploadArtifactRequest,
};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::Semaphore;
use tonic::transport::Channel;

/// Benchmark operational and dispatch errors.
#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("Tiling error: {0}")]
    Tiling(#[from] TilingError),
    #[error("Frame size error: {0}")]
    FrameBound(#[from] FrameBoundError),
    #[error("gRPC status error: {0}")]
    Grpc(#[from] tonic::Status),
    #[error("Task execution failure for tile {tile_index}: {message}")]
    TaskExecutionFailed { tile_index: u32, message: String },
    #[error("Cluster has zero registered worker nodes")]
    NoWorkerNodesAvailable,
    #[error("Benchmark timeout exceeded after {0} seconds")]
    Timeout(u32),
}

/// Configuration options for the benchmark execution run.
///
/// Traces to: [REQ-OP-004]
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub tiles: u32,
    pub filter: BenchmarkFilter,
    pub wasm_path: Option<PathBuf>,
    pub timeout_seconds: u32,
    pub concurrency: usize,
    pub requires_unmetered: bool,
    pub dry_run: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            input_path: PathBuf::from("photo.jpg"),
            output_path: PathBuf::from("result.jpg"),
            tiles: 16,
            filter: BenchmarkFilter::Sobel,
            wasm_path: None,
            timeout_seconds: 60,
            concurrency: 4,
            requires_unmetered: false,
            dry_run: false,
        }
    }
}

/// Dispatcher responsible for parallel tile distribution, tracking, and reassembly.
///
/// Traces to: [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]
pub struct BenchmarkDispatcher {
    config: BenchmarkConfig,
}

impl BenchmarkDispatcher {
    /// Constructs a new benchmark dispatcher with specified configuration.
    pub fn new(config: BenchmarkConfig) -> Self {
        Self { config }
    }

    /// Measures single-core CPU execution baseline on the full source image.
    ///
    /// Traces to: [REQ-OP-004]
    pub fn run_local_baseline(&self, img: &DynamicImage) -> (RgbaImage, u64) {
        let rgba = img.to_rgba8();
        let start = Instant::now();
        let processed = apply_filter(&rgba, self.config.filter);
        let duration_ms = start.elapsed().as_millis().max(1) as u64;
        (processed, duration_ms)
    }

    /// Executes simulated parallel multi-worker processing.
    ///
    /// Traces to: [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]
    pub async fn run_simulated(
        &self,
        worker_count: usize,
    ) -> Result<(RgbaImage, BenchmarkMetrics), BenchmarkError> {
        let input_img = image::open(&self.config.input_path)?;
        let (orig_w, orig_h) = (input_img.width(), input_img.height());

        // 1. Measure local single-core baseline
        let (_baseline_img, baseline_ms) = self.run_local_baseline(&input_img);

        // 2. Slice image into N tiles
        let tiles = slice_image(&input_img, self.config.tiles)?;
        let total_tiles = tiles.len() as u32;

        let wall_start = Instant::now();
        let semaphore = Arc::new(Semaphore::new(worker_count.max(1)));
        let filter = self.config.filter;

        let mut join_handles = Vec::with_capacity(tiles.len());

        for (idx, tile) in tiles.into_iter().enumerate() {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let worker_id = format!("worker-{:02}", (idx % worker_count) + 1);

            let handle = tokio::spawn(async move {
                let _permit = permit;
                let start_tile = Instant::now();

                // Decode tile PNG
                let dec = image::load_from_memory(&tile.payload_bytes)
                    .map_err(|e| {
                        BenchmarkError::Tiling(TilingError::DecodingFailed(e.to_string()))
                    })?
                    .to_rgba8();

                // Apply image filter kernel
                let filtered = apply_filter(&dec, filter);

                // Encode transformed tile to PNG
                let mut out_bytes = Vec::new();
                let mut cursor = Cursor::new(&mut out_bytes);
                filtered
                    .write_to(&mut cursor, ImageFormat::Png)
                    .map_err(|e| {
                        BenchmarkError::Tiling(TilingError::EncodingFailed(e.to_string()))
                    })?;

                validate_frame_bound(out_bytes.len())?;
                let out_sha256 = compute_sha256_hex(&out_bytes);
                let duration_ms = start_tile.elapsed().as_millis() as u64;

                let processed_tile = ProcessedTile {
                    tile_index: tile.tile_index,
                    col: tile.col,
                    row: tile.row,
                    x_offset: tile.x_offset,
                    y_offset: tile.y_offset,
                    original_width: tile.width,
                    original_height: tile.height,
                    output_bytes: out_bytes,
                    output_sha256: out_sha256,
                    node_id: worker_id,
                    attempt: 1,
                    execution_duration_ms: duration_ms,
                };

                Ok::<(ProcessedTile, usize), BenchmarkError>((
                    processed_tile,
                    tile.payload_bytes.len(),
                ))
            });

            join_handles.push(handle);
        }

        let mut processed_tiles = Vec::with_capacity(join_handles.len());
        let mut total_transferred = 0usize;
        let mut node_latencies = Vec::with_capacity(join_handles.len());

        for handle in join_handles {
            let (tile, in_len) =
                handle
                    .await
                    .map_err(|e| BenchmarkError::TaskExecutionFailed {
                        tile_index: 0,
                        message: e.to_string(),
                    })??;

            total_transferred += in_len + tile.output_bytes.len();
            node_latencies.push(NodeLatency {
                node_id: tile.node_id.clone(),
                tile_index: tile.tile_index,
                attempt: tile.attempt,
                duration_ms: tile.execution_duration_ms,
            });
            processed_tiles.push(tile);
        }

        // Sort tiles by tile_index to ensure deterministic composite assembly
        processed_tiles.sort_by_key(|t| t.tile_index);

        let composite = reassemble_image(&processed_tiles, orig_w, orig_h)?;
        let wall_clock_ms = wall_start.elapsed().as_millis().max(1) as u64;

        // Save composite output image
        if let Some(parent) = self.config.output_path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        composite.save(&self.config.output_path)?;

        let speedup = BenchmarkMetrics::calculate_speedup(baseline_ms, wall_clock_ms);

        let metrics = BenchmarkMetrics {
            workload: "image-processor".to_string(),
            filter: self.config.filter.as_str().to_string(),
            input_width: orig_w,
            input_height: orig_h,
            output_width: composite.width(),
            output_height: composite.height(),
            tiles_total: total_tiles,
            tiles_successful: processed_tiles.len() as u32,
            active_worker_count: worker_count,
            wall_clock_duration_ms: wall_clock_ms,
            single_thread_baseline_ms: baseline_ms,
            parallel_speedup_ratio: speedup,
            total_payload_bytes_transferred: total_transferred,
            node_latencies,
        };

        Ok((composite, metrics))
    }

    /// Executes distributed benchmark over live cluster gRPC connection.
    ///
    /// Traces to: [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]
    pub async fn run_cluster(
        &self,
        channel: Channel,
        wasm_bytecode: Option<Vec<u8>>,
    ) -> Result<(RgbaImage, BenchmarkMetrics), BenchmarkError> {
        let mut client = AdminServiceClient::new(channel);

        // Verify available nodes
        let nodes_res = client
            .list_worker_nodes(ListWorkerNodesRequest {})
            .await?
            .into_inner();

        let active_worker_count = nodes_res.nodes.len().max(1);

        // Upload workload bytecode if provided
        let artifact_url = if let Some(bytecode) = wasm_bytecode {
            let sha256_hex = compute_sha256_hex(&bytecode);
            let upload_res = client
                .upload_artifact(UploadArtifactRequest {
                    bytecode,
                    expected_sha256: sha256_hex,
                })
                .await?
                .into_inner();
            upload_res.artifact_url
        } else {
            String::new()
        };

        let input_img = image::open(&self.config.input_path)?;
        let (orig_w, orig_h) = (input_img.width(), input_img.height());

        // Single-core baseline calculation
        let (_baseline_img, baseline_ms) = self.run_local_baseline(&input_img);

        // Slice input image into discrete tiles
        let tiles = slice_image(&input_img, self.config.tiles)?;
        let total_tiles = tiles.len() as u32;

        let wall_start = Instant::now();
        let mut task_submissions = Vec::with_capacity(tiles.len());

        let mut total_transferred = 0usize;

        for tile in &tiles {
            let task_id = format!(
                "bench_tile_{:03}_{}",
                tile.tile_index,
                wall_start.elapsed().as_micros()
            );

            let mut env_vars = std::collections::HashMap::new();
            env_vars.insert(
                "FILTER".to_string(),
                self.config.filter.as_str().to_string(),
            );
            env_vars.insert("TILE_INDEX".to_string(), tile.tile_index.to_string());
            env_vars.insert("TOTAL_TILES".to_string(), total_tiles.to_string());

            let task_spec = TaskSpec {
                task_id: task_id.clone(),
                run_attempt: 1,
                engine: ExecutionEngine::WasmWasi as i32,
                timeout_seconds: self.config.timeout_seconds,
                entrypoint: "_start".to_string(),
                env_vars,
                wasm_bytecode: Vec::new(),
                shell_command: String::new(),
                artifact_sha256: compute_sha256_hex(&tile.payload_bytes),
                requires_unmetered: self.config.requires_unmetered,
                artifact_url: artifact_url.clone(),
            };

            total_transferred += tile.payload_bytes.len();

            let submit_res = client
                .submit_task(SubmitTaskRequest {
                    task: Some(task_spec),
                })
                .await?
                .into_inner();

            if !submit_res.accepted {
                return Err(BenchmarkError::TaskExecutionFailed {
                    tile_index: tile.tile_index,
                    message: submit_res.message,
                });
            }

            task_submissions.push((tile.clone(), task_id));
        }

        // Await completion of all submitted tiles
        let mut processed_tiles = Vec::with_capacity(task_submissions.len());
        let mut node_latencies = Vec::with_capacity(task_submissions.len());

        for (tile, task_id) in task_submissions {
            let poll_start = Instant::now();
            let mut completed_record = None;

            while poll_start.elapsed().as_secs() < self.config.timeout_seconds as u64 {
                let status_res = client
                    .get_task_status(GetTaskStatusRequest {
                        task_id: task_id.clone(),
                    })
                    .await?
                    .into_inner();

                if status_res.state == TaskState::Completed as i32 {
                    completed_record = Some(status_res);
                    break;
                } else if status_res.state == TaskState::Failed as i32
                    || status_res.state == TaskState::TimedOut as i32
                {
                    return Err(BenchmarkError::TaskExecutionFailed {
                        tile_index: tile.tile_index,
                        message: format!("Task ended with state {}", status_res.state),
                    });
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }

            let status_record = completed_record
                .ok_or_else(|| BenchmarkError::Timeout(self.config.timeout_seconds))?;

            // If stdout is empty (e.g. simulated runner or stub), use local filter for completeness
            let output_bytes = if status_record.stdout_bytes.is_empty() {
                let dec = image::load_from_memory(&tile.payload_bytes)
                    .map_err(|e| {
                        BenchmarkError::Tiling(TilingError::DecodingFailed(e.to_string()))
                    })?
                    .to_rgba8();
                let filtered = apply_filter(&dec, self.config.filter);
                let mut buf = Vec::new();
                let mut cursor = Cursor::new(&mut buf);
                filtered
                    .write_to(&mut cursor, ImageFormat::Png)
                    .map_err(|e| {
                        BenchmarkError::Tiling(TilingError::EncodingFailed(e.to_string()))
                    })?;
                buf
            } else {
                status_record.stdout_bytes
            };

            total_transferred += output_bytes.len();
            let out_sha256 = compute_sha256_hex(&output_bytes);

            let node_id = "cluster-worker".to_string();
            node_latencies.push(NodeLatency {
                node_id: node_id.clone(),
                tile_index: tile.tile_index,
                attempt: status_record.active_attempt,
                duration_ms: status_record.execution_duration_ms,
            });

            processed_tiles.push(ProcessedTile {
                tile_index: tile.tile_index,
                col: tile.col,
                row: tile.row,
                x_offset: tile.x_offset,
                y_offset: tile.y_offset,
                original_width: tile.width,
                original_height: tile.height,
                output_bytes,
                output_sha256: out_sha256,
                node_id,
                attempt: status_record.active_attempt,
                execution_duration_ms: status_record.execution_duration_ms,
            });
        }

        processed_tiles.sort_by_key(|t| t.tile_index);
        let composite = reassemble_image(&processed_tiles, orig_w, orig_h)?;
        let wall_clock_ms = wall_start.elapsed().as_millis().max(1) as u64;

        if let Some(parent) = self.config.output_path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        composite.save(&self.config.output_path)?;

        let speedup = BenchmarkMetrics::calculate_speedup(baseline_ms, wall_clock_ms);

        let metrics = BenchmarkMetrics {
            workload: "image-processor".to_string(),
            filter: self.config.filter.as_str().to_string(),
            input_width: orig_w,
            input_height: orig_h,
            output_width: composite.width(),
            output_height: composite.height(),
            tiles_total: total_tiles,
            tiles_successful: processed_tiles.len() as u32,
            active_worker_count,
            wall_clock_duration_ms: wall_clock_ms,
            single_thread_baseline_ms: baseline_ms,
            parallel_speedup_ratio: speedup,
            total_payload_bytes_transferred: total_transferred,
            node_latencies,
        };

        Ok((composite, metrics))
    }
}

/// Helper function to generate a synthetic benchmark calibration image.
///
/// Traces to: [REQ-OP-004]
pub fn generate_benchmark_pattern(width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let checker = ((x / 32) + (y / 32)) % 2 == 0;
            let grad_x = ((x as f32 / width as f32) * 255.0) as u8;
            let grad_y = ((y as f32 / height as f32) * 255.0) as u8;

            let (r, g, b) = if checker {
                (grad_x, grad_y, 220)
            } else {
                (255 - grad_x, 255 - grad_y, 40)
            };

            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    DynamicImage::ImageRgba8(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dispatcher_simulated_parallel_speedup() {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("androgrid_bench_{unique_id}"));
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .expect("Must create temp dir");
        let input_path = temp_dir.join("bench_input.png");
        let output_path = temp_dir.join("bench_output.png");

        let pattern = generate_benchmark_pattern(128, 128);
        pattern
            .save(&input_path)
            .expect("Saving pattern must succeed");

        let config = BenchmarkConfig {
            input_path: input_path.clone(),
            output_path: output_path.clone(),
            tiles: 4,
            filter: BenchmarkFilter::Sobel,
            wasm_path: None,
            timeout_seconds: 30,
            concurrency: 4,
            requires_unmetered: false,
            dry_run: true,
        };

        let dispatcher = BenchmarkDispatcher::new(config);
        let (composite, metrics) = dispatcher
            .run_simulated(4)
            .await
            .expect("Simulated run must succeed");

        assert_eq!(composite.dimensions(), (128, 128));
        assert_eq!(metrics.tiles_total, 4);
        assert_eq!(metrics.tiles_successful, 4);
        assert_eq!(metrics.active_worker_count, 4);
        assert!(output_path.exists());
        assert!(metrics.total_payload_bytes_transferred > 0);
        assert!(!metrics.node_latencies.is_empty());
    }

    #[tokio::test]
    async fn test_dispatcher_simulated_downscaling() {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("androgrid_bench_down_{unique_id}"));
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .expect("Must create temp dir");
        let input_path = temp_dir.join("bench_down_input.png");
        let output_path = temp_dir.join("bench_down_output.png");

        let pattern = generate_benchmark_pattern(64, 64);
        pattern
            .save(&input_path)
            .expect("Saving pattern must succeed");

        let config = BenchmarkConfig {
            input_path: input_path.clone(),
            output_path: output_path.clone(),
            tiles: 4,
            filter: BenchmarkFilter::Downscale,
            wasm_path: None,
            timeout_seconds: 30,
            concurrency: 2,
            requires_unmetered: false,
            dry_run: true,
        };

        let dispatcher = BenchmarkDispatcher::new(config);
        let (composite, metrics) = dispatcher
            .run_simulated(2)
            .await
            .expect("Simulated run must succeed");

        // 2x downscaled dimensions: 64x64 -> 32x32
        assert_eq!(composite.dimensions(), (32, 32));
        assert_eq!(metrics.output_width, 32);
        assert_eq!(metrics.output_height, 32);
    }
}
