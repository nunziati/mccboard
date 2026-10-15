//! Draws the tray icon at runtime: the percentage in a blocky pixel font on a rounded square
//! in the account's color. The digits are integer-scaled 3×5 glyphs, so they stay crisp when
//! the shell scales the 32-pixel icon down to 16. Without a percentage it shows a dash, or
//! three dots while an account's access token waits for Claude Code to renew it.

use mccboard_core::TrayIcon;

/// The icon's side in pixels.
pub(crate) const SIZE: u32 = 32;

const GLYPH_W: usize = 3;
const GLYPH_H: usize = 5;
const CORNER_RADIUS: f32 = 6.0;
/// What a stale reading's color is mixed with, half and half.
const FADE_TO: [u8; 3] = [0x9c, 0xa3, 0xaf];

/// The icon's pixels, RGBA, `SIZE`×`SIZE`, row by row.
pub(crate) fn render(icon: &TrayIcon) -> Vec<u8> {
    let mut background = parse_hex(icon.color.as_str());
    if icon.stale {
        for (channel, fade) in background.iter_mut().zip(FADE_TO) {
            *channel = ((u16::from(*channel) + u16::from(fade)) / 2) as u8;
        }
    }
    // Whichever of black and white contrasts more with the background.
    let foreground = if luminance(background) > 0.179 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    };

    let size = SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            if inside_rounded_square(x, y, size) {
                put(&mut rgba, x, y, background);
            }
        }
    }

    let text = match icon.percent {
        Some(percent) => percent.to_string(),
        // An expired token isn't an alarm: Claude Code renews it when it runs. Three dots
        // say "waiting for it".
        None if icon.token_expired => "···".to_owned(),
        None => "-".to_owned(),
    };
    let glyphs: Vec<&[u8; GLYPH_H]> = text.chars().filter_map(glyph).collect();
    let (scale, gap) = if glyphs.len() <= 2 { (4, 4) } else { (3, 2) };
    let width = glyphs.len() * GLYPH_W * scale + glyphs.len().saturating_sub(1) * gap;
    let x0 = (size - width.min(size)) / 2;
    let y0 = (size - GLYPH_H * scale) / 2;
    for (index, rows) in glyphs.iter().enumerate() {
        let left = x0 + index * (GLYPH_W * scale + gap);
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..GLYPH_W {
                if bits & (0b100 >> column) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (x, y) = (left + column * scale + dx, y0 + row * scale + dy);
                        if x < size && y < size {
                            put(&mut rgba, x, y, foreground);
                        }
                    }
                }
            }
        }
    }
    rgba
}

fn put(rgba: &mut [u8], x: usize, y: usize, [r, g, b]: [u8; 3]) {
    let at = (y * SIZE as usize + x) * 4;
    rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
}

fn inside_rounded_square(x: usize, y: usize, size: usize) -> bool {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let max = size as f32;
    let cx = px.clamp(CORNER_RADIUS, max - CORNER_RADIUS);
    let cy = py.clamp(CORNER_RADIUS, max - CORNER_RADIUS);
    (px - cx).powi(2) + (py - cy).powi(2) <= CORNER_RADIUS.powi(2)
}

/// `#rrggbb` (the core validates account colors) as RGB; gray if it can't be read.
fn parse_hex(color: &str) -> [u8; 3] {
    let hex = color.trim_start_matches('#');
    let channel = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|c| u8::from_str_radix(c, 16).ok())
    };
    match (channel(0), channel(2), channel(4)) {
        (Some(r), Some(g), Some(b)) => [r, g, b],
        _ => [0x6b, 0x72, 0x80],
    }
}

/// Relative luminance, 0 (black) to 1 (white).
fn luminance([r, g, b]: [u8; 3]) -> f32 {
    let linear = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// 3×5 glyphs, one row per byte, most significant of the three bits on the left.
fn glyph(c: char) -> Option<&'static [u8; GLYPH_H]> {
    Some(match c {
        '0' => &[0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => &[0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => &[0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => &[0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => &[0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => &[0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => &[0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => &[0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => &[0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => &[0b111, 0b101, 0b111, 0b001, 0b111],
        '-' => &[0b000, 0b000, 0b111, 0b000, 0b000],
        // A bold dot, so three of them stay visible at 16 pixels.
        '·' => &[0b000, 0b000, 0b011, 0b011, 0b000],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(percent: Option<u8>, color: &str, stale: bool) -> TrayIcon {
        TrayIcon {
            percent,
            color: color.parse().unwrap(),
            account: None,
            stale,
            token_expired: false,
        }
    }

    fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
        let at = (y * SIZE as usize + x) * 4;
        rgba[at..at + 4].try_into().unwrap()
    }

    fn count(rgba: &[u8], color: [u8; 4]) -> usize {
        rgba.chunks(4).filter(|p| *p == color).count()
    }

    #[test]
    fn the_icon_is_a_square_of_rgba_pixels_in_the_account_color() {
        let rgba = render(&icon(Some(42), "#2563eb", false));
        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(pixel(&rgba, 16, 1), [0x25, 0x63, 0xeb, 255]);
        // Rounded corners are transparent.
        assert_eq!(pixel(&rgba, 0, 0)[3], 0);
        // White digits on a dark color.
        assert!(count(&rgba, [255, 255, 255, 255]) > 100);
    }

    #[test]
    fn light_colors_get_dark_digits() {
        let rgba = render(&icon(Some(42), "#ca8a04", false));
        assert!(count(&rgba, [0, 0, 0, 255]) > 100);
    }

    #[test]
    fn different_percentages_draw_differently_and_100_fits() {
        let a = render(&icon(Some(18), "#2563eb", false));
        let b = render(&icon(Some(81), "#2563eb", false));
        assert_ne!(a, b);
        let full = render(&icon(Some(100), "#2563eb", false));
        assert_ne!(full, render(&icon(Some(10), "#2563eb", false)));
        // Three glyphs of 5 cells by 3 pixels tall, every cell drawn on the background.
        let cells_per_glyph = [8, 12, 12];
        let cells: usize = cells_per_glyph.iter().sum();
        assert_eq!(count(&full, [255, 255, 255, 255]), cells * 9);
    }

    #[test]
    fn a_stale_reading_is_faded() {
        let fresh = render(&icon(Some(42), "#2563eb", false));
        let stale = render(&icon(Some(42), "#2563eb", true));
        assert_ne!(pixel(&fresh, 16, 1), pixel(&stale, 16, 1));
    }

    #[test]
    fn an_expired_token_without_a_percentage_draws_three_dots() {
        let mut expired = icon(None, "#6b7280", false);
        expired.token_expired = true;
        let rgba = render(&expired);
        assert_ne!(rgba, render(&icon(None, "#6b7280", false)));
        // Three dots of 2×2 cells, scaled by 3.
        assert_eq!(count(&rgba, [255, 255, 255, 255]), 3 * 4 * 9);
    }

    #[test]
    fn no_percentage_draws_a_dash() {
        let rgba = render(&icon(None, "#6b7280", false));
        // Only the dash's 3×1 cells, scaled by 4.
        assert_eq!(count(&rgba, [255, 255, 255, 255]), 3 * 4 * 4);
    }
}
