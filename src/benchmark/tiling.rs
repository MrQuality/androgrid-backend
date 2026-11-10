//! Image Geometric Tiling and Composite Reassembly
//!
//! Slices large input images into discrete, independent tiles respecting the
//! gRPC 16 MB frame payload boundary, and reassembles distributed worker outputs
//! into a composite reconstructed image.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [REQ-OP-004], [INV-006]

use crate::{validate_frame_bound, FrameBoundError};
use image::{DynamicImage, GenericImageView, ImageBuffer, ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use thiserror::Error;

/// Tiling and reassembly operational errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TilingError {
    #[error("Image dimensions {width}x{height} cannot be sliced into {tiles} tiles")]
    InvalidDimensions { width: u32, height: u32, tiles: u32 },
    #[error("Zero tiles requested: must specify at least 1 tile")]
    ZeroTilesRequested,
    #[error("Image payload encoding failed: {0}")]
    EncodingFailed(String),
    #[error("Image payload decoding failed: {0}")]
    DecodingFailed(String),
    #[error("Frame size boundary violation: {0}")]
    FrameBoundExceeded(#[from] FrameBoundError),
    #[error("Missing tile at index {0} during reassembly")]
    MissingTile(u32),
}

/// Sliced image tile payload ready for decoupled cluster distribution.
///
/// Traces to: [NFR-PERF-001], [REQ-OP-004]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageTile {
    pub tile_index: u32,
    pub total_tiles: u32,
    pub grid_x: u32,
    pub grid_y: u32,
    pub col: u32,
    pub row: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub width: u32,
    pub height: u32,
    pub payload_bytes: Vec<u8>,
    pub payload_sha256: String,
}

/// Processed tile payload returned by a worker node.
///
/// Traces to: [REQ-OP-004], [INV-005]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedTile {
    pub tile_index: u32,
    pub col: u32,
    pub row: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub output_bytes: Vec<u8>,
    pub output_sha256: String,
    pub node_id: String,
    pub attempt: u32,
    pub execution_duration_ms: u64,
}

/// Calculates the optimal 2D grid layout (columns, rows) for a given tile count.
///
/// Traces to: [REQ-OP-004]
pub fn calculate_optimal_grid(tile_count: u32) -> (u32, u32) {
    if tile_count <= 1 {
        return (1, 1);
    }

    let sqrt = (tile_count as f64).sqrt().floor() as u32;
    for cols in (1..=sqrt).rev() {
        if tile_count % cols == 0 {
            let rows = tile_count / cols;
            // Prefer wider or square layouts
            return (rows, cols);
        }
    }

    // Fallback for prime tile counts
    (tile_count, 1)
}

/// Computes the SHA-256 hexadecimal digest for a byte buffer.
pub fn compute_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Slices an input image into geometric tiles adhering to the 16 MB frame bound.
///
/// Traces to: [NFR-PERF-001], [REQ-OP-004]
///
/// # Arguments
/// * `img` - Reference to input dynamic image.
/// * `tile_count` - Target number of tiles (e.g. 1, 4, 9, 16).
///
/// # Returns
/// * `Ok(Vec<ImageTile>)` - Vector of sliced and PNG-encoded tiles.
/// * `Err(TilingError)` - If dimensions or frame sizes are invalid.
pub fn slice_image(img: &DynamicImage, tile_count: u32) -> Result<Vec<ImageTile>, TilingError> {
    if tile_count == 0 {
        return Err(TilingError::ZeroTilesRequested);
    }

    let (img_w, img_h) = img.dimensions();
    if img_w == 0 || img_h == 0 {
        return Err(TilingError::InvalidDimensions {
            width: img_w,
            height: img_h,
            tiles: tile_count,
        });
    }

    let (grid_x, grid_y) = calculate_optimal_grid(tile_count);
    let mut tiles = Vec::with_capacity(tile_count as usize);

    let base_tile_w = img_w / grid_x;
    let base_tile_h = img_h / grid_y;

    if base_tile_w == 0 || base_tile_h == 0 {
        return Err(TilingError::InvalidDimensions {
            width: img_w,
            height: img_h,
            tiles: tile_count,
        });
    }

    let mut tile_idx = 0;
    for row in 0..grid_y {
        let y_offset = row * base_tile_h;
        let tile_h = if row == grid_y - 1 {
            img_h - y_offset
        } else {
            base_tile_h
        };

        for col in 0..grid_x {
            let x_offset = col * base_tile_w;
            let tile_w = if col == grid_x - 1 {
                img_w - x_offset
            } else {
                base_tile_w
            };

            let sub_view = img.view(x_offset, y_offset, tile_w, tile_h);
            let sub_img = sub_view.to_image();

            let mut encoded_buf = Vec::new();
            let mut cursor = Cursor::new(&mut encoded_buf);
            sub_img
                .write_to(&mut cursor, ImageFormat::Png)
                .map_err(|e| TilingError::EncodingFailed(e.to_string()))?;

            // Strict enforcement of [NFR-PERF-001]
            validate_frame_bound(encoded_buf.len())?;

            let digest = compute_sha256_hex(&encoded_buf);

            tiles.push(ImageTile {
                tile_index: tile_idx,
                total_tiles: tile_count,
                grid_x,
                grid_y,
                col,
                row,
                x_offset,
                y_offset,
                width: tile_w,
                height: tile_h,
                payload_bytes: encoded_buf,
                payload_sha256: digest,
            });

            tile_idx += 1;
        }
    }

    Ok(tiles)
}

