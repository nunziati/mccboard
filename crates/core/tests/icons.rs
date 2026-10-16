//! Desktop profile icons (spec #34, tickets #37 and #38): the package's logos found in its
//! install folder, the taskbar icon recolored to the account's color, the tray mark, the
//! `.ico` files and their cache under `~/.mccboard/state/icons/`. Images are synthesised here:
//! Anthropic's logo is never part of the repository.

mod common;

use std::fs;
use std::path::Path;

use common::{TempHome, UUID_A, UUID_B, UUID_C};
use mccboard_core::{
    AccountUuid, Color, Error, IconCache, IconSources, Image, PackageLogos, RemoveLevel,
};

const ORANGE: [u8; 3] = [0xd9, 0x77, 0x57];

fn color(hex: &str) -> Color {
    hex.parse().unwrap()
}

fn uuid(s: &str) -> AccountUuid {
    AccountUuid::new(s)
}

/// A `side`×`side` image, transparent but for a square tile of `tile` pixels in its middle
/// (wide padding, like the package's tile logos) holding a white disc on Claude's orange.
fn plated(side: u32, tile: u32) -> Image {
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let start = (side - tile) / 2;
    let centre = side as f32 / 2.0;
    for y in start..start + tile {
        for x in start..start + tile {
            let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
            let white = (dx * dx + dy * dy).sqrt() < tile as f32 / 4.0;
            let px = if white {
                [255, 255, 255, 255]
            } else {
                [ORANGE[0], ORANGE[1], ORANGE[2], 255]
            };
            let at = ((y * side + x) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&px);
        }
    }
    Image::new(side, side, rgba)
}

