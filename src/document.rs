use crate::ink::{InkPoint, InkStroke, Nib};
use chrono::{DateTime, Utc};
use egui::{Color32, Pos2, Vec2};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Feuille fixe, indépendante de la fenêtre (environ trois A4).
pub const PAGE_W: f32 = 794.0 * 3.0;
pub const PAGE_H: f32 = 1123.0 * 3.0;
pub const PAGE_GAP: f32 = 56.0;
/// À l’ouverture, on ne montre qu’un coin : `1/PAGE_SPAN` de la feuille.
pub const PAGE_SPAN: f32 = 3.0;

fn default_page_w() -> f32 {
    PAGE_W
}
fn default_page_h() -> f32 {
    PAGE_H
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaperKind {
    Blank,
    Lined,
    Grid,
    Dots,
    #[default]
    Millimetre,
    Slate,
}

impl PaperKind {
    pub fn label(self) -> &'static str {
        match self {
            PaperKind::Blank => "Blank",
            PaperKind::Lined => "Lined",
            PaperKind::Grid => "Grid",
            PaperKind::Dots => "Dots",
            PaperKind::Millimetre => "Millimeter",
            PaperKind::Slate => "Slate",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            PaperKind::Blank => PaperKind::Lined,
            PaperKind::Lined => PaperKind::Grid,
            PaperKind::Grid => PaperKind::Dots,
            PaperKind::Dots => PaperKind::Millimetre,
            PaperKind::Millimetre => PaperKind::Slate,
            PaperKind::Slate => PaperKind::Blank,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextBox {
    pub id: Uuid,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub text: String,
    pub size_pt: f32,
    pub color: [u8; 4],
}

impl TextBox {
    pub fn new(pos: Pos2, color: Color32) -> Self {
        Self {
            id: Uuid::new_v4(),
            pos: [pos.x, pos.y],
            size: [280.0, 80.0],
            text: String::new(),
            size_pt: 18.0,
            color: [color.r(), color.g(), color.b(), color.a()],
        }
    }

    pub fn min(&self) -> Pos2 {
        Pos2::new(self.pos[0], self.pos[1])
    }

    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.min(), Vec2::new(self.size[0], self.size[1]))
    }

    pub fn color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.color[0], self.color[1], self.color[2], self.color[3])
    }

    pub fn translate(&mut self, d: Vec2) {
        self.pos[0] += d.x;
        self.pos[1] += d.y;
    }

    pub fn contains(&self, p: Pos2) -> bool {
        self.rect().contains(p)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageObj {
    pub id: Uuid,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub file: String,
}

impl ImageObj {
    pub fn min(&self) -> Pos2 {
        Pos2::new(self.pos[0], self.pos[1])
    }

    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.min(), Vec2::new(self.size[0], self.size[1]))
    }

    pub fn translate(&mut self, d: Vec2) {
        self.pos[0] += d.x;
        self.pos[1] += d.y;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub strokes: Vec<InkStroke>,
    pub texts: Vec<TextBox>,
    pub images: Vec<ImageObj>,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub paper: PaperKind,
    pub cover: u8,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub pinned: bool,
    pub pages: Vec<Page>,
    #[serde(default = "default_page_w")]
    pub page_w: f32,
    #[serde(default = "default_page_h")]
    pub page_h: f32,
}

impl Note {
    pub fn blank(title: impl Into<String>, cover: u8) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            paper: PaperKind::Millimetre,
            cover,
            created: now,
            updated: now,
            pinned: false,
            pages: vec![Page::default()],
            page_w: PAGE_W,
            page_h: PAGE_H,
        }
    }

    pub fn page_size(&self) -> (f32, f32) {
        (self.page_w.max(1.0), self.page_h.max(1.0))
    }

    pub fn touch(&mut self) {
        self.updated = Utc::now();
    }

    pub fn add_page(&mut self) {
        self.pages.push(Page::default());
        self.touch();
    }
}

pub fn stroke_from_polyline(
    pts: &[[f32; 2]],
    nib: Nib,
    color: Color32,
    width: f32,
) -> InkStroke {
    let mut s = InkStroke::new(nib, color, width);
    for p in pts {
        s.points.push(InkPoint::new(Pos2::new(p[0], p[1]), 0.85));
    }
    s
}

pub fn squiggle(from: Pos2, to: Pos2, amp: f32, n: usize) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let base = from.lerp(to, t);
        let dir = (to - from).normalized();
        let nrm = Vec2::new(-dir.y, dir.x);
        let wobble = (t * std::f32::consts::PI * 3.0).sin() * amp * (1.0 - (t - 0.5).abs());
        let p = base + nrm * wobble;
        out.push([p.x, p.y]);
    }
    out
}
