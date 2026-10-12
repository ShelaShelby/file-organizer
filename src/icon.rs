//! The app icon (`assets/icon.ico`, drawn by `tools/make_icon.py`). The
//! same file is embedded in the exe by `build.rs` (Explorer, taskbar, tray);
//! here the 64 px bitmap inside it is read for the window icon, so no image
//! decoding crate is needed.

use eframe::egui::IconData;

const ICO: &[u8] = include_bytes!("../assets/icon.ico");

pub fn icon() -> IconData {
    from_ico(ICO, 64).unwrap_or_default()
}

fn u16_at(d: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes(d.get(i..i + 2)?.try_into().ok()?) as usize)
}

fn u32_at(d: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?) as usize)
}

/// Reads the `size` x `size` 32-bit bitmap entry of an .ico file.
fn from_ico(d: &[u8], size: usize) -> Option<IconData> {
    let count = u16_at(d, 4)?;
    for i in 0..count {
        let e = 6 + 16 * i;
        if *d.get(e)? as usize != size {
            continue;
        }
        let off = u32_at(d, e + 12)?;
        let bmp = d.get(off..)?;
        if u32_at(bmp, 0)? != 40 || u16_at(bmp, 14)? != 32 {
            continue; // not a plain 32-bit bitmap (e.g. PNG)
        }
        let pixels = bmp.get(40..40 + size * size * 4)?;
        let mut rgba = vec![0u8; size * size * 4];
        for y in 0..size {
            let src = &pixels[(size - 1 - y) * size * 4..][..size * 4]; // rows are stored bottom-up
            for x in 0..size {
                let (b, g, r, a) = (src[x * 4], src[x * 4 + 1], src[x * 4 + 2], src[x * 4 + 3]);
                rgba[(y * size + x) * 4..][..4].copy_from_slice(&[r, g, b, a]);
            }
        }
        return Some(IconData {
            rgba,
            width: size as u32,
            height: size as u32,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_file_has_a_64px_bitmap() {
        let i = super::from_ico(super::ICO, 64).expect("64 px entry");
        assert_eq!((i.width, i.height, i.rgba.len()), (64, 64, 64 * 64 * 4));
        assert!(i.rgba.chunks(4).any(|p| p[3] == 255));
    }
}