/// A `side`×`side` image with a white disc of radius `radius` in its middle on a transparent
/// background: the mark as the unplated logo and the tray icon show it.
fn unplated(side: u32, radius: f32) -> Image {
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let centre = side as f32 / 2.0;
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
            if (dx * dx + dy * dy).sqrt() < radius {
                let at = ((y * side + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    Image::new(side, side, rgba)
}

fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width + x) * 4) as usize;
    image.rgba[at..at + 4].try_into().unwrap()
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// The frames of an `.ico`: (side, image bytes).
fn frames(ico: &[u8]) -> Vec<(u32, &[u8])> {
    assert_eq!(u16_at(ico, 0), 0, "reserved");
    assert_eq!(u16_at(ico, 2), 1, "type: icon");
    let count = u16_at(ico, 4) as usize;
    (0..count)
        .map(|i| {
            let entry = 6 + 16 * i;
            let side = match ico[entry] {
                0 => 256,
                n => n as u32,
            };
            assert_eq!(ico[entry + 1] as u32 % 256, side % 256, "square");
            assert_eq!(u16_at(ico, entry + 4), 1, "planes");
            assert_eq!(u16_at(ico, entry + 6), 32, "bits per pixel");
            let (len, offset) = (
                u32_at(ico, entry + 8) as usize,
                u32_at(ico, entry + 12) as usize,
            );
            (side, &ico[offset..offset + len])
        })
        .collect()
}

#[test]
fn trimming_keeps_only_the_visible_pixels_in_a_square() {
    // 40×30, visible pixels in a 12 wide × 6 tall band; a faint pixel (alpha 16) doesn't count.
    let (w, h) = (40u32, 30u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 10..16 {
        for x in 5..17 {
            let at = ((y * w + x) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&[1, 2, 3, 255]);
        }
    }
    rgba[((2 * w + 30) * 4 + 3) as usize] = 16;
    let trimmed = Image::new(w, h, rgba).trimmed();

    assert_eq!((trimmed.width, trimmed.height), (12, 12));
    assert_eq!(pixel(&trimmed, 0, 0)[3], 0, "padding above the band");
    assert_eq!(
        pixel(&trimmed, 0, 3),
        [1, 2, 3, 255],
        "band centred vertically"
    );
    assert_eq!(pixel(&trimmed, 11, 8), [1, 2, 3, 255]);
    assert_eq!(pixel(&trimmed, 11, 9)[3], 0, "padding below the band");
}

#[test]
fn recoloring_takes_the_account_hue_and_keeps_white_grey_and_alpha() {
    let rgba = vec![
        ORANGE[0], ORANGE[1], ORANGE[2], 200, // Claude's orange, half transparent
        255, 255, 255, 255, // white
        128, 128, 128, 255, // grey
        10, 20, 30, 0, // fully transparent
    ];
    let blue = color("#2563eb");
    let out = Image::new(4, 1, rgba.clone()).recolored(&blue);

    assert_eq!(
        pixel(&out, 0, 0),
        [0x25, 0x63, 0xeb, 200],
        "orange becomes the color"
    );
    assert_eq!(
        &out.rgba[4..],
        &rgba[4..],
        "white, grey and transparent stay"
    );

    // Recoloring with Claude's own orange gives the image back.
    let same = Image::new(4, 1, rgba.clone()).recolored(&color("#d97757"));
    for (a, b) in same.rgba.iter().zip(&rgba) {
        assert!((*a as i32 - *b as i32).abs() <= 1);
    }
}

#[test]
fn recoloring_keeps_the_lightness_offset_from_claude_orange() {
    // A darker orange (same hue and saturation, lower lightness) stays darker in the new color.
    let dark = [0xa8, 0x4d, 0x30];
    let out = Image::new(
        2,
        1,
        vec![
            ORANGE[0], ORANGE[1], ORANGE[2], 255, dark[0], dark[1], dark[2], 255,
        ],
    )
    .recolored(&color("#16a34a"));
    let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    assert!(sum(pixel(&out, 1, 0)) < sum(pixel(&out, 0, 0)));
    let p = pixel(&out, 1, 0);
    assert!(p[1] > p[0] && p[1] > p[2], "still green: {p:?}");
}

#[test]
fn resizing_scales_a_square_image() {
    let image = plated(100, 100);
    for side in [16, 256] {
        let out = image.resized(side);
        assert_eq!((out.width, out.height), (side, side));
        assert_eq!(out.rgba.len(), (side * side * 4) as usize);
        assert_eq!(
            pixel(&out, side / 2, side / 2),
            [255, 255, 255, 255],
            "white middle"
        );
        assert_eq!(
            pixel(&out, 0, 0),
            [ORANGE[0], ORANGE[1], ORANGE[2], 255],
            "orange corner"
        );
    }
}

#[test]
fn png_round_trip() {
    let image = Image::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(Image::from_png(&image.to_png()).unwrap(), image);
}

#[test]
fn the_app_icon_is_an_ico_with_dib_frames_and_a_png_at_256() {
    let sources = IconSources {
        plated: Some(plated(300, 120)),
        unplated: None,
    };
    let ico = sources.app_ico(&color("#2563eb")).unwrap();
    let frames = frames(&ico);

    let sides: Vec<u32> = frames.iter().map(|(side, _)| *side).collect();
    assert_eq!(sides, [16, 20, 24, 30, 32, 36, 40, 48, 64, 72, 96, 256]);
    assert_eq!(
        u32_at(&ico, 6 + 12),
        6 + 16 * 12,
        "first image after the directory"
    );

    for (side, data) in &frames[..11] {
        let side = *side as usize;
        assert_eq!(u32_at(data, 0), 40, "BITMAPINFOHEADER");
        assert_eq!(u32_at(data, 4) as usize, side, "width");
        assert_eq!(
            u32_at(data, 8) as usize,
            2 * side,
            "height counts the AND mask"
        );
        assert_eq!(u16_at(data, 14), 32, "32-bit");
        let mask_row = side.div_ceil(32) * 4;
        assert_eq!(data.len(), 40 + side * side * 4 + mask_row * side);
    }

    let (_, png) = frames[11];
    assert_eq!(&png[..4], b"\x89PNG");
    let big = Image::from_png(png).unwrap();
    assert_eq!((big.width, big.height), (256, 256));
    // Trimmed: the tile fills the frame, its corner is the account's blue, the disc white.
    assert_eq!(pixel(&big, 2, 2), [0x25, 0x63, 0xeb, 255]);
    assert_eq!(pixel(&big, 128, 128), [255, 255, 255, 255]);
}

#[test]
fn the_dib_frames_are_bottom_up_bgra_with_a_transparency_mask() {
    // Top half a greyish beige (unsaturated: recoloring leaves it), bottom half transparent.
    let side = 16u32;
    let mut rgba = vec![0u8; (side * side * 4) as usize];
    for at in (0..(side * side / 2) as usize).map(|i| i * 4) {
        rgba[at..at + 4].copy_from_slice(&[180, 160, 140, 255]);
    }
    let sources = IconSources {
        plated: None,
        unplated: Some(Image::new(side, side, rgba)),
    };
    // Trimming makes the beige half a 16×8 band centred in 16×16: rows 4..12 are beige.
    let ico = sources.app_ico(&color("#d97757")).unwrap();
    let (_, dib) = frames(&ico)[0];
    let pixels = &dib[40..40 + 16 * 16 * 4];
    let row = |y: usize| &pixels[(15 - y) * 64..][..64]; // bottom-up
    assert_eq!(&row(5)[..4], &[140, 160, 180, 255], "BGRA");
    assert_eq!(row(0)[3], 0, "transparent");
    let mask = &dib[40 + 16 * 16 * 4..];
    let mask_row = |y: usize| &mask[(15 - y) * 4..][..4];
    assert_eq!(
        mask_row(0),
        &[0xff, 0xff, 0, 0],
        "transparent pixels set, padding zero"
    );
    assert_eq!(mask_row(5), &[0, 0, 0, 0]);
}

#[test]
fn the_tray_mark_is_the_mark_alone_in_the_account_color() {
    let green = color("#16a34a");
    for sources in [
        IconSources {
            plated: None,
            unplated: Some(unplated(200, 60.0)),
        },
        // Only the plated logo: the mark is its white part, the tile goes.
        IconSources {
            plated: Some(plated(200, 160)),
            unplated: None,
        },
    ] {
        let mark = sources.tray_mark(&green, 16).unwrap();
        assert_eq!((mark.width, mark.height), (16, 16));
        for (x, y) in [(0, 0), (15, 0), (0, 15), (15, 15)] {
            assert_eq!(pixel(&mark, x, y)[3], 0, "transparent corner");
        }
        let centre = pixel(&mark, 8, 8);
        assert_eq!(centre, [0x16, 0xa3, 0x4a, 255], "the mark in the color");
        for p in mark.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0) {
            assert_eq!(
                &p[..3],
                &[0x16, 0xa3, 0x4a],
                "every visible pixel in the color"
            );
        }
        // Trimmed: the disc touches the frame's edges.
        assert!(pixel(&mark, 0, 8)[3] > 0 && pixel(&mark, 8, 0)[3] > 0);
    }
}

#[test]
fn the_tray_icon_has_the_small_icon_sizes() {
    let sources = IconSources {
        plated: None,
        unplated: Some(unplated(64, 20.0)),
    };
    let ico = sources.tray_ico(&color("#9333ea")).unwrap();
    let sides: Vec<u32> = frames(&ico).iter().map(|(side, _)| *side).collect();
    assert_eq!(sides, [16, 20, 24, 32, 40, 48]);
}

#[test]
fn no_source_image_is_an_error() {
    let sources = IconSources {
        plated: None,
        unplated: None,
    };
    let blue = color("#2563eb");
    assert!(matches!(sources.app_ico(&blue), Err(Error::NoIconSource)));
    assert!(matches!(sources.tray_ico(&blue), Err(Error::NoIconSource)));
}

// --- The package's logos -------------------------------------------------------------------

const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Applications>
    <Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Claude" Description="Claude"
        BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png"
        Square44x44Logo="Assets\Square44x44Logo.png" />
    </Application>
  </Applications>
</Package>
"#;

fn write_png(path: &Path, side: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let image = Image::new(side, side, vec![255; (side * side * 4) as usize]);
    fs::write(path, image.to_png()).unwrap();
}

fn package(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("AppxManifest.xml"), MANIFEST).unwrap();
    for file in files {
        write_png(&dir.path().join(file), 4);
    }
    dir
}

