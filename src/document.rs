use std::collections::BTreeMap;

use crate::ink::{InkPoint, InkStroke, Nib};
use chrono::{DateTime, Utc};
use egui::{Color32, Pos2, Vec2};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Landscape A4 sheet, independent of the window.
pub const PAGE_W: f32 = 1123.0;
pub const PAGE_H: f32 = 794.0;
pub const PAGE_GAP: f32 = 56.0;

fn default_page_w() -> f32 {
    PAGE_W
}
fn default_page_h() -> f32 {
    PAGE_H
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SheetJoin {
    #[default]
    Linked,
    Separate,
}

impl SheetJoin {
    pub fn gap(self) -> f32 {
        match self {
            SheetJoin::Linked => 0.0,
            SheetJoin::Separate => PAGE_GAP,
        }
    }
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
    #[serde(default)]
    pub col: i32,
    #[serde(default)]
    pub row: i32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            col: 0,
            row: 0,
        }
    }
}

impl Page {
    pub fn at(col: i32, row: i32) -> Self {
        Self {
            col,
            row,
            ..Self::default()
        }
    }
}

impl Page {
    pub fn translate(&mut self, d: Vec2) {
        for s in &mut self.strokes {
            s.translate(d);
        }
        for t in &mut self.texts {
            t.translate(d);
        }
        for im in &mut self.images {
            im.translate(d);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub paper: PaperKind,
    pub cover: u8,
    #[serde(default)]
    pub emoji: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub pinned: bool,
    pub pages: Vec<Page>,
    #[serde(default = "default_page_w")]
    pub page_w: f32,
    #[serde(default = "default_page_h")]
    pub page_h: f32,
    #[serde(default)]
    pub sheet_join: SheetJoin,
}

impl Note {
    pub fn blank(title: impl Into<String>, cover: u8) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            paper: PaperKind::Millimetre,
            cover,
            emoji: String::new(),
            created: now,
            updated: now,
            pinned: false,
            pages: vec![Page::default()],
            page_w: PAGE_W,
            page_h: PAGE_H,
            sheet_join: SheetJoin::Linked,
        }
    }

    pub fn page_size(&self) -> (f32, f32) {
        (self.page_w.max(1.0), self.page_h.max(1.0))
    }

    pub fn touch(&mut self) {
        self.updated = Utc::now();
    }

    pub fn cell_taken(&self, col: i32, row: i32) -> bool {
        self.pages.iter().any(|p| p.col == col && p.row == row)
    }

    /// Old notes stacked pages by vec index with no col/row. Spread them vertically.
    pub fn normalize_page_grid(&mut self) {
        if self.pages.len() <= 1 {
            return;
        }
        if self.pages.iter().all(|p| p.col == 0 && p.row == 0) {
            for (i, p) in self.pages.iter_mut().enumerate() {
                p.row = i as i32;
            }
        }
    }

