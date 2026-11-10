//! Performance Metrics and Parallel Speedup Aggregation
//!
//! Measures end-to-end wall-clock completion times, per-node dispatch and execution
//! latencies, payload transfer volumes, and computes speedup ratios against a
//! single-core baseline.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-004], [NFR-PERF-001]

use serde::{Deserialize, Serialize};

/// Per-node task execution latency and attempt tracking.
///
/// Traces to: [REQ-OP-004]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLatency {
    pub node_id: String,
    pub tile_index: u32,
    pub attempt: u32,
    pub duration_ms: u64,
}

/// Comprehensive benchmark execution report and throughput metrics.
///
/// Traces to: [REQ-OP-004], [NFR-PERF-001]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkMetrics {
    pub workload: String,
    pub filter: String,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub tiles_total: u32,
    pub tiles_successful: u32,
    pub active_worker_count: usize,
    pub wall_clock_duration_ms: u64,
    pub single_thread_baseline_ms: u64,
    pub parallel_speedup_ratio: f64,
    pub total_payload_bytes_transferred: usize,
    pub node_latencies: Vec<NodeLatency>,
}

impl BenchmarkMetrics {
    /// Computes parallel speedup ratio given single-thread baseline and wall-clock duration.
    ///
    /// Traces to: [REQ-OP-004]
    pub fn calculate_speedup(baseline_ms: u64, wall_clock_ms: u64) -> f64 {
        if wall_clock_ms == 0 {
            1.0
        } else {
            (baseline_ms as f64) / (wall_clock_ms as f64)
        }
    }

    /// Formats a human-readable ANSI terminal benchmark summary.
    pub fn format_summary(&self) -> String {
        let mut out = String::new();
        out.push_str("========================================================\n");
        out.push_str("          AndroGrid Parallel Compute Benchmark          \n");
        out.push_str("========================================================\n");
        out.push_str(&format!("Workload:                 {}\n", self.workload));
        out.push_str(&format!("Filter Operation:         {}\n", self.filter));
        out.push_str(&format!(
            "Input Dimensions:         {}x{} pixels\n",
            self.input_width, self.input_height
        ));
        out.push_str(&format!(
            "Output Dimensions:        {}x{} pixels\n",
            self.output_width, self.output_height
        ));
        out.push_str(&format!(
            "Tiles:                    {}/{} completed\n",
            self.tiles_successful, self.tiles_total
        ));
        out.push_str(&format!(
            "Active Cluster Workers:   {}\n",
            self.active_worker_count
        ));
        out.push_str("--------------------------------------------------------\n");
        out.push_str(&format!(
            "Single-Core Baseline:     {} ms\n",
            self.single_thread_baseline_ms
        ));
        out.push_str(&format!(
            "Distributed Wall-Clock:   {} ms\n",
            self.wall_clock_duration_ms
        ));
        out.push_str(&format!(
            "Parallel Speedup Ratio:   {:.2}x\n",
            self.parallel_speedup_ratio
        ));
        out.push_str(&format!(
            "Payload Transferred:      {:.2} MB ({} bytes)\n",
            (self.total_payload_bytes_transferred as f64) / (1024.0 * 1024.0),
            self.total_payload_bytes_transferred
        ));
        out.push_str("--------------------------------------------------------\n");
        out.push_str("Worker Latency Distribution:\n");

        if self.node_latencies.is_empty() {
            out.push_str("  (none recorded)\n");
        } else {
            for entry in &self.node_latencies {
                out.push_str(&format!(
                    "  - Node {}: tile #{} (attempt {}) finished in {} ms\n",
                    entry.node_id, entry.tile_index, entry.attempt, entry.duration_ms
                ));
            }
        }
        out.push_str("========================================================\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_speedup_ratio() {
        assert_eq!(BenchmarkMetrics::calculate_speedup(1000, 250), 4.0);
        assert_eq!(BenchmarkMetrics::calculate_speedup(500, 500), 1.0);
        assert_eq!(BenchmarkMetrics::calculate_speedup(100, 0), 1.0);
    }

    #[test]
    fn test_benchmark_metrics_format_summary() {
        let metrics = BenchmarkMetrics {
            workload: "image-processor".to_string(),
            filter: "sobel".to_string(),
            input_width: 1024,
            input_height: 1024,
            output_width: 1024,
            output_height: 1024,
            tiles_total: 4,
            tiles_successful: 4,
            active_worker_count: 2,
            wall_clock_duration_ms: 120,
            single_thread_baseline_ms: 360,
            parallel_speedup_ratio: 3.0,
            total_payload_bytes_transferred: 524288,
            node_latencies: vec![
                NodeLatency {
                    node_id: "worker-alpha".to_string(),
                    tile_index: 0,
                    attempt: 1,
                    duration_ms: 60,
                },
                NodeLatency {
                    node_id: "worker-beta".to_string(),
                    tile_index: 1,
                    attempt: 1,
                    duration_ms: 58,
                },
            ],
        };

        let summary = metrics.format_summary();
        assert!(summary.contains("Parallel Compute Benchmark"));
        assert!(summary.contains("Parallel Speedup Ratio:   3.00x"));
        assert!(summary.contains("worker-alpha"));
    }
}
