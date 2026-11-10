# ADR-003: Canonical Hero Workload: Distributed Image Processing & Parallel Compute Benchmark

## Status
Accepted

## Architectural Scope & Requirements
- **Feature Reference:** Issue #26
- **GitHub Issue:** https://github.com/MrQuality/androgrid-backend/issues/26
- **Traceability:** `[INV-005]`, `[INV-006]`, `[NFR-PERF-001]`, `[REQ-OP-004]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
Mobile edge computing clusters frequently face skepticism regarding practical utility beyond trivial synthetic test scripts (such as Fibonacci recursions or string echoing). Operators and developers require a canonical, visually demonstrable "Hero" workload that proves measurable multi-core parallel speedup when tasks are distributed across connected Android devices.

Key engineering constraints:
1. **[INV-006] WASI Sandboxing:** The compute workload must compile to `wasm32-wasip1` and execute within unrooted Android mobile runtimes (Chicory / Wasmtime) without native C dependencies or host library requirements.
2. **[NFR-PERF-001] Message Frame Boundaries:** Individual image tile chunk payloads transferred over gRPC or content-addressed artifact servers must strictly adhere to the 16 MB message ceiling (defaulting to 512x512 tiles, approx. 150 KB to 1 MB).
3. **[INV-005] Speculative Dual-Assignment & Attempt Fencing:** The parallel distribution loop must handle opportunistic Wi-Fi preemption and monotonic attempt fencing (`[REQ-OP-004]`), ensuring the winning worker node commits the tile output without race conditions.
4. **Empirical Benchmarking Rigor:** The benchmark engine must measure single-core CPU execution baseline times and compute verified parallel speedup ratios, node latency distributions, and total network bandwidth consumed.
5. **Typographic Standard:** Zero occurrences of Unicode em dashes (U+2014) or en dashes (U+2013).

---

## Decision
We implement a canonical image-processing pipeline and distributed benchmark suite comprising:

1. **WASI Image Processor Workload (`workloads/image-processor`):**
   - Pure Rust crate targeting `wasm32-wasip1` with binary footprint under 500 KB (`441 KB`).
   - Implements 6 deterministic image transformation filters:
     - 3x3 Sobel gradient edge detection.
     - 3x3 separable Gaussian blur.
     - 2x bilinear downscaling.
     - Luminance CDF histogram equalization.
     - ITU-R BT.601 perceptual grayscale.
     - Alpha-preserving color inversion.
   - Dual wire format support: standard PNG byte stream and raw uncompressed `AGRAW` pixel frames.

2. **Geometric Image Slicer & Composite Reassembler (`src/benchmark/tiling.rs`):**
   - Partitions high-resolution images into $N$ rectangular tiles adhering to optimal 2D aspect grids (e.g. 1, 4, 9, 16 tiles).
   - Validates that each encoded tile payload respects `MAX_FRAME_SIZE_BYTES` (16 MB).
   - Reassembles processed tiles into a unified composite image, automatically accounting for downscaled tile coordinate remapping.

3. **High-Concurrency Benchmark Dispatcher (`src/benchmark/dispatcher.rs`):**
   - Measures single-core local baseline runtime on the full source image before dispatch.
   - Supports both remote cluster execution over mTLS gRPC (`AdminServiceClient`) and local simulated multi-worker execution via Tokio async worker pools.
   - Computes wall-clock completion duration, per-node latency distribution, total bytes transferred, and empirical parallel speedup ratios ($T_{baseline} / T_{wall}$).

4. **Operator CLI Subcommand (`androgrid-cli benchmark`):**
   - Exposes benchmark options: `--input`, `--output`, `--tiles`, `--filter`, `--concurrency`, `--timeout`, `--wasm`, and `--dry-run`.
   - Supports both ANSI terminal summary tables and machine-readable JSON output (`--format json`).

---

## Consequences
- **Positive:**
  - Provides a tangible, visually verifiable edge compute showcase for operators and open-source evaluators.
  - Guarantees hermetic WASI P1 sandboxing across both rooted and unrooted Android devices (`[INV-006]`).
  - Strict compliance with `[NFR-PERF-001]` frame boundaries through geometric tile chunking.
  - Repeatable empirical measurement of cluster throughput and multi-worker speedup ratios.
- **Trade-offs:**
  - Convolution filters (Sobel, Gaussian blur) on independent tiles without overlap padding exhibit minor boundary artifacting at tile seams; considered acceptable for standard embarrassingly parallel benchmarking.
