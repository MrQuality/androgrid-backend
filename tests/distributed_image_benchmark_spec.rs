//! Integration Specification: Canonical Hero Workload Distributed Image Benchmark
//!
//! Validates WASI Preview 1 sandboxing, geometric tile slicing, frame size boundaries,
//! filter transformations, simulated multi-worker dispatch, attempt fencing, and
//! parallel speedup metric aggregation.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]

use androgrid_backend::{
    apply_filter, calculate_optimal_grid, generate_benchmark_pattern, reassemble_image,
    slice_image, validate_frame_bound, BenchmarkConfig, BenchmarkDispatcher, BenchmarkFilter,
    ProcessedTile, WasiValidator, MAX_FRAME_SIZE_BYTES,
};
use image::ImageFormat;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

fn get_fixture_wasm_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("tests")
        .join("fixtures")
        .join("wasm")
        .join("image_processor.wasm")
}

fn get_sample_image_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("tests")
        .join("fixtures")
        .join("images")
        .join("benchmark_sample.png")
}

/// Validates that the compiled image-processor WASI module conforms to WASI Preview 1,
/// contains standard memory/entrypoint exports, and remains under the 500 KB binary limit.
///
/// Traces to: [INV-006], [NFR-PERF-001]
#[test]
fn test_image_processor_wasm_wasi_validation() {
    let wasm_path = get_fixture_wasm_path();
    assert!(
        wasm_path.exists(),
        "image_processor.wasm must exist at {:?}",
        wasm_path
    );

    let bytecode = fs::read(&wasm_path).expect("Failed to read image_processor.wasm bytecode");
    assert!(
        bytecode.len() < 500 * 1024,
        "Module size of {} bytes exceeds 500 KB limit for mobile edge distribution",
        bytecode.len()
    );

    let manifest = WasiValidator::validate(&bytecode, "_start")
        .expect("WASI Preview 1 validation must succeed");

    assert_eq!(manifest.entrypoint, "_start");
    assert!(manifest.is_wasi_compatible);
    assert_eq!(manifest.artifact_sha256.len(), 64);
    assert!(manifest.exports.contains(&"_start".to_string()));
    assert!(manifest.exports.contains(&"memory".to_string()));
}

/// Validates geometric image tiling and strict frame size boundaries across various grid configurations.
///
/// Traces to: [NFR-PERF-001], [REQ-OP-004]
#[test]
fn test_image_geometric_tiling_and_frame_sizing_invariants() {
    let pattern = generate_benchmark_pattern(512, 512);

    for tile_count in [1, 4, 9, 16] {
        let tiles = slice_image(&pattern, tile_count).expect("Tiling must succeed");
        assert_eq!(tiles.len(), tile_count as usize);

        let (grid_x, grid_y) = calculate_optimal_grid(tile_count);
        assert_eq!(grid_x * grid_y, tile_count);

        for t in &tiles {
            assert!(t.payload_bytes.len() < MAX_FRAME_SIZE_BYTES);
            assert!(validate_frame_bound(t.payload_bytes.len()).is_ok());
            assert_eq!(t.payload_sha256.len(), 64);
            assert_eq!(t.total_tiles, tile_count);

            // Verify payload is valid PNG
            let dec = image::load_from_memory(&t.payload_bytes).expect("Tile must decode as PNG");
            assert_eq!(dec.width(), t.width);
            assert_eq!(dec.height(), t.height);
        }
    }
}

