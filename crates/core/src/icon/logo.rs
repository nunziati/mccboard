//! mccboard's logo (#45), "L4": a lowercase geometric `m`, three stems and two arches, drawn as
//! a track and filled solid below about half its height, like a usage window filling up.
//!
//! The mark is defined on a 24-unit grid, as the SVG path
//! `M4 20V10a4 4 0 0 1 8 0v10M12 10a4 4 0 0 1 8 0v10` stroked with butt caps, filled below
//! y = 12.5. Two styles:
//! - **Tray**: the mark alone, monochrome like the system's icons: white on the dark taskbar,
//!   near-black on the light one. The tray shows it while the taskbar widget is shown.
//! - **App**: the white mark on a graphite rounded tile (`#1f2430`): the window, installer and
//!   Start-menu icon, rendered once into `src-tauri/icons` (`examples/app_icon.rs`).
//!
//! Drawn by sampling each pixel on an 8×8 grid, so it is crisp at 16 px and smooth at 1024.

use super::Image;
use crate::TaskbarTheme;

/// The logo's two styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoStyle {
    /// The mark alone, for the tray of a taskbar with this theme.
    Tray(TaskbarTheme),
    /// The white mark on the graphite tile.
    App,
}

/// The app icon's tile.
const TILE: [u8; 3] = [0x1f, 0x24, 0x30];
/// The mark's color above its fill line, as a fraction of the ink's opacity.
const TRACK_ALPHA: f32 = 0.35;
/// On the light taskbar the track is a little lighter.
const LIGHT_TRACK_ALPHA: f32 = 0.3;
/// The fill line on the 24-unit grid: the mark is solid below it.
const FILL_LINE: f32 = 12.5;
/// Samples per pixel side.
const SAMPLES: u32 = 8;

/// mccboard's logo in `style`, `size × size` pixels.
pub fn mccboard_logo(size: u32, style: LogoStyle) -> Image {
    let side = size as f32;
    // Small icons get a thicker stroke and almost no margin, so the mark stays legible.
    let small = size <= 20;
    let stroke = if small { 3.9 } else { 3.4 };
    let (ink, track_alpha, pad) = match style {
        LogoStyle::Tray(TaskbarTheme::Dark) => {
            ([255; 3], TRACK_ALPHA, if small { 0.02 } else { 0.06 })
        }
        LogoStyle::Tray(TaskbarTheme::Light) => (
            [0x1b; 3],
            LIGHT_TRACK_ALPHA,
            if small { 0.02 } else { 0.06 },
        ),
        LogoStyle::App => ([255; 3], TRACK_ALPHA, 0.2),
    };
    let scale = side * (1.0 - 2.0 * pad) / 24.0;
    let offset = side * pad;

    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // Premultiplied sums over the samples.
            let mut sum = [0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    let mut color = [0f32; 4];
                    if style == LogoStyle::App && in_tile(px, py, side) {
                        color = over(color, TILE, 1.0);
                    }
                    let (gx, gy) = ((px - offset) / scale, (py - offset) / scale);
                    if in_mark(gx, gy, stroke) {
                        let alpha = if gy >= FILL_LINE { 1.0 } else { track_alpha };
                        color = over(color, ink, alpha);
                    }
                    for (total, channel) in sum.iter_mut().zip(color) {
                        *total += channel;
                    }
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let alpha = sum[3] / n;
            let channel = |c: f32| {
                if alpha > 0.0 {
                    (c / n / alpha * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            rgba.extend([
                channel(sum[0]),
                channel(sum[1]),
                channel(sum[2]),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
    Image::new(size, size, rgba)
}

/// `rgb` at `alpha` over `below`, both premultiplied, channels 0–1.
fn over(below: [f32; 4], rgb: [u8; 3], alpha: f32) -> [f32; 4] {
    let top = |i: usize| f32::from(rgb[i]) / 255.0 * alpha;
    [
        top(0) + below[0] * (1.0 - alpha),
        top(1) + below[1] * (1.0 - alpha),
        top(2) + below[2] * (1.0 - alpha),
        alpha + below[3] * (1.0 - alpha),
    ]
}

/// Inside the app icon's rounded tile: 88% of the side, centred, corners 22% of the side.
fn in_tile(x: f32, y: f32, side: f32) -> bool {
    let (left, right) = (side * 0.06, side * 0.94);
    let radius = side * 0.22;
    if x < left || x > right || y < left || y > right {
        return false;
    }
    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(left + radius, right - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

/// Inside the `m` stroked `stroke` units wide, on the 24-unit grid: three stems from y = 10
/// down to 20 (butt ends), and two arches of radius 4 over them.
fn in_mark(x: f32, y: f32, stroke: f32) -> bool {
    let half = stroke / 2.0;
    if y > 20.0 {
        return false;
    }
    if y >= 10.0 {
        return [4.0, 12.0, 20.0]
            .iter()
            .any(|stem| (x - stem).abs() <= half);
    }
    [8.0, 16.0].iter().any(|centre| {
        let distance = ((x - centre).powi(2) + (y - 10.0).powi(2)).sqrt();
        (distance - 4.0).abs() <= half
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * image.width + x) * 4) as usize;
        image.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn the_tray_mark_is_solid_below_its_fill_line_and_a_track_above() {
        let image = mccboard_logo(24, LogoStyle::Tray(TaskbarTheme::Dark));
        assert_eq!((image.width, image.height), (24, 24));
        // Bottom of the left stem: solid white.
        let stem = pixel(&image, 4, 18);
        assert_eq!(stem, [255, 255, 255, 255]);
        // Top of the left arch: white, about a third opaque.
        let arch = pixel(&image, 8, 6);
        assert_eq!(&arch[..3], &[255, 255, 255]);
        assert!((85..=93).contains(&arch[3]), "{arch:?}");
        // Between the stems, and the corners: transparent.
        assert_eq!(pixel(&image, 8, 18)[3], 0);
        assert_eq!(pixel(&image, 0, 0)[3], 0);
        assert_eq!(pixel(&image, 23, 23)[3], 0);
    }

    #[test]
    fn the_tray_mark_is_near_black_on_the_light_taskbar() {
        let image = mccboard_logo(16, LogoStyle::Tray(TaskbarTheme::Light));
        let solid: Vec<[u8; 4]> = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .map(|(x, y)| pixel(&image, x, y))
            .filter(|p| p[3] == 255)
            .collect();
        assert!(!solid.is_empty());
        assert!(solid.iter().all(|p| p[..3] == [0x1b; 3]));
    }

    #[test]
    fn the_app_icon_is_the_white_mark_on_the_graphite_tile() {
        let image = mccboard_logo(64, LogoStyle::App);
        // The tile, away from the mark.
        assert_eq!(pixel(&image, 32, 8), [0x1f, 0x24, 0x30, 255]);
        // The solid part of the left stem.
        let left_stem = (64.0 * 0.2 + 4.0 * 64.0 * 0.6 / 24.0) as u32;
        assert_eq!(pixel(&image, left_stem, 40), [255, 255, 255, 255]);
        // Rounded corners stay transparent.
        assert_eq!(pixel(&image, 1, 1)[3], 0);
    }

    #[test]
    fn every_tray_size_draws_something() {
        for size in [16, 20, 24, 32, 40, 48] {
            for theme in [TaskbarTheme::Light, TaskbarTheme::Dark] {
                let image = mccboard_logo(size, LogoStyle::Tray(theme));
                let solid = image.rgba.chunks(4).filter(|p| p[3] == 255).count();
                assert!(solid as u32 >= size, "{size} px {theme:?}: {solid}");
            }
        }
    }
}
