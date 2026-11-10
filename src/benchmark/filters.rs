//! Core Image Processing Filter Kernels for Local Baseline and Verification
//!
//! Provides deterministic reference implementations of Sobel edge detection,
//! Gaussian blur, bilinear downscaling, and histogram equalization to benchmark
//! single-core baseline CPU throughput against distributed cluster execution.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-004], [NFR-PERF-001]

use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};

/// Supported image filtering and transformation operations.
///
/// Traces to: [REQ-OP-004]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BenchmarkFilter {
    /// 3x3 Sobel gradient magnitude edge detection.
    Sobel,
    /// 3x3 separable Gaussian smoothing filter.
    Blur,
    /// Bilinear interpolation 2x downscaling.
    Downscale,
    /// Luminance cumulative distribution function histogram equalization.
    Equalize,
    /// Grayscale conversion via ITU-R BT.601 luminance weights.
    Grayscale,
    /// Color inversion.
    Invert,
}

impl BenchmarkFilter {
    /// Parses an operation name from string.
    ///
    /// Traces to: [REQ-OP-004]
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "sobel" | "edge" | "edges" => Some(Self::Sobel),
            "blur" | "gaussian" | "gaussian_blur" => Some(Self::Blur),
            "downscale" | "downsample" | "resize" => Some(Self::Downscale),
            "equalize" | "histogram" | "hist_eq" => Some(Self::Equalize),
            "grayscale" | "greyscale" | "gray" => Some(Self::Grayscale),
            "invert" | "negative" => Some(Self::Invert),
            _ => None,
        }
    }

    /// Canonical CLI identifier name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sobel => "sobel",
            Self::Blur => "blur",
            Self::Downscale => "downscale",
            Self::Equalize => "equalize",
            Self::Grayscale => "grayscale",
            Self::Invert => "invert",
        }
    }
}

/// Converts an RGB pixel to its perceptual grayscale luminance (ITU-R BT.601).
///
/// Traces to: [REQ-OP-004]
#[inline]
pub fn rgb_to_luma(r: u8, g: u8, b: u8) -> u8 {
    let y = 0.299 * (r as f32) + 0.587 * (g as f32) + 0.114 * (b as f32);
    y.clamp(0.0, 255.0) as u8
}

/// Applies the selected filter operation to an RGBA image buffer.
///
/// Traces to: [INV-006], [REQ-OP-004]
///
/// # Arguments
/// * `img` - Source RGBA image buffer.
/// * `filter` - Target filter operation.
///
/// # Returns
/// * Transformed RGBA image buffer.
pub fn apply_filter(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    filter: BenchmarkFilter,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    match filter {
        BenchmarkFilter::Sobel => apply_sobel(img),
        BenchmarkFilter::Blur => apply_gaussian_blur_3x3(img),
        BenchmarkFilter::Downscale => apply_bilinear_downscale_2x(img),
        BenchmarkFilter::Equalize => apply_histogram_equalization(img),
        BenchmarkFilter::Grayscale => apply_grayscale(img),
        BenchmarkFilter::Invert => apply_invert(img),
    }
}

/// Applies a 3x3 Sobel edge detection filter.
///
/// Traces to: [INV-006], [REQ-OP-004]
pub fn apply_sobel(img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = img.dimensions();
    let mut output = ImageBuffer::new(width, height);

    if width < 3 || height < 3 {
        return img.clone();
    }

    let mut luma_grid = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x, y);
            luma_grid[(y * width + x) as usize] = rgb_to_luma(p[0], p[1], p[2]);
        }
    }

    let get_l = |x: i32, y: i32| -> i32 {
        let cx = x.clamp(0, width as i32 - 1) as u32;
        let cy = y.clamp(0, height as i32 - 1) as u32;
        luma_grid[(cy * width + cx) as usize] as i32
    };

    for y in 0..height {
        let iy = y as i32;
        for x in 0..width {
            let ix = x as i32;

            let gx = -get_l(ix - 1, iy - 1) + get_l(ix + 1, iy - 1) - 2 * get_l(ix - 1, iy)
                + 2 * get_l(ix + 1, iy)
                - get_l(ix - 1, iy + 1)
                + get_l(ix + 1, iy + 1);

            let gy = -get_l(ix - 1, iy - 1) - 2 * get_l(ix, iy - 1) - get_l(ix + 1, iy - 1)
                + get_l(ix - 1, iy + 1)
                + 2 * get_l(ix, iy + 1)
                + get_l(ix + 1, iy + 1);

            let mag = ((gx * gx + gy * gy) as f32).sqrt().clamp(0.0, 255.0) as u8;
            let orig_alpha = img.get_pixel(x, y)[3];
            output.put_pixel(x, y, Rgba([mag, mag, mag, orig_alpha]));
        }
    }

    output
}

/// Applies a 3x3 separable Gaussian smoothing filter.
///
/// Traces to: [INV-006], [REQ-OP-004]
pub fn apply_gaussian_blur_3x3(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = img.dimensions();
    if width < 3 || height < 3 {
        return img.clone();
    }

    // Horizontal pass
    let mut temp = vec![[0u16; 4]; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let mut acc = [0u32; 4];
            for k in -1i32..=1i32 {
                let cx = (x as i32 + k).clamp(0, width as i32 - 1) as u32;
                let p = img.get_pixel(cx, y);
                let weight = if k == 0 { 2u32 } else { 1u32 };
                for c in 0..4 {
                    acc[c] += (p[c] as u32) * weight;
                }
            }
            let idx = (y * width + x) as usize;
            for c in 0..4 {
                temp[idx][c] = (acc[c] / 4) as u16;
            }
        }
    }

    // Vertical pass
    let mut output = ImageBuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut acc = [0u32; 4];
            for k in -1i32..=1i32 {
                let cy = (y as i32 + k).clamp(0, height as i32 - 1) as u32;
                let idx = (cy * width + x) as usize;
                let weight = if k == 0 { 2u32 } else { 1u32 };
                for c in 0..4 {
                    acc[c] += (temp[idx][c] as u32) * weight;
                }
            }
            let final_pixel = Rgba([
                (acc[0] / 4).min(255) as u8,
                (acc[1] / 4).min(255) as u8,
                (acc[2] / 4).min(255) as u8,
                (acc[3] / 4).min(255) as u8,
            ]);
            output.put_pixel(x, y, final_pixel);
        }
    }

    output
}