    /// Rebuilds the notebook as one continuous sheet or as equal unit pages.
    /// Linked ↔ Separate is reversible at `PAGE_W` × `PAGE_H`.
    pub fn apply_sheet_join(&mut self, to: SheetJoin) {
        if self.pages.is_empty() {
            self.pages.push(Page::default());
        }
        let (ow, oh) = self.page_size();
        let tw = PAGE_W;
        let th = PAGE_H;
        let mut min_c = i32::MAX;
        let mut min_r = i32::MAX;
        let mut max_c = i32::MIN;
        let mut max_r = i32::MIN;
        for p in &self.pages {
            min_c = min_c.min(p.col);
            min_r = min_r.min(p.row);
            max_c = max_c.max(p.col);
            max_r = max_r.max(p.row);
        }
        let origin_x = min_c as f32 * ow;
        let origin_y = min_r as f32 * oh;
        let cover_w = (max_c - min_c + 1) as f32 * ow;
        let cover_h = (max_r - min_r + 1) as f32 * oh;
        let cols = tiles_along(cover_w, tw);
        let rows = tiles_along(cover_h, th);

        let mut strokes = Vec::new();
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for p in &self.pages {
            let dx = p.col as f32 * ow - origin_x;
            let dy = p.row as f32 * oh - origin_y;
            let shift = Vec2::new(dx, dy);
            for mut s in p.strokes.iter().cloned() {
                s.translate(shift);
                strokes.push(s);
            }
            for mut t in p.texts.iter().cloned() {
                t.translate(shift);
                texts.push(t);
            }
            for mut im in p.images.iter().cloned() {
                im.translate(shift);
                images.push(im);
            }
        }

        self.sheet_join = to;
        match to {
            SheetJoin::Linked => {
                self.page_w = cols as f32 * tw;
                self.page_h = rows as f32 * th;
                self.pages = vec![Page {
                    strokes,
                    texts,
                    images,
                    col: 0,
                    row: 0,
                }];
            }
            SheetJoin::Separate => {
                self.page_w = tw;
                self.page_h = th;
                let mut tiles: BTreeMap<(i32, i32), Page> = BTreeMap::new();
                for r in 0..rows {
                    for c in 0..cols {
                        tiles.insert((c, r), Page::at(c, r));
                    }
                }
                for s in strokes {
                    for (c, r, mut piece) in split_stroke_across_tiles(&s, tw, th) {
                        let (c, r) = clamp_cell(c, r, cols, rows);
                        piece.translate(Vec2::new(-(c as f32 * tw), -(r as f32 * th)));
                        tiles.entry((c, r)).or_insert_with(|| Page::at(c, r)).strokes.push(piece);
                    }
                }
                for mut t in texts {
                    let (c, r) = clamp_cell(
                        (t.pos[0] / tw).floor() as i32,
                        (t.pos[1] / th).floor() as i32,
                        cols,
                        rows,
                    );
                    t.pos[0] -= c as f32 * tw;
                    t.pos[1] -= r as f32 * th;
                    tiles.entry((c, r)).or_insert_with(|| Page::at(c, r)).texts.push(t);
                }
                for mut im in images {
                    let (c, r) = clamp_cell(
                        (im.pos[0] / tw).floor() as i32,
                        (im.pos[1] / th).floor() as i32,
                        cols,
                        rows,
                    );
                    im.pos[0] -= c as f32 * tw;
                    im.pos[1] -= r as f32 * th;
                    tiles
                        .entry((c, r))
                        .or_insert_with(|| Page::at(c, r))
                        .images
                        .push(im);
                }
                self.pages = tiles.into_values().collect();
            }
        }
    }
}

fn tiles_along(len: f32, unit: f32) -> i32 {
    let unit = unit.max(1.0);
    ((len / unit) - 1e-3).ceil().max(1.0) as i32
}

fn clamp_cell(c: i32, r: i32, cols: i32, rows: i32) -> (i32, i32) {
    (
        c.clamp(0, cols.saturating_sub(1).max(0)),
        r.clamp(0, rows.saturating_sub(1).max(0)),
    )
}

fn split_stroke_across_tiles(stroke: &InkStroke, tw: f32, th: f32) -> Vec<(i32, i32, InkStroke)> {
    if stroke.points.is_empty() {
        return Vec::new();
    }
    let cell = |p: InkPoint| ((p.x / tw).floor() as i32, (p.y / th).floor() as i32);
    let mut out = Vec::new();
    let mut cur_cell = cell(stroke.points[0]);
    let mut cur = InkStroke::new(stroke.nib, stroke.color32(), stroke.width);
    cur.points.push(stroke.points[0]);
    for pair in stroke.points.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let mut hits: Vec<(f32, InkPoint)> = Vec::new();
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        if dx.abs() > 1e-8 {
            let lo = a.x.min(b.x);
            let hi = a.x.max(b.x);
            let k0 = ((lo + 1e-4) / tw).ceil() as i32;
            let k1 = ((hi - 1e-4) / tw).floor() as i32;
            for k in k0..=k1 {
                let x = k as f32 * tw;
                let t = (x - a.x) / dx;
                if t > 1e-6 && t < 1.0 - 1e-6 {
                    let y = a.y + dy * t;
                    let p = a.p + (b.p - a.p) * t;
                    hits.push((t, InkPoint::new(Pos2::new(x, y), p)));
                }
            }
        }
        if dy.abs() > 1e-8 {
            let lo = a.y.min(b.y);
            let hi = a.y.max(b.y);
            let k0 = ((lo + 1e-4) / th).ceil() as i32;
            let k1 = ((hi - 1e-4) / th).floor() as i32;
            for k in k0..=k1 {
                let y = k as f32 * th;
                let t = (y - a.y) / dy;
                if t > 1e-6 && t < 1.0 - 1e-6 {
                    let x = a.x + dx * t;
                    let p = a.p + (b.p - a.p) * t;
                    hits.push((t, InkPoint::new(Pos2::new(x, y), p)));
                }
            }
        }
        hits.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        hits.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-5);
        for (_, p) in hits {
            cur.points.push(p);
            out.push((cur_cell.0, cur_cell.1, std::mem::replace(
                &mut cur,
                InkStroke::new(stroke.nib, stroke.color32(), stroke.width),
            )));
            cur.points.push(p);
            cur_cell = cell(p);
        }
        cur.points.push(b);
        cur_cell = cell(b);
    }
    if !cur.points.is_empty() {
        out.push((cur_cell.0, cur_cell.1, cur));
    }
    out
}

