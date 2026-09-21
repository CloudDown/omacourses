use crate::document::PAGE_GAP;
use egui::{Pos2, Rect, Vec2};

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
        self.zoom = (self.zoom * factor).clamp(0.18, 6.0);
        let now = self.to_screen(paper, rect);
        self.pan += screen - now;
    }

    pub fn fit_page(&mut self, rect: Rect, page: usize, page_w: f32, page_h: f32) {
        let margin = 8.0;
        let z = ((rect.width() - margin * 2.0) / page_w.max(1.0))
            .min((rect.height() - margin * 2.0) / page_h.max(1.0))
            .clamp(0.15, 8.0);
        self.zoom = z;
        let origin_y = page as f32 * (page_h + PAGE_GAP);
        let pw = page_w * z;
        let ph = page_h * z;
        self.pan = Vec2::new(
            (rect.width() - pw) * 0.5,
            (rect.height() - ph) * 0.5 - origin_y * z,
        );
    }
}

pub fn page_origin(page: usize, page_h: f32) -> Vec2 {
    Vec2::new(0.0, page as f32 * (page_h + PAGE_GAP))
}

pub fn page_at_y(y: f32, pages: usize, page_h: f32) -> usize {
    if pages == 0 {
        return 0;
    }
    let stride = page_h + PAGE_GAP;
    ((y / stride).floor() as i32).clamp(0, pages as i32 - 1) as usize
}