/// Applies bilinear 2x downscaling.
///
/// Traces to: [INV-006], [REQ-OP-004]
pub fn apply_bilinear_downscale_2x(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (src_w, src_h) = img.dimensions();
    let dst_w = (src_w / 2).max(1);
    let dst_h = (src_h / 2).max(1);
    let mut output = ImageBuffer::new(dst_w, dst_h);

    let x_ratio = src_w as f32 / dst_w as f32;
    let y_ratio = src_h as f32 / dst_h as f32;

    for dy in 0..dst_h {
        let sy = (dy as f32 + 0.5) * y_ratio - 0.5;
        let y0 = sy.floor().clamp(0.0, (src_h - 1) as f32) as u32;
        let y1 = (y0 + 1).min(src_h - 1);
        let fy = sy - sy.floor();

        for dx in 0..dst_w {
            let sx = (dx as f32 + 0.5) * x_ratio - 0.5;
            let x0 = sx.floor().clamp(0.0, (src_w - 1) as f32) as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let fx = sx - sx.floor();

            let p00 = img.get_pixel(x0, y0);
            let p10 = img.get_pixel(x1, y0);
            let p01 = img.get_pixel(x0, y1);
            let p11 = img.get_pixel(x1, y1);

            let mut out_channels = [0u8; 4];
            for c in 0..4 {
                let top = (p00[c] as f32) * (1.0 - fx) + (p10[c] as f32) * fx;
                let bottom = (p01[c] as f32) * (1.0 - fx) + (p11[c] as f32) * fx;
                let val = top * (1.0 - fy) + bottom * fy;
                out_channels[c] = val.round().clamp(0.0, 255.0) as u8;
            }

            output.put_pixel(dx, dy, Rgba(out_channels));
        }
    }

    output
}

/// Applies cumulative distribution function (CDF) histogram equalization across luminance.
///
/// Traces to: [INV-006], [REQ-OP-004]
pub fn apply_histogram_equalization(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = img.dimensions();
    let total_pixels = width * height;
    if total_pixels == 0 {
        return img.clone();
    }

    let mut hist = [0u32; 256];
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x, y);
            let luma = rgb_to_luma(p[0], p[1], p[2]);
            hist[luma as usize] += 1;
        }
    }

    let mut cdf = [0u32; 256];
    let mut sum = 0u32;
    for (i, slot) in cdf.iter_mut().enumerate() {
        sum += hist[i];
        *slot = sum;
    }

    let cdf_min = cdf.iter().copied().find(|&c| c > 0).unwrap_or(0);
    let denominator = (total_pixels - cdf_min) as f32;

    let mut map = [0u8; 256];
    if denominator > 0.0 {
        for (i, slot) in map.iter_mut().enumerate() {
            if cdf[i] >= cdf_min {
                let v = (((cdf[i] - cdf_min) as f32 / denominator) * 255.0).round();
                *slot = v.clamp(0.0, 255.0) as u8;
            }
        }
    } else {
        for (i, slot) in map.iter_mut().enumerate() {
            *slot = i as u8;
        }
    }

    let mut output = ImageBuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x, y);
            let orig_l = rgb_to_luma(p[0], p[1], p[2]);
            let new_l = map[orig_l as usize];

            if orig_l == 0 {
                output.put_pixel(x, y, Rgba([new_l, new_l, new_l, p[3]]));
            } else {
                let ratio = (new_l as f32) / (orig_l as f32);
                let r = ((p[0] as f32) * ratio).clamp(0.0, 255.0) as u8;
                let g = ((p[1] as f32) * ratio).clamp(0.0, 255.0) as u8;
                let b = ((p[2] as f32) * ratio).clamp(0.0, 255.0) as u8;
                output.put_pixel(x, y, Rgba([r, g, b, p[3]]));
            }
        }
    }

    output
}

/// Applies grayscale conversion.
///
/// Traces to: [REQ-OP-004]
pub fn apply_grayscale(img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = img.dimensions();
    let mut output = ImageBuffer::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x, y);
            let y_val = rgb_to_luma(p[0], p[1], p[2]);
            output.put_pixel(x, y, Rgba([y_val, y_val, y_val, p[3]]));
        }
    }

    output
}

/// Inverts color channels while preserving alpha.
///
/// Traces to: [REQ-OP-004]
pub fn apply_invert(img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = img.dimensions();
    let mut output = ImageBuffer::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x, y);
            output.put_pixel(x, y, Rgba([255 - p[0], 255 - p[1], 255 - p[2], p[3]]));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_filter_string_round_trip() {
        let filters = [
            (BenchmarkFilter::Sobel, "sobel"),
            (BenchmarkFilter::Blur, "blur"),
            (BenchmarkFilter::Downscale, "downscale"),
            (BenchmarkFilter::Equalize, "equalize"),
            (BenchmarkFilter::Grayscale, "grayscale"),
            (BenchmarkFilter::Invert, "invert"),
        ];

        for (filter, name) in filters {
            assert_eq!(filter.as_str(), name);
            assert_eq!(BenchmarkFilter::from_str_name(name), Some(filter));
        }
    }
}