/// Reassembles processed tiles into a unified composite image.
///
/// Traces to: [REQ-OP-004], [NFR-PERF-001]
///
/// # Arguments
/// * `tiles` - Processed tile outputs collected from worker nodes.
/// * `orig_width` - Original unscaled source image width.
/// * `orig_height` - Original unscaled source image height.
///
/// # Returns
/// * `Ok(RgbaImage)` - Reassembled composite image.
/// * `Err(TilingError)` - If decoding or placement fails.
pub fn reassemble_image(
    tiles: &[ProcessedTile],
    orig_width: u32,
    orig_height: u32,
) -> Result<RgbaImage, TilingError> {
    if tiles.is_empty() {
        return Err(TilingError::ZeroTilesRequested);
    }

    // Decode all tiles first to inspect dimensions and determine if downscaling occurred
    let mut decoded_tiles = Vec::with_capacity(tiles.len());
    for t in tiles {
        let dec = image::load_from_memory(&t.output_bytes)
            .map_err(|e| TilingError::DecodingFailed(e.to_string()))?
            .to_rgba8();
        decoded_tiles.push((t, dec));
    }

    // Determine scale ratio from first tile
    let (first_meta, first_dec) = &decoded_tiles[0];
    let scale_x = first_dec.width() as f64 / first_meta.original_width as f64;
    let scale_y = first_dec.height() as f64 / first_meta.original_height as f64;

    let target_w = (orig_width as f64 * scale_x).round().max(1.0) as u32;
    let target_h = (orig_height as f64 * scale_y).round().max(1.0) as u32;

    let mut composite = ImageBuffer::new(target_w, target_h);

    for (meta, tile_img) in &decoded_tiles {
        let dest_x = (meta.x_offset as f64 * scale_x).round() as u32;
        let dest_y = (meta.y_offset as f64 * scale_y).round() as u32;

        let (tw, th) = tile_img.dimensions();
        for py in 0..th {
            let cy = dest_y + py;
            if cy >= target_h {
                continue;
            }
            for px in 0..tw {
                let cx = dest_x + px;
                if cx >= target_w {
                    continue;
                }
                composite.put_pixel(cx, cy, *tile_img.get_pixel(px, py));
            }
        }
    }

    Ok(composite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn test_calculate_optimal_grid() {
        assert_eq!(calculate_optimal_grid(1), (1, 1));
        assert_eq!(calculate_optimal_grid(2), (2, 1));
        assert_eq!(calculate_optimal_grid(4), (2, 2));
        assert_eq!(calculate_optimal_grid(6), (3, 2));
        assert_eq!(calculate_optimal_grid(9), (3, 3));
        assert_eq!(calculate_optimal_grid(16), (4, 4));
    }

    #[test]
    fn test_slice_and_reassemble_identity() {
        // Create 20x20 test image with distinct quadrant colors
        let mut img = RgbaImage::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let r = (x * 12) as u8;
                let g = (y * 12) as u8;
                img.put_pixel(x, y, Rgba([r, g, 128, 255]));
            }
        }
        let dyn_img = DynamicImage::ImageRgba8(img.clone());

        // Slice into 4 tiles (2x2)
        let tiles = slice_image(&dyn_img, 4).expect("Slicing must succeed");
        assert_eq!(tiles.len(), 4);

        for t in &tiles {
            assert!(t.payload_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(validate_frame_bound(t.payload_bytes.len()).is_ok());
            assert_eq!(t.width, 10);
            assert_eq!(t.height, 10);
        }

        // Simulate identity pass-through
        let processed: Vec<ProcessedTile> = tiles
            .into_iter()
            .map(|t| ProcessedTile {
                tile_index: t.tile_index,
                col: t.col,
                row: t.row,
                x_offset: t.x_offset,
                y_offset: t.y_offset,
                original_width: t.width,
                original_height: t.height,
                output_bytes: t.payload_bytes.clone(),
                output_sha256: t.payload_sha256,
                node_id: "test-node-1".to_string(),
                attempt: 1,
                execution_duration_ms: 5,
            })
            .collect();

        let composite = reassemble_image(&processed, 20, 20).expect("Reassembly must succeed");
        assert_eq!(composite.dimensions(), (20, 20));

        // Exact pixel parity assertion
        for y in 0..20 {
            for x in 0..20 {
                assert_eq!(composite.get_pixel(x, y), img.get_pixel(x, y));
            }
        }
    }

    #[test]
    fn test_slice_image_odd_dimensions_coverage() {
        // 25x15 image sliced into 4 tiles: should handle remaining boundary pixels
        let mut img = RgbaImage::new(25, 15);
        for y in 0..15 {
            for x in 0..25 {
                img.put_pixel(x, y, Rgba([x as u8, y as u8, 200, 255]));
            }
        }
        let dyn_img = DynamicImage::ImageRgba8(img.clone());

        let tiles = slice_image(&dyn_img, 4).expect("Slicing must succeed");
        assert_eq!(tiles.len(), 4);

        let processed: Vec<ProcessedTile> = tiles
            .into_iter()
            .map(|t| ProcessedTile {
                tile_index: t.tile_index,
                col: t.col,
                row: t.row,
                x_offset: t.x_offset,
                y_offset: t.y_offset,
                original_width: t.width,
                original_height: t.height,
                output_bytes: t.payload_bytes.clone(),
                output_sha256: t.payload_sha256,
                node_id: "test-node-1".to_string(),
                attempt: 1,
                execution_duration_ms: 10,
            })
            .collect();

        let composite = reassemble_image(&processed, 25, 15).expect("Reassembly must succeed");
        assert_eq!(composite.dimensions(), (25, 15));

        for y in 0..15 {
            for x in 0..25 {
                assert_eq!(composite.get_pixel(x, y), img.get_pixel(x, y));
            }
        }
    }
}
