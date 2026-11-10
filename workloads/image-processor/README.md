# Canonical Hero Workload: Distributed Image Processor

## 1. Executive Summary & Overview
This directory contains the canonical "Hero" distributed compute workload for AndroGrid: a pure-Rust image-processing and computer vision pipeline targeting WebAssembly (`wasm32-wasip1`). It demonstrates parallel compute acceleration across multiple Android worker nodes by executing compute-intensive image filtering on discrete image tiles.

Traces to: AG-SPEC-2026-V2, [INV-005], [INV-006], [NFR-PERF-001], [REQ-OP-004]

---

## 2. Capabilities & Supported Filter Operations
The workload executes inside the unrooted Android WebAssembly runtime (Chicory/Wasmtime) without native C dependencies. Supported operations:

| Operation | Environment Value (`FILTER`) | Description |
| :--- | :--- | :--- |
| **Sobel Edge Detection** | `sobel` or `edges` | 3x3 horizontal and vertical Sobel gradient convolution magnitude. |
| **Gaussian Blur** | `blur` or `gaussian` | 3x3 separable Gaussian smoothing filter kernel. |
| **Bilinear Downscaling** | `downscale` or `resize` | 2x image downsampling with bilinear area interpolation. |
| **Histogram Equalization** | `equalize` or `hist_eq` | Cumulative distribution function luminance contrast enhancement. |
| **Grayscale** | `grayscale` or `gray` | ITU-R BT.601 perceptual luminance conversion. |
| **Invert** | `invert` or `negative` | Color inversion preserving original alpha channel. |

---

## 3. Wire Protocols & I/O Interface
The workload reads from standard input (`stdin`) and writes the processed output to standard output (`stdout`):

1. **PNG Stream:** Standard PNG file header (`0x89504E470D0A1A0A`). Decoded and re-encoded in pure Rust.
2. **Raw Tile Stream (`AGRAW`):** Raw uncompressed RGB/RGBA frame payload:
   - Magic: `b"AGRAW"` (5 bytes)
   - Width: `u32` (big-endian, 4 bytes)
   - Height: `u32` (big-endian, 4 bytes)
   - Channels: `u8` (1 byte: 3 for RGB, 4 for RGBA)
   - Payload: `width * height * channels` bytes

---

## 4. Compilation & Verification
Ensure the `wasm32-wasip1` target is installed:
```bash
rustup target add wasm32-wasip1
```

Compile the optimized release WebAssembly binary:
```bash
cargo build --target wasm32-wasip1 --release
```

The resulting binary is located at:
`target/wasm32-wasip1/release/image-processor.wasm`

Binary size is strictly bounded under 500 KB to guarantee rapid distribution across constrained mobile networks (`[NFR-PERF-001]`).

---

## 5. Standalone Execution Example
```bash
# Apply Sobel edge detection to a tile
cat tile_00.png | FILTER=sobel wasmtime target/wasm32-wasip1/release/image-processor.wasm > tile_00_sobel.png

# Apply Gaussian blur
cat tile_01.png | FILTER=blur wasmtime target/wasm32-wasip1/release/image-processor.wasm > tile_01_blur.png
```
