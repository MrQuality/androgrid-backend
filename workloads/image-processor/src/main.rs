//! Canonical WASI Image Processing Workload
//!
//! Demonstrates high-throughput parallel image processing on distributed Android
//! worker nodes executing within the unrooted WebAssembly/WASI sandbox.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-004], [NFR-PERF-001]

mod filters;

use filters::{apply_filter, FilterOperation};
use image::{ImageBuffer, ImageFormat, Rgba};
use std::env;
use std::io::{self, Cursor, Read, Write};
use std::time::Instant;

/// Maximum payload size allowable per input tile chunk (16 MB).
///
/// Traces to: [NFR-PERF-001]
pub const MAX_TILE_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Raw image tile header magic identifier: b"AGRAW"
pub const RAW_MAGIC: &[u8; 5] = b"AGRAW";

/// Output format for processed image tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileFormat {
    Png,
    RawRgba,
}

/// RGBA pixel buffer alias.
pub type RgbaTileBuffer = ImageBuffer<Rgba<u8>, Vec<u8>>;

/// Result type for tile decoding operations.
pub type TileDecodeResult = Result<(RgbaTileBuffer, TileFormat), Box<dyn std::error::Error>>;

/// Reads the image from byte stream, detecting PNG or AGRAW wire format.
///
/// Traces to: [REQ-OP-004], [NFR-PERF-001]
pub fn decode_tile(bytes: &[u8]) -> TileDecodeResult {
    if bytes.len() > MAX_TILE_PAYLOAD_BYTES {
        return Err(format!(
            "Input payload of {} bytes exceeds 16 MB frame limit",
            bytes.len()
        )
        .into());
    }

    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let dyn_img = image::load_from_memory_with_format(bytes, ImageFormat::Png)?;
        Ok((dyn_img.to_rgba8(), TileFormat::Png))
    } else if bytes.starts_with(RAW_MAGIC) {
        if bytes.len() < 14 {
            return Err("AGRAW buffer truncated: missing header fields".into());
        }
        let width = u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
        let height = u32::from_be_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]);
        let channels = bytes[13];

        let expected_data_len = (width as usize) * (height as usize) * (channels as usize);
        if bytes.len() < 14 + expected_data_len {
            return Err("AGRAW buffer truncated: missing pixel payload".into());
        }

        let pixel_bytes = &bytes[14..14 + expected_data_len];
        let mut img = ImageBuffer::new(width, height);

        match channels {
            3 => {
                let mut idx = 0;
                for y in 0..height {
                    for x in 0..width {
                        let r = pixel_bytes[idx];
                        let g = pixel_bytes[idx + 1];
                        let b = pixel_bytes[idx + 2];
                        img.put_pixel(x, y, Rgba([r, g, b, 255]));
                        idx += 3;
                    }
                }
            }
            4 => {
                let mut idx = 0;
                for y in 0..height {
                    for x in 0..width {
                        let r = pixel_bytes[idx];
                        let g = pixel_bytes[idx + 1];
                        let b = pixel_bytes[idx + 2];
                        let a = pixel_bytes[idx + 3];
                        img.put_pixel(x, y, Rgba([r, g, b, a]));
                        idx += 4;
                    }
                }
            }
            other => return Err(format!("Unsupported AGRAW channel count: {other}").into()),
        }

        Ok((img, TileFormat::RawRgba))
    } else {
        // Fallback: attempt general image decoding
        let dyn_img = image::load_from_memory(bytes)?;
        Ok((dyn_img.to_rgba8(), TileFormat::Png))
    }
}

