//! The `.ico` format: a directory of square frames, 32-bit DIBs below 256 px and a PNG at 256
//! (what Windows' own icons do: the taskbar, Alt+Tab and Explorer each pick their size).

use super::Image;

/// `.ico` bytes holding `frames` (square images, each 1 to 256 px), in the order given.
pub(super) fn write(frames: &[Image]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| {
            assert_eq!(frame.width, frame.height, "icon frames are square");
            assert!(
                (1..=256).contains(&frame.width),
                "icon frames are 1 to 256 px"
            );
            if frame.width == 256 {
                frame.to_png()
            } else {
                dib(frame)
            }
        })
        .collect();

    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes()); // reserved
    out.extend(1u16.to_le_bytes()); // type: icon
    out.extend((frames.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * frames.len() as u32;
    for (frame, data) in frames.iter().zip(&images) {
        let side = (frame.width % 256) as u8; // 0 means 256
        out.extend([side, side, 0, 0]); // width, height, palette colors, reserved
        out.extend(1u16.to_le_bytes()); // planes
        out.extend(32u16.to_le_bytes()); // bits per pixel
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in images {
        out.extend(data);
    }
    out
}

/// A `BITMAPINFOHEADER`, the BGRA rows bottom-up, then the 1-bit AND mask (set where the
/// pixel is fully transparent, rows padded to 4 bytes). The header's height counts both.
fn dib(frame: &Image) -> Vec<u8> {
    let s = frame.width as usize;
    let mask_row = s.div_ceil(32) * 4;
    let image_size = s * s * 4 + mask_row * s;
    let mut out = Vec::with_capacity(40 + image_size);
    out.extend(40u32.to_le_bytes()); // biSize
    out.extend((s as i32).to_le_bytes()); // biWidth
    out.extend((2 * s as i32).to_le_bytes()); // biHeight: color + mask
    out.extend(1u16.to_le_bytes()); // biPlanes
    out.extend(32u16.to_le_bytes()); // biBitCount
    out.extend(0u32.to_le_bytes()); // biCompression: BI_RGB
    out.extend((image_size as u32).to_le_bytes()); // biSizeImage
    out.extend([0u8; 16]); // resolution, colors used, colors important
    for y in (0..s).rev() {
        for p in frame.rgba[y * s * 4..(y + 1) * s * 4].as_chunks::<4>().0 {
            out.extend([p[2], p[1], p[0], p[3]]);
        }
    }
    for y in (0..s).rev() {
        let mut row = vec![0u8; mask_row];
        for x in 0..s {
            if frame.rgba[(y * s + x) * 4 + 3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend(row);
    }
    out
}
