//! Pixel operations on RGBA8 images: trimming, resampling, recoloring and the tray mark.

use super::Image;

/// Pixels at or below this alpha don't count as visible when trimming.
const VISIBLE_ALPHA: u8 = 16;

/// Pixels at or above this HSL saturation are recolored; below it (white, grey, the
/// anti-aliased edges between white and orange) they stay.
const SATURATED: f32 = 0.3;

/// Claude's orange `#D97757` (L ≈ 0.596): recolored pixels keep their lightness offset from it.
const CLAUDE_ORANGE: [u8; 3] = [0xd9, 0x77, 0x57];

/// The bounding box of the pixels with alpha above [`VISIBLE_ALPHA`], centred in a
/// transparent square of side max(width, height): no margin. A fully transparent image gives
/// a 1×1 transparent one.
pub(super) fn trim_square(image: &Image) -> Image {
    let (w, h) = (image.width as usize, image.height as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if image.rgba[(y * w + x) * 4 + 3] > VISIBLE_ALPHA {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 {
        return Image::new(1, 1, vec![0; 4]);
    }
    let side = (x1 - x0).max(y1 - y0);
    let (ox, oy) = ((side - (x1 - x0)) / 2, (side - (y1 - y0)) / 2);
    let mut out = vec![0u8; side * side * 4];
    for y in y0..y1 {
        let src = &image.rgba[(y * w + x0) * 4..(y * w + x1) * 4];
        let at = ((y - y0 + oy) * side + ox) * 4;
        out[at..at + src.len()].copy_from_slice(src);
    }
    Image::new(side as u32, side as u32, out)
}

/// A square image resampled to `side`×`side`: each output pixel is the area-weighted average
/// of the source pixels under it, in premultiplied alpha (so transparent pixels lend no
/// color). Works both ways: down to 16 px and slightly up to 256.
pub(super) fn resize(image: &Image, side: u32) -> Image {
    assert_eq!(image.width, image.height, "resize takes a square image");
    let (s, d) = (image.width as usize, side as usize);
    let scale = s as f64 / d as f64;
    // The source cells [i, i + 1) under the output cell [o, o + 1), with their overlap.
    let spans: Vec<Vec<(usize, f64)>> = (0..d)
        .map(|o| {
            let (a, b) = (o as f64 * scale, (o + 1) as f64 * scale);
            (a.floor() as usize..(b.ceil() as usize).min(s))
                .map(|i| (i, (b.min(i as f64 + 1.0) - a.max(i as f64)).max(0.0)))
                .collect()
        })
        .collect();
    let mut out = vec![0u8; d * d * 4];
    for (oy, rows) in spans.iter().enumerate() {
        for (ox, cols) in spans.iter().enumerate() {
            let mut acc = [0f64; 4];
            let mut total = 0f64;
            for &(sy, wy) in rows {
                for &(sx, wx) in cols {
                    let p = &image.rgba[(sy * s + sx) * 4..][..4];
                    let weight = wx * wy;
                    let alpha = p[3] as f64 / 255.0;
                    for c in 0..3 {
                        acc[c] += p[c] as f64 * alpha * weight;
                    }
                    acc[3] += alpha * weight;
                    total += weight;
                }
            }
            let px = &mut out[(oy * d + ox) * 4..][..4];
            if acc[3] > 0.0 {
                for c in 0..3 {
                    px[c] = (acc[c] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                px[3] = (acc[3] / total * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image::new(side, side, out)
}

/// Saturated pixels (s ≥ 0.3) take `color`'s hue and saturation, with `color`'s lightness
/// moved by the pixel's lightness offset from Claude's orange. White, grey and alpha stay.
pub(super) fn recolor(image: &mut Image, color: [u8; 3]) {
    let (hue, saturation, lightness) = rgb_to_hsl(color);
    let (_, _, orange) = rgb_to_hsl(CLAUDE_ORANGE);
    for p in image.rgba.as_chunks_mut::<4>().0.iter_mut() {
        if p[3] == 0 {
            continue;
        }
        let (_, s, l) = rgb_to_hsl([p[0], p[1], p[2]]);
        if s >= SATURATED {
            let [r, g, b] = hsl_to_rgb(hue, saturation, (lightness + (l - orange)).clamp(0.0, 1.0));
            p[..3].copy_from_slice(&[r, g, b]);
        }
    }
}

/// Where the mark is in a source image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkIn {
    /// Its visible pixels: the unplated logo, the tray icon.
    Alpha,
    /// Its near-white pixels: the plated logo, the mark drawn on its tile.
    White,
}

/// Only the mark of `image`, every pixel `color` with the mark's coverage as alpha, on a
/// transparent background.
pub(super) fn mark(image: &Image, mark_in: MarkIn, color: [u8; 3]) -> Image {
    let mut out = Vec::with_capacity(image.rgba.len());
    for p in image.rgba.as_chunks::<4>().0 {
        let coverage = match mark_in {
            MarkIn::Alpha => 1.0,
            MarkIn::White => whiteness([p[0], p[1], p[2]]),
        };
        let alpha = (p[3] as f32 * coverage).round() as u8;
        if alpha == 0 {
            out.extend([0; 4]);
        } else {
            out.extend([color[0], color[1], color[2], alpha]);
        }
    }
    Image::new(image.width, image.height, out)
}

/// How white a pixel is, 0 to 1, from its darkest channel: Claude's orange tile (darkest
/// channel 0.34) is 0, white is 1, and the anti-aliased edge between them in between.
fn whiteness(rgb: [u8; 3]) -> f32 {
    let darkest = rgb.into_iter().min().unwrap_or(0) as f32 / 255.0;
    ((darkest - 0.55) / (0.95 - 0.55)).clamp(0.0, 1.0)
}

/// (hue in degrees, saturation, lightness), each of the last two 0 to 1.
fn rgb_to_hsl([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let sector = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (sector * 60.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_orange_in_hsl() {
        let (h, s, l) = rgb_to_hsl(CLAUDE_ORANGE);
        assert!((h - 15.0).abs() < 1.0, "{h}");
        assert!((s - 0.63).abs() < 0.01, "{s}");
        assert!((l - 0.596).abs() < 0.001, "{l}");
    }

    #[test]
    fn hsl_round_trip() {
        for rgb in [
            [0x25, 0x63, 0xeb],
            [0x16, 0xa3, 0x4a],
            [255, 0, 0],
            [12, 200, 99],
        ] {
            let (h, s, l) = rgb_to_hsl(rgb);
            assert_eq!(hsl_to_rgb(h, s, l), rgb);
        }
    }

    #[test]
    fn whiteness_of_the_tile_the_mark_and_their_edge() {
        assert_eq!(whiteness(CLAUDE_ORANGE), 0.0);
        assert_eq!(whiteness([255, 255, 255]), 1.0);
        let edge = whiteness([236, 187, 171]);
        assert!(edge > 0.0 && edge < 1.0);
    }
}