/// Encodes the processed image tile to bytes.
///
/// Traces to: [REQ-OP-004], [NFR-PERF-001]
pub fn encode_tile(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    format: TileFormat,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match format {
        TileFormat::Png => {
            let mut buf = Vec::new();
            let mut cursor = Cursor::new(&mut buf);
            img.write_to(&mut cursor, ImageFormat::Png)?;
            Ok(buf)
        }
        TileFormat::RawRgba => {
            let (w, h) = img.dimensions();
            let mut buf = Vec::with_capacity(14 + (w * h * 4) as usize);
            buf.extend_from_slice(RAW_MAGIC);
            buf.extend_from_slice(&w.to_be_bytes());
            buf.extend_from_slice(&h.to_be_bytes());
            buf.push(4u8); // 4 channels: RGBA
            for p in img.pixels() {
                buf.extend_from_slice(&p.0);
            }
            Ok(buf)
        }
    }
}

/// Main entry point for the WASI image processing workload.
///
/// Traces to: [INV-006], [REQ-OP-004], [NFR-PERF-001]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start_time = Instant::now();

    // Read filter configuration from environment variable
    let filter_name = env::var("OPERATION")
        .or_else(|_| env::var("FILTER"))
        .unwrap_or_else(|_| "sobel".to_string());

    let op = FilterOperation::from_str_name(&filter_name).unwrap_or(FilterOperation::Sobel);

    // Read input image payload from stdin
    let mut stdin_bytes = Vec::new();
    io::stdin().read_to_end(&mut stdin_bytes)?;

    if stdin_bytes.is_empty() {
        eprintln!("[image-processor] Warning: received empty payload on stdin");
        return Ok(());
    }

    let (input_img, detected_format) = decode_tile(&stdin_bytes)?;
    let (src_w, src_h) = input_img.dimensions();

    let output_img = apply_filter(&input_img, op);
    let (dst_w, dst_h) = output_img.dimensions();

    // Format override via env var
    let out_format = match env::var("OUTPUT_FORMAT").ok().as_deref() {
        Some("raw") => TileFormat::RawRgba,
        Some("png") => TileFormat::Png,
        _ => detected_format,
    };

    let encoded_output = encode_tile(&output_img, out_format)?;
    io::stdout().write_all(&encoded_output)?;
    io::stdout().flush()?;

    let elapsed_micros = start_time.elapsed().as_micros();
    eprintln!(
        "[image-processor] Completed {:?} on {}x{} -> {}x{} in {} micros (payload: {} bytes)",
        op,
        src_w,
        src_h,
        dst_w,
        dst_h,
        elapsed_micros,
        encoded_output.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_format_encode_decode_round_trip() {
        let mut img = ImageBuffer::new(2, 2);
        img.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
        img.put_pixel(1, 0, Rgba([40, 50, 60, 255]));
        img.put_pixel(0, 1, Rgba([70, 80, 90, 255]));
        img.put_pixel(1, 1, Rgba([100, 110, 120, 255]));

        let encoded = encode_tile(&img, TileFormat::RawRgba).expect("Encoding must succeed");
        assert!(encoded.starts_with(RAW_MAGIC));

        let (decoded, fmt) = decode_tile(&encoded).expect("Decoding must succeed");
        assert_eq!(fmt, TileFormat::RawRgba);
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.get_pixel(1, 1), &Rgba([100, 110, 120, 255]));
    }

    #[test]
    fn test_png_format_encode_decode_round_trip() {
        let mut img = ImageBuffer::new(3, 3);
        for y in 0..3 {
            for x in 0..3 {
                img.put_pixel(x, y, Rgba([50, 100, 150, 255]));
            }
        }

        let encoded = encode_tile(&img, TileFormat::Png).expect("PNG encode must succeed");
        assert!(encoded.starts_with(b"\x89PNG\r\n\x1a\n"));

        let (decoded, fmt) = decode_tile(&encoded).expect("PNG decode must succeed");
        assert_eq!(fmt, TileFormat::Png);
        assert_eq!(decoded.dimensions(), (3, 3));
    }

    #[test]
    fn test_frame_size_upper_boundary_rejected() {
        let oversized = vec![0u8; MAX_TILE_PAYLOAD_BYTES + 1];
        let res = decode_tile(&oversized);
        assert!(res.is_err());
    }
}
