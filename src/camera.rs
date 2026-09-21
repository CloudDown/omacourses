use crate::document::{PAGE_GAP, PAGE_H, PAGE_W};
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

    pub fn fit_page(&mut self, rect: Rect, page: usize) {
        let margin = 48.0;
        let z = ((rect.width() - margin * 2.0) / PAGE_W)
            .min((rect.height() - margin * 2.0) / PAGE_H)
            .clamp(0.2, 3.0);
        self.zoom = z;
        let origin_y = page as f32 * (PAGE_H + PAGE_GAP);
        let pw = PAGE_W * z;
        let ph = PAGE_H * z;
        self.pan = Vec2::new(
            (rect.width() - pw) * 0.5,
            (rect.height() - ph) * 0.5 - origin_y * z,
        );
    }
}

pub fn page_origin(page: usize) -> Vec2 {
    Vec2::new(0.0, page as f32 * (PAGE_H + PAGE_GAP))
}

pub fn page_at_y(y: f32, pages: usize) -> usize {
    if pages == 0 {
        return 0;
    }
    let stride = PAGE_H + PAGE_GAP;
    ((y / stride).floor() as i32).clamp(0, pages as i32 - 1) as usize
}
