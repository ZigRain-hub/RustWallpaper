use super::*;

pub(super) unsafe fn show(hwnd: Handle, effect: u8, progress: f32) -> bool {
    let mut rect = Rect::default();
    if GetClientRect(hwnd, &mut rect) == 0 { return false; }
    let width = rect.right.max(1);
    let height = rect.bottom.max(1);
    let p = progress.clamp(0.0, 1.0) as f64;
    let region = if p >= 1.0 || effect == 0 {
        CreateRectRgn(0, 0, 0, 0)
    } else if effect == 1 {
        let wave = (width as f64 * 0.07).max(24.0);
        let base = -wave + p * (width as f64 + 2.0 * wave);
        let mut points = Vec::with_capacity(67);
        points.push(Point { x: width, y: 0 });
        points.push(Point { x: width, y: height });
        for i in (0..=64).rev() {
            let y = height * i / 64;
            let edge = base + wave * (std::f64::consts::PI * y as f64 / height as f64).sin();
            points.push(Point { x: (edge.round() as i32).clamp(0, width), y });
        }
        CreatePolygonRgn(points.as_ptr(), points.len() as i32, 1)
    } else if effect == 2 {
        let full = CreateRectRgn(0, 0, width, height);
        let cx = width as f64 / 2.0;
        let cy = height as f64 / 2.0;
        let radius = (cx * cx + cy * cy).sqrt() * p + 1.0;
        let hole = CreateEllipticRgn((cx-radius).floor() as i32, (cy-radius).floor() as i32,
            (cx+radius).ceil() as i32, (cy+radius).ceil() as i32);
        if full.is_null() || hole.is_null() {
            if !full.is_null() { DeleteObject(full); }
            if !hole.is_null() { DeleteObject(hole); }
            return false;
        }
        let ok = CombineRgn(full, full, hole, 4) != 0;
        DeleteObject(hole);
        if !ok { DeleteObject(full); return false; }
        full
    } else { return false; };
    if region.is_null() { return false; }
    if SetWindowRgn(hwnd, region, 1) == 0 { DeleteObject(region); false } else { true }
}