#[test]
fn the_package_logos_are_the_largest_targetsize_plated_and_unplated() {
    let dir = package(&[
        "Assets/Square150x150Logo.scale-400.png",
        "Assets/Square44x44Logo.scale-200.png",
        "Assets/Square44x44Logo.targetsize-16.png",
        "Assets/Square44x44Logo.targetsize-48.png",
        "Assets/Square44x44Logo.targetsize-256.png",
        "Assets/Square44x44Logo.targetsize-256_contrast-black.png",
        "Assets/Square44x44Logo.targetsize-32_altform-unplated.png",
        "Assets/Square44x44Logo.altform-unplated_targetsize-256.png",
        "Assets/Square44x44Logo.targetsize-256_altform-lightunplated.png",
        "Assets/contrast-white/Square44x44Logo.targetsize-512.png",
        "Assets/Square44x44Logo.targetsize-1024_altform-unplated_contrast-white.png",
    ]);
    let logos = PackageLogos::find(dir.path());
    let assets = dir.path().join("Assets");
    assert_eq!(
        logos.plated,
        Some(assets.join("Square44x44Logo.targetsize-256.png"))
    );
    assert_eq!(
        logos.unplated,
        Some(assets.join("Square44x44Logo.altform-unplated_targetsize-256.png"))
    );
}

