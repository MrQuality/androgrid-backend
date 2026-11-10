//! Image Processing Algorithms for WASI Compute Workload
//!
//! Implements pure-Rust, zero-native-dependency image transformation kernels
//! executing within the WebAssembly/WASI unrooted sandbox.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-004], [NFR-PERF-001]

use image::{ImageBuffer, Rgba};

/// Supported image filtering and transformation operations.
///
/// Traces to: [REQ-OP-004]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOperation {
    /// 3x3 Sobel gradient magnitude edge detection.
    Sobel,
    /// 3x3 separable Gaussian smoothing filter.
    Blur,
    /// Bilinear interpolation 2x downscaling.
    Downscale,
    /// Luminance cumulative distribution function histogram equalization.
    Equalize,
    /// Grayscale conversion via standard ITU-R BT.601 luminance weights.
    Grayscale,
    /// Color inversion.
    Invert,
}

impl FilterOperation {
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
}

/// Applies the selected filter operation to an RGBA image buffer.
///
/// Traces to: [INV-006], [REQ-OP-004]
///
/// # Arguments
/// * `img` - Source RGBA image buffer.
/// * `op` - Target filter operation.
///
/// # Returns
/// * Transformed RGBA image buffer.
pub fn apply_filter(
    img: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    op: FilterOperation,
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    match op {
        FilterOperation::Sobel => apply_sobel(img),
        FilterOperation::Blur => apply_gaussian_blur_3x3(img),
        FilterOperation::Downscale => apply_bilinear_downscale_2x(img),
        FilterOperation::Equalize => apply_histogram_equalization(img),
        FilterOperation::Grayscale => apply_grayscale(img),
        FilterOperation::Invert => apply_invert(img),
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

/// Applies a 3x3 Sobel edge detection filter.
///
/// Computes horizontal and vertical gradient approximations:
/// Gx = [[-1, 0, 1], [-2, 0, 2], [-1, 0, 1]]
/// Gy = [[-1, -2, -1], [0, 0, 0], [1, 2, 1]]
/// Magnitude = clamp(sqrt(Gx^2 + Gy^2), 0, 255)
///
/// Traces to: [INV-006], [REQ-OP-004]
pub fn apply_sobel(img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let (width, height) = img.dimensions();
    let mut output = ImageBuffer::new(width, height);

    if width < 3 || height < 3 {
        return img.clone();
    }

    // Pre-calculate luminance channel for performance
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
/// Uses kernel [1, 2, 1] / 4 in both horizontal and vertical passes.
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

    // Calculate luminance histogram
    let mut hist = [0u32; 256];
    for y in 0..height {
        for x in 0..width {
            let p = img.get_pixel(x, y);
            let luma = rgb_to_luma(p[0], p[1], p[2]);
            hist[luma as usize] += 1;
        }
    }

    // Cumulative Distribution Function (CDF)
    let mut cdf = [0u32; 256];
    let mut sum = 0u32;
    for (i, slot) in cdf.iter_mut().enumerate() {
        sum += hist[i];
        *slot = sum;
    }

    let cdf_min = cdf.iter().copied().find(|&c| c > 0).unwrap_or(0);
    let denominator = (total_pixels - cdf_min) as f32;

    // Mapping table
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

    // Apply equalization to luminance while preserving color ratios
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
    fn test_filter_operation_from_str() {
        assert_eq!(
            FilterOperation::from_str_name("sobel"),
            Some(FilterOperation::Sobel)
        );
        assert_eq!(
            FilterOperation::from_str_name("edge"),
            Some(FilterOperation::Sobel)
        );
        assert_eq!(
            FilterOperation::from_str_name("blur"),
            Some(FilterOperation::Blur)
        );
        assert_eq!(
            FilterOperation::from_str_name("downscale"),
            Some(FilterOperation::Downscale)
        );
        assert_eq!(
            FilterOperation::from_str_name("equalize"),
            Some(FilterOperation::Equalize)
        );
        assert_eq!(
            FilterOperation::from_str_name("grayscale"),
            Some(FilterOperation::Grayscale)
        );
        assert_eq!(
            FilterOperation::from_str_name("invert"),
            Some(FilterOperation::Invert)
        );
        assert_eq!(FilterOperation::from_str_name("unknown"), None);
    }

    #[test]
    fn test_invert_filter() {
        let mut img = ImageBuffer::new(2, 2);
        img.put_pixel(0, 0, Rgba([100, 150, 200, 255]));
        let inverted = apply_invert(&img);
        let p = inverted.get_pixel(0, 0);
        assert_eq!(p[0], 155);
        assert_eq!(p[1], 105);
        assert_eq!(p[2], 55);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn test_bilinear_downscale() {
        let mut img = ImageBuffer::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.put_pixel(x, y, Rgba([200, 100, 50, 255]));
            }
        }
        let downscaled = apply_bilinear_downscale_2x(&img);
        assert_eq!(downscaled.dimensions(), (2, 2));
        let p = downscaled.get_pixel(0, 0);
        assert_eq!(p[0], 200);
        assert_eq!(p[1], 100);
        assert_eq!(p[2], 50);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn test_gaussian_blur_smoothing() {
        let mut img = ImageBuffer::new(5, 5);
        for y in 0..5 {
            for x in 0..5 {
                img.put_pixel(x, y, Rgba([10, 10, 10, 255]));
            }
        }
        img.put_pixel(2, 2, Rgba([250, 250, 250, 255])); // Center impulse
        let blurred = apply_gaussian_blur_3x3(&img);
        assert_eq!(blurred.dimensions(), (5, 5));
        // Center pixel should be attenuated by Gaussian kernel
        let center = blurred.get_pixel(2, 2);
        assert!(center[0] < 250);
        assert!(center[0] > 10);
    }

    #[test]
    fn test_sobel_edge_detection() {
        let mut img = ImageBuffer::new(6, 6);
        // Vertical step edge: left half black, right half white
        for y in 0..6 {
            for x in 0..3 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
            for x in 3..6 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let edges = apply_sobel(&img);
        assert_eq!(edges.dimensions(), (6, 6));
        // Edge boundary pixels at x=2 and x=3 should have high gradient magnitude
        let edge_p = edges.get_pixel(2, 3);
        assert!(edge_p[0] > 100);
        // Flat region at x=0 should have near-zero gradient
        let flat_p = edges.get_pixel(0, 3);
        assert_eq!(flat_p[0], 0);
    }

    #[test]
    fn test_histogram_equalization() {
        let mut img = ImageBuffer::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                // Low contrast image in range 50..65
                let val = (50 + x * 4) as u8;
                img.put_pixel(x, y, Rgba([val, val, val, 255]));
            }
        }
        let equalized = apply_histogram_equalization(&img);
        assert_eq!(equalized.dimensions(), (4, 4));
        let p_min = equalized.get_pixel(0, 0);
        let p_max = equalized.get_pixel(3, 3);
        // Equalization expands dynamic range across 0..255
        assert!(p_max[0] > p_min[0]);
    }
}
