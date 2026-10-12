//! Window icon, drawn in code (a folder with a tick) so no image files or
//! decoding crates are needed.

use eframe::egui::IconData;

pub fn icon() -> IconData {
    const S: u32 = 64;
    let mut rgba = vec![0u8; (S * S * 4) as usize];
    let folder = [244u8, 180, 60, 255];
    let tab = [214u8, 150, 40, 255];
    let tick = [255u8, 255, 255, 255];
    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32, y as f32);
            let mut c = None;
            if (6.0..=28.0).contains(&fx) && (10.0..=18.0).contains(&fy) {
                c = Some(tab);
            }
            if (4.0..=60.0).contains(&fx) && (16.0..=56.0).contains(&fy) {
                c = Some(folder);
            }
            // tick: two thick line segments
            let d1 = seg_dist(fx, fy, 18.0, 36.0, 28.0, 46.0);
            let d2 = seg_dist(fx, fy, 28.0, 46.0, 47.0, 26.0);
            if c.is_some() && d1.min(d2) < 3.5 {
                c = Some(tick);
            }
            if let Some(px) = c {
                let i = ((y * S + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&px);
            }
        }
    }
    IconData { rgba, width: S, height: S }
}

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}