pub fn stroke_from_polyline(pts: &[[f32; 2]], nib: Nib, color: Color32, width: f32) -> InkStroke {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_spreads_legacy_stack() {
        let mut n = Note::blank("t", 0);
        n.pages.push(Page::default());
        n.pages.push(Page::default());
        n.normalize_page_grid();
        assert_eq!(
            n.pages.iter().map(|p| (p.col, p.row)).collect::<Vec<_>>(),
            vec![(0, 0), (0, 1), (0, 2)]
        );
    }

    #[test]
    fn normalize_keeps_placed_grid() {
        let mut n = Note::blank("t", 0);
        n.pages = vec![Page::at(0, 0), Page::at(1, 0), Page::at(0, 1)];
        n.normalize_page_grid();
        assert_eq!(
            n.pages.iter().map(|p| (p.col, p.row)).collect::<Vec<_>>(),
            vec![(0, 0), (1, 0), (0, 1)]
        );
    }

    #[test]
    fn sheet_join_default_is_linked() {
        let n = Note::blank("t", 0);
        assert_eq!(n.sheet_join, SheetJoin::Linked);
        let parsed: Note = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000000","title":"t","paper":"Blank","cover":0,"created":"2020-01-01T00:00:00Z","updated":"2020-01-01T00:00:00Z","pinned":false,"pages":[]}"#,
        )
        .unwrap();
        assert_eq!(parsed.sheet_join, SheetJoin::Linked);
        assert!(parsed.pages.is_empty());
    }

    fn mark(x: f32, y: f32) -> InkStroke {
        let mut s = InkStroke::new(Nib::Fineliner, Color32::BLACK, 2.0);
        s.points.push(InkPoint::new(Pos2::new(x, y), 1.0));
        s
    }

    #[test]
    fn separate_splits_grown_sheet_into_unit_pages() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.page_h = PAGE_H;
        n.pages[0].strokes.push(mark(10.0, 12.0));
        n.pages[0].strokes.push(mark(PAGE_W + 40.0, 8.0));
        n.apply_sheet_join(SheetJoin::Separate);
        assert_eq!(n.sheet_join, SheetJoin::Separate);
        assert_eq!(n.page_w, PAGE_W);
        assert_eq!(n.page_h, PAGE_H);
        assert_eq!(n.pages.len(), 2);
        let cells: Vec<_> = n.pages.iter().map(|p| (p.col, p.row)).collect();
        assert_eq!(cells, vec![(0, 0), (1, 0)]);
        assert_eq!(n.pages[0].strokes[0].points[0].x, 10.0);
        assert!((n.pages[1].strokes[0].points[0].x - 40.0).abs() < 0.01);
    }

    #[test]
    fn linked_merges_grid_into_one_sheet() {
        let mut n = Note::blank("t", 0);
        n.sheet_join = SheetJoin::Separate;
        n.pages = vec![Page::at(0, 0), Page::at(1, 0)];
        n.pages[1].strokes.push(mark(40.0, 8.0));
        n.apply_sheet_join(SheetJoin::Linked);
        assert_eq!(n.pages.len(), 1);
        assert_eq!(n.page_w, PAGE_W * 2.0);
        assert_eq!(n.page_h, PAGE_H);
        assert!((n.pages[0].strokes[0].points[0].x - (PAGE_W + 40.0)).abs() < 0.01);
    }

    #[test]
    fn join_roundtrip_keeps_marks() {
        let mut n = Note::blank("t", 0);
        n.page_w = PAGE_W * 2.0;
        n.page_h = PAGE_H * 2.0;
        n.pages[0].strokes.push(mark(PAGE_W + 5.0, PAGE_H + 6.0));
        n.apply_sheet_join(SheetJoin::Separate);
        n.apply_sheet_join(SheetJoin::Linked);
        assert_eq!(n.pages.len(), 1);
        assert_eq!(n.page_w, PAGE_W * 2.0);
        assert_eq!(n.page_h, PAGE_H * 2.0);
        let p = &n.pages[0].strokes[0].points[0];
        assert!((p.x - (PAGE_W + 5.0)).abs() < 0.05);
        assert!((p.y - (PAGE_H + 6.0)).abs() < 0.05);
    }
}