/// Validates filter transformations and composite reassembly with pixel-level assertions.
///
/// Traces to: [INV-006], [REQ-OP-004]
#[test]
fn test_image_filter_transformations_and_reconstruction() {
    let pattern = generate_benchmark_pattern(128, 128);
    let tiles = slice_image(&pattern, 4).expect("Slicing must succeed");

    for filter in [
        BenchmarkFilter::Sobel,
        BenchmarkFilter::Blur,
        BenchmarkFilter::Equalize,
        BenchmarkFilter::Grayscale,
        BenchmarkFilter::Invert,
    ] {
        let mut processed = Vec::new();
        for t in &tiles {
            let dec = image::load_from_memory(&t.payload_bytes)
                .unwrap()
                .to_rgba8();
            let filtered = apply_filter(&dec, filter);

            let mut buf = Vec::new();
            let mut cursor = Cursor::new(&mut buf);
            filtered.write_to(&mut cursor, ImageFormat::Png).unwrap();

            processed.push(ProcessedTile {
                tile_index: t.tile_index,
                col: t.col,
                row: t.row,
                x_offset: t.x_offset,
                y_offset: t.y_offset,
                original_width: t.width,
                original_height: t.height,
                output_bytes: buf,
                output_sha256: t.payload_sha256.clone(),
                node_id: "worker-01".to_string(),
                attempt: 1,
                execution_duration_ms: 10,
            });
        }

        let composite = reassemble_image(&processed, 128, 128).expect("Reassembly must succeed");
        assert_eq!(composite.dimensions(), (128, 128));

        // For local-stencil and point filters, interior pixels match direct full-image processing
        if filter != BenchmarkFilter::Equalize {
            let direct_expected = apply_filter(&pattern.to_rgba8(), filter);
            for y in 10..50 {
                for x in 10..50 {
                    let comp_p = composite.get_pixel(x, y);
                    let dir_p = direct_expected.get_pixel(x, y);
                    assert_eq!(
                        comp_p, dir_p,
                        "Mismatch at ({x}, {y}) for filter {filter:?}"
                    );
                }
            }
        }
    }
}

/// Validates simulated multi-worker benchmark dispatch, parallel speedup calculation,
/// latency reporting, and total payload volume tracking.
///
/// Traces to: [INV-005], [REQ-OP-004], [NFR-PERF-001]
#[tokio::test]
async fn test_benchmark_dispatcher_speedup_and_latency_aggregation() {
    let sample_path = get_sample_image_path();
    if !sample_path.exists() {
        if let Some(parent) = sample_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create fixtures/images dir");
        }
        let pattern = generate_benchmark_pattern(256, 256);
        pattern
            .save(&sample_path)
            .expect("Failed to save sample image fixture");
    }

    let temp_dir = std::env::temp_dir().join(format!(
        "androgrid_spec_bench_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).expect("Failed to create tempdir");
    let output_path = temp_dir.join("benchmark_result.png");

    let config = BenchmarkConfig {
        input_path: sample_path.clone(),
        output_path: output_path.clone(),
        tiles: 4,
        filter: BenchmarkFilter::Sobel,
        wasm_path: Some(get_fixture_wasm_path()),
        timeout_seconds: 30,
        concurrency: 4,
        requires_unmetered: false,
        dry_run: true,
    };

    let dispatcher = BenchmarkDispatcher::new(config);
    let (composite, metrics) = dispatcher
        .run_simulated(4)
        .await
        .expect("Benchmark execution must succeed");

    assert_eq!(composite.dimensions(), (256, 256));
    assert!(output_path.exists());
    assert_eq!(metrics.tiles_total, 4);
    assert_eq!(metrics.tiles_successful, 4);
    assert_eq!(metrics.active_worker_count, 4);
    assert!(metrics.single_thread_baseline_ms > 0);
    assert!(metrics.wall_clock_duration_ms > 0);
    assert!(metrics.parallel_speedup_ratio > 0.0);
    assert!(metrics.total_payload_bytes_transferred > 0);
    assert_eq!(metrics.node_latencies.len(), 4);

    let summary = metrics.format_summary();
    assert!(summary.contains("AndroGrid Parallel Compute Benchmark"));
    assert!(summary.contains("Parallel Speedup Ratio:"));
}

/// Typographic regression test asserting zero occurrences of forbidden Unicode dashes
/// (U+2014, U+2013) across workload and benchmark source files.
#[test]
fn test_zero_unicode_dashes_in_workloads() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let paths_to_check = [
        manifest_dir.join("src").join("benchmark"),
        manifest_dir.join("workloads").join("image-processor"),
    ];

    fn check_dir(dir: &std::path::Path) {
        if !dir.exists() {
            return;
        }
        for entry in fs::read_dir(dir).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let p = entry.path();
            if p.is_dir() {
                if !p.to_string_lossy().contains("target") {
                    check_dir(&p);
                }
            } else if let Some(ext) = p.extension() {
                if ext == "rs" || ext == "md" || ext == "toml" {
                    let content = fs::read_to_string(&p).unwrap_or_default();
                    assert!(
                        !content.contains('\u{2014}'),
                        "Em dash (U+2014) found in {:?}",
                        p
                    );
                    assert!(
                        !content.contains('\u{2013}'),
                        "En dash (U+2013) found in {:?}",
                        p
                    );
                }
            }
        }
    }

    for path in paths_to_check {
        check_dir(&path);
    }
}
