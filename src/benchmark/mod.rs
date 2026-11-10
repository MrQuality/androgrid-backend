//! Canonical Distributed Benchmark and Parallel Compute Engine
//!
//! Provides distributed image slicing, WebAssembly task distribution,
//! attempt fencing, and composite image reconstruction for mobile edge nodes.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]

pub mod dispatcher;
pub mod filters;
pub mod metrics;
pub mod tiling;

pub use dispatcher::{
    generate_benchmark_pattern, BenchmarkConfig, BenchmarkDispatcher, BenchmarkError,
};
pub use filters::{apply_filter, BenchmarkFilter};
pub use metrics::{BenchmarkMetrics, NodeLatency};
pub use tiling::{
    calculate_optimal_grid, compute_sha256_hex, reassemble_image, slice_image, ImageTile,
    ProcessedTile, TilingError,
};