#[test]
fn without_targetsize_the_plated_logo_is_the_largest_scale_then_the_bare_file() {
    let dir = package(&[
        "Assets/Square44x44Logo.scale-100.png",
        "Assets/scale-400/Square44x44Logo.png",
        "Assets/Square44x44Logo.scale-200.png",
        "Assets/Square44x44Logo.png",
    ]);
    let logos = PackageLogos::find(dir.path());
    assert_eq!(
        logos.plated,
        Some(dir.path().join("Assets/scale-400/Square44x44Logo.png"))
    );
    assert_eq!(logos.unplated, None);

    let dir = package(&["Assets/Square44x44Logo.png"]);
    assert_eq!(
        PackageLogos::find(dir.path()).plated,
        Some(dir.path().join("Assets/Square44x44Logo.png"))
    );
}

#[test]
fn no_manifest_or_no_logo_finds_nothing() {
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(PackageLogos::find(empty.path()), PackageLogos::default());
    assert_eq!(
        PackageLogos::find(&empty.path().join("missing")),
        PackageLogos::default()
    );

    let dir = package(&["Assets/Square150x150Logo.png"]);
    assert_eq!(PackageLogos::find(dir.path()), PackageLogos::default());
}

#[test]
fn a_manifest_path_leaving_the_install_folder_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("install");
    fs::create_dir_all(&install).unwrap();
    fs::write(
        install.join("AppxManifest.xml"),
        MANIFEST.replace(r"Assets\Square44x44Logo.png", r"..\Square44x44Logo.png"),
    )
    .unwrap();
    write_png(&dir.path().join("Square44x44Logo.targetsize-256.png"), 4);
    assert_eq!(PackageLogos::find(&install), PackageLogos::default());
}

#[test]
fn icon_sources_load_the_package_logos() {
    let dir = package(&[]);
    write_png(
        &dir.path().join("Assets/Square44x44Logo.targetsize-256.png"),
        8,
    );
    write_png(
        &dir.path()
            .join("Assets/Square44x44Logo.targetsize-256_altform-unplated.png"),
        6,
    );
    let sources = IconSources::from_package(dir.path()).unwrap();
    assert_eq!(sources.plated.map(|i| i.width), Some(8));
    assert_eq!(sources.unplated.map(|i| i.width), Some(6));

    let broken = package(&[]);
    let path = broken
        .path()
        .join("Assets/Square44x44Logo.targetsize-256.png");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"not a png").unwrap();
    match IconSources::from_package(broken.path()) {
        Err(err @ Error::IconSourceUnreadable { .. }) => assert_eq!(err.path(), Some(&*path)),
        other => panic!("expected IconSourceUnreadable, got {other:?}"),
    }
}

// --- The cache -----------------------------------------------------------------------------

