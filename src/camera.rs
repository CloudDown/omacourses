use egui::{Pos2, Rect, Vec2};

pub const ZOOM_MIN: f32 = 0.05;
pub const ZOOM_MAX: f32 = 8.0;
/// Levels relative to the screen size (1.0 = the sheet fills the window).
pub const ZOOM_STOPS: [f32; 9] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0];

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn to_screen(self, paper: Pos2, rect: Rect) -> Pos2 {
        Pos2::new(
            rect.min.x + self.pan.x + paper.x * self.zoom,
            rect.min.y + self.pan.y + paper.y * self.zoom,
        )
    }

    pub fn to_paper(self, screen: Pos2, rect: Rect) -> Pos2 {
        Pos2::new(
            (screen.x - rect.min.x - self.pan.x) / self.zoom,
            (screen.y - rect.min.y - self.pan.y) / self.zoom,
        )
    }

    pub fn zoom_at(&mut self, screen: Pos2, rect: Rect, factor: f32) {
        let paper = self.to_paper(screen, rect);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let now = self.to_screen(paper, rect);
        self.pan += screen - now;
    }

    pub fn set_zoom_at(&mut self, screen: Pos2, rect: Rect, zoom: f32) {
        let target = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let factor = target / self.zoom.max(0.001);
        self.zoom_at(screen, rect, factor);
    }

    pub fn fit_zoom(rect: Rect, page_w: f32, page_h: f32) -> f32 {
        let margin = 8.0;
        ((rect.width() - margin * 2.0) / page_w.max(1.0))
            .min((rect.height() - margin * 2.0) / page_h.max(1.0))
            .clamp(ZOOM_MIN, ZOOM_MAX)
    }

    pub fn fit_page(&mut self, rect: Rect, origin: Vec2, page_w: f32, page_h: f32) {
        let z = Self::fit_zoom(rect, page_w, page_h);
        self.zoom = z;
        let pw = page_w * z;
        let ph = page_h * z;
        self.pan = Vec2::new(
            (rect.width() - pw) * 0.5 - origin.x * z,
            (rect.height() - ph) * 0.5 - origin.y * z,
        );
    }

    /// Top-left corner: write into it, zoomed out enough to see the whole sheet.
    pub fn show_writing(&mut self, rect: Rect, origin: Vec2, page_w: f32, page_h: f32) {
        let margin = 8.0;
        let z_w = (rect.width() - margin * 2.0) / page_w.max(1.0);
        let z_fit = Self::fit_zoom(rect, page_w, page_h);
        self.zoom = z_w.max(z_fit * 1.7).clamp(ZOOM_MIN, ZOOM_MAX);
        self.pan = Vec2::new(
            margin - origin.x * self.zoom,
            margin - origin.y * self.zoom,
        );
    }
}

pub fn page_origin(col: i32, row: i32, page_w: f32, page_h: f32, gap: f32) -> Vec2 {
    Vec2::new(
        col as f32 * (page_w + gap),
        row as f32 * (page_h + gap),
    )
}

/// Page whose sheet contains `p`, or the nearest sheet if `p` sits in a gutter.
pub fn page_at(p: Pos2, cells: &[(i32, i32)], page_w: f32, page_h: f32, gap: f32) -> usize {
    if cells.is_empty() {
        return 0;
    }
    let w = page_w.max(1.0);
    let h = page_h.max(1.0);
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, &(col, row)) in cells.iter().enumerate() {
        let o = page_origin(col, row, w, h, gap);
        let r = Rect::from_min_size(Pos2::new(o.x, o.y), Vec2::new(w, h));
        if r.contains(p) {
            return i;
        }
        let cx = p.x.clamp(r.min.x, r.max.x);
        let cy = p.y.clamp(r.min.y, r.max.y);
        let d = (p.x - cx).hypot(p.y - cy);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{PAGE_GAP, PAGE_H, PAGE_W};

    #[test]
    fn origin_is_cell_times_stride() {
        let o = page_origin(1, 2, PAGE_W, PAGE_H, PAGE_GAP);
        assert_eq!(o.x, PAGE_W + PAGE_GAP);
        assert_eq!(o.y, 2.0 * (PAGE_H + PAGE_GAP));
    }

    #[test]
    fn page_at_hits_sheet_then_gutter() {
        let cells = [(0, 0), (1, 0), (0, 1)];
        let on_right = Pos2::new(PAGE_W + PAGE_GAP + 10.0, 10.0);
        assert_eq!(page_at(on_right, &cells, PAGE_W, PAGE_H, PAGE_GAP), 1);
        let gutter = Pos2::new(PAGE_W + PAGE_GAP * 0.5, 10.0);
        assert_eq!(page_at(gutter, &cells, PAGE_W, PAGE_H, PAGE_GAP), 0);
        let below = Pos2::new(10.0, PAGE_H + PAGE_GAP + 10.0);
        assert_eq!(page_at(below, &cells, PAGE_W, PAGE_H, PAGE_GAP), 2);
    }

    #[test]
    fn linked_origin_has_no_gap() {
        let o = page_origin(1, 0, PAGE_W, PAGE_H, 0.0);
        assert_eq!(o.x, PAGE_W);
        assert_eq!(o.y, 0.0);
    }
}