fn sources() -> IconSources {
    IconSources {
        plated: Some(plated(64, 48)),
        unplated: Some(unplated(64, 20.0)),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn the_cache_names_icons_by_account_and_color() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let cache = IconCache::new(&core.paths());
    let dir = core.paths().state_dir().join("icons");

    let icons = cache.icons(&uuid(UUID_B), &color("#2563eb"));
    assert_eq!(icons.app, dir.join(format!("{UUID_B}-2563eb.ico")));
    assert_eq!(icons.tray, dir.join(format!("{UUID_B}-2563eb-tray.ico")));
    assert!(!icons.app.exists());

    let ensured = cache
        .ensure(&uuid(UUID_B), &color("#2563eb"), || Ok(sources()))
        .unwrap();
    assert_eq!(ensured, icons);
    assert_eq!(frames(&fs::read(&icons.app).unwrap()).len(), 12);
    assert_eq!(frames(&fs::read(&icons.tray).unwrap()).len(), 6);
    assert_eq!(
        names(&dir),
        [
            format!("{UUID_B}-2563eb-tray.ico"),
            format!("{UUID_B}-2563eb.ico")
        ]
    );

    // Cached: the sources aren't loaded again.
    let again = cache
        .ensure(&uuid(UUID_B), &color("#2563eb"), || {
            panic!("sources loaded for a cached icon")
        })
        .unwrap();
    assert_eq!(again, icons);
}

#[test]
fn a_new_color_gets_new_files_and_the_old_ones_go() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let cache = IconCache::new(&core.paths());
    let dir = core.paths().state_dir().join("icons");
    cache
        .ensure(&uuid(UUID_B), &color("#2563eb"), || Ok(sources()))
        .unwrap();
    cache
        .ensure(&uuid(UUID_C), &color("#2563eb"), || Ok(sources()))
        .unwrap();
    fs::write(dir.join("unrelated.txt"), "keep").unwrap();

    let icons = cache
        .ensure(&uuid(UUID_B), &color("#16a34a"), || Ok(sources()))
        .unwrap();
    assert_eq!(icons.app, dir.join(format!("{UUID_B}-16a34a.ico")));
    assert_eq!(
        names(&dir),
        [
            format!("{UUID_B}-16a34a-tray.ico"),
            format!("{UUID_B}-16a34a.ico"),
            format!("{UUID_C}-2563eb-tray.ico"),
            format!("{UUID_C}-2563eb.ico"),
            "unrelated.txt".to_owned(),
        ]
    );
}

#[test]
fn removing_an_account_s_icons_keeps_the_others() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let cache = IconCache::new(&core.paths());
    let dir = core.paths().state_dir().join("icons");
    cache.remove(&uuid(UUID_B)).unwrap(); // nothing cached yet: fine
    for account in [UUID_B, UUID_C] {
        cache
            .ensure(&uuid(account), &color("#2563eb"), || Ok(sources()))
            .unwrap();
    }

    cache.remove(&uuid(UUID_B)).unwrap();
    assert_eq!(
        names(&dir),
        [
            format!("{UUID_C}-2563eb-tray.ico"),
            format!("{UUID_C}-2563eb.ico")
        ]
    );
}

#[test]
fn a_failed_source_writes_nothing() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let cache = IconCache::new(&core.paths());
    let result = cache.ensure(&uuid(UUID_B), &color("#2563eb"), || {
        Err(Error::NoIconSource)
    });
    assert!(matches!(result, Err(Error::NoIconSource)));
    assert!(names(&core.paths().state_dir().join("icons")).is_empty());
}

#[test]
fn removing_a_desktop_profile_removes_its_cached_icons() {
    let home = TempHome::new();
    let (core, _env) = home.core();
    let primary = home.primary();
    primary.logged_in_as(UUID_A, "a@example.com");
    core.register(&primary.dir).unwrap();
    let b = home.linked(".claude-b");
    b.logged_in_as(UUID_B, "b@example.com");
    core.register(&b.dir).unwrap();
    let profile = core.add_desktop_profile(&uuid(UUID_B), None).unwrap();

    let cache = IconCache::new(&core.paths());
    let icons = cache
        .ensure(&uuid(UUID_B), &profile.color, || Ok(sources()))
        .unwrap();
    assert!(icons.app.exists() && icons.tray.exists());

    core.remove_desktop_profile(&uuid(UUID_B), RemoveLevel::FromMccboard)
        .unwrap();
    assert!(!icons.app.exists() && !icons.tray.exists());
}
