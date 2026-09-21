//! Encre vectorielle : points, pression, ruban, formes.

use egui::{Color32, Mesh, Pos2, Stroke as EStroke, TextureId, Vec2, epaint::Vertex};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Nib {
    #[default]
    Fineliner,
    Brush,
    Pencil,
    Highlighter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tool {
    #[default]
    Fineliner,
    Brush,
    Pencil,
    Highlighter,
    EraserStroke,
    EraserArea,
    Lasso,
    Text,
    Image,
}

impl Tool {
    pub fn nib(self) -> Option<Nib> {
        match self {
            Tool::Fineliner => Some(Nib::Fineliner),
            Tool::Brush => Some(Nib::Brush),
            Tool::Pencil => Some(Nib::Pencil),
            Tool::Highlighter => Some(Nib::Highlighter),
            _ => None,
        }
    }

    pub fn is_ink(self) -> bool {
        self.nib().is_some()
    }

    pub fn is_eraser(self) -> bool {
        matches!(self, Tool::EraserStroke | Tool::EraserArea)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Fineliner => "feutre",
            Tool::Brush => "plume",
            Tool::Pencil => "crayon",
            Tool::Highlighter => "surligneur",
            Tool::EraserStroke => "gomme trait",
            Tool::EraserArea => "gomme zone",
            Tool::Lasso => "lasso",
            Tool::Text => "texte",
            Tool::Image => "image",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    pub p: f32,
}

impl InkPoint {
    pub fn new(pos: Pos2, p: f32) -> Self {
        Self {
            x: pos.x,
            y: pos.y,
            p: p.clamp(0.08, 1.0),
        }
    }

    pub fn pos(self) -> Pos2 {
        Pos2::new(self.x, self.y)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InkStroke {
    pub id: Uuid,
    pub nib: Nib,
    pub color: [u8; 4],
    pub width: f32,
    pub points: Vec<InkPoint>,
    #[serde(skip)]
    pub mesh: Option<Mesh>,
}

impl InkStroke {
    pub fn new(nib: Nib, color: Color32, width: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            nib,
            color: color.to_array(),
            width,
            points: Vec::new(),
            mesh: None,
        }
    }

    pub fn color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.color[0], self.color[1], self.color[2], self.color[3])
    }

    pub fn push(&mut self, p: InkPoint) {
        if let Some(last) = self.points.last() {
            let dx = p.x - last.x;
            let dy = p.y - last.y;
            if dx * dx + dy * dy < 0.36 {
                return;
            }
        }
        self.points.push(p);
        self.mesh = None;
    }

    pub fn translate(&mut self, d: Vec2) {
        for p in &mut self.points {
            p.x += d.x;
            p.y += d.y;
        }
        self.mesh = None;
    }

    pub fn bbox(&self) -> Option<(Pos2, Pos2)> {
        let mut it = self.points.iter();
        let first = it.next()?;
        let mut min = first.pos();
        let mut max = min;
        for p in it {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        Some((min, max))
    }

    pub fn hits(&self, pos: Pos2, extra: f32) -> bool {
        let r = self.width * 0.55 + extra;
        let r2 = r * r;
        for w in self.points.windows(2) {
            if dist2_seg(pos, w[0].pos(), w[1].pos()) <= r2 {
                return true;
            }
        }
        if self.points.len() == 1 {
            return self.points[0].pos().distance_sq(pos) <= r2;
        }
        false
    }

    pub fn tessellate(&mut self) -> &Mesh {
        if self.mesh.is_none() {
            self.mesh = Some(ribbon_mesh(&self.points, self.width, self.nib, self.color32()));
        }
        self.mesh.as_ref().unwrap()
    }
}

pub fn map_mesh(src: &Mesh, map: impl Fn(Pos2) -> Pos2) -> Mesh {
    let mut m = src.clone();
    for v in &mut m.vertices {
        v.pos = map(v.pos);
    }
    m
}

pub fn ribbon_mesh(points: &[InkPoint], width: f32, nib: Nib, color: Color32) -> Mesh {
    let mut mesh = Mesh::with_texture(TextureId::default());
    if points.is_empty() {
        return mesh;
    }
    let color = premultiply(color);
    if points.len() == 1 {
        add_disc(&mut mesh, points[0].pos(), width * 0.45 * points[0].p, color);
        return mesh;
    }

    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    for (i, pt) in points.iter().enumerate() {
        let tan = if i == 0 {
            points[1].pos() - points[0].pos()
        } else if i + 1 == points.len() {
            points[i].pos() - points[i - 1].pos()
        } else {
            points[i + 1].pos() - points[i - 1].pos()
        };
        let n = rot90(tan.normalized());
        let mut press = pt.p;
        match nib {
            Nib::Fineliner => press = 1.0,
            Nib::Highlighter => press = 1.0,
            Nib::Pencil => press = 0.35 + press * 0.65,
            Nib::Brush => press = 0.18 + press * 0.82,
        }
        let mut r = width * 0.5 * press;
        if nib == Nib::Highlighter {
            r = width * 0.5;
        }
        r = r.max(0.25);
        left.push(pt.pos() + n * r);
        right.push(pt.pos() - n * r);
    }

    add_disc(&mut mesh, points[0].pos(), dist(points[0].pos(), left[0]), color);
    add_disc(
        &mut mesh,
        points.last().unwrap().pos(),
        dist(points.last().unwrap().pos(), *left.last().unwrap()),
        color,
    );

    for i in 0..points.len() - 1 {
        add_tri(&mut mesh, left[i], right[i], left[i + 1], color);
        add_tri(&mut mesh, right[i], right[i + 1], left[i + 1], color);
    }
    mesh
}

pub fn ribbon_outline(points: &[InkPoint], width: f32, nib: Nib) -> Vec<Pos2> {
    if points.is_empty() {
        return vec![];
    }
    if points.len() == 1 {
        return circle_pts(points[0].pos(), width * 0.45 * points[0].p, 14);
    }
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (i, pt) in points.iter().enumerate() {
        let tan = if i == 0 {
            points[1].pos() - points[0].pos()
        } else if i + 1 == points.len() {
            points[i].pos() - points[i - 1].pos()
        } else {
            points[i + 1].pos() - points[i - 1].pos()
        };
        let n = rot90(tan.normalized());
        let mut press = pt.p;
        match nib {
            Nib::Fineliner | Nib::Highlighter => press = 1.0,
            Nib::Pencil => press = 0.35 + press * 0.65,
            Nib::Brush => press = 0.18 + press * 0.82,
        }
        let r = (width * 0.5 * press).max(0.25);
        left.push(pt.pos() + n * r);
        right.push(pt.pos() - n * r);
    }
    left.extend(right.into_iter().rev());
    left
}

fn add_tri(mesh: &mut Mesh, a: Pos2, b: Pos2, c: Pos2, color: Color32) {
    let i = mesh.vertices.len() as u32;
    for p in [a, b, c] {
        mesh.vertices.push(Vertex {
            pos: p,
            uv: Pos2::ZERO,
            color,
        });
    }
    mesh.indices.extend_from_slice(&[i, i + 1, i + 2]);
}

fn add_disc(mesh: &mut Mesh, c: Pos2, r: f32, color: Color32) {
    let r = r.max(0.3);
    let n = 12u32;
    let start = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        pos: c,
        uv: Pos2::ZERO,
        color,
    });
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        mesh.vertices.push(Vertex {
            pos: c + Vec2::angled(a) * r,
            uv: Pos2::ZERO,
            color,
        });
    }
    for i in 0..n {
        mesh.indices.extend_from_slice(&[start, start + i + 1, start + i + 2]);
    }
}

fn circle_pts(c: Pos2, r: f32, n: usize) -> Vec<Pos2> {
    (0..n)
        .map(|i| c + Vec2::angled(i as f32 / n as f32 * std::f32::consts::TAU) * r)
        .collect()
}

fn rot90(v: Vec2) -> Vec2 {
    if v.length_sq() < 1e-8 {
        Vec2::new(0.0, -1.0)
    } else {
        Vec2::new(-v.y, v.x)
    }
}

fn dist(a: Pos2, b: Pos2) -> f32 {
    a.distance(b)
}

fn dist2_seg(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-8)).clamp(0.0, 1.0);
    (a + ab * t).distance_sq(p)
}

pub fn point_in_poly(p: Pos2, poly: &[Pos2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let pi = poly[i];
        let pj = poly[j];
        if ((pi.y > p.y) != (pj.y > p.y))
            && (p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y + 1e-8) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn premultiply(c: Color32) -> Color32 {
    let a = c.a() as u16;
    Color32::from_rgba_premultiplied(
        (c.r() as u16 * a / 255) as u8,
        (c.g() as u16 * a / 255) as u8,
        (c.b() as u16 * a / 255) as u8,
        c.a(),
    )
}

/// Pression hardware, sinon vitesse (plume : lent = plus gras).
pub fn mixed_pressure(nib: Nib, hw: Option<f32>, speed: f32) -> f32 {
    if let Some(p) = hw {
        return p.clamp(0.08, 1.0);
    }
    match nib {
        Nib::Fineliner | Nib::Highlighter => 1.0,
        Nib::Brush => {
            let t = (speed / 900.0).clamp(0.0, 1.0);
            0.95 - t * 0.55
        }
        Nib::Pencil => {
            let t = (speed / 700.0).clamp(0.0, 1.0);
            0.55 + (1.0 - t) * 0.35
        }
    }
}

pub fn maybe_snap_shape(stroke: &InkStroke, force_line: bool) -> Option<InkStroke> {
    if stroke.points.len() < 6 {
        if force_line && stroke.points.len() >= 2 {
            return Some(line_stroke(stroke));
        }
        return None;
    }
    if force_line {
        return Some(line_stroke(stroke));
    }
    let start = stroke.points[0].pos();
    let end = stroke.points.last().unwrap().pos();
    let closed = start.distance(end) < 36.0;
    if !closed {
        if line_error(stroke) < 3.2 {
            return Some(line_stroke(stroke));
        }
        return None;
    }
    if let Some(s) = fit_rect(stroke) {
        return Some(s);
    }
    if let Some(s) = fit_circle(stroke) {
        return Some(s);
    }
    None
}

fn line_stroke(stroke: &InkStroke) -> InkStroke {
    let a = stroke.points[0];
    let b = *stroke.points.last().unwrap();
    let mut s = stroke.clone();
    s.id = Uuid::new_v4();
    s.points = vec![a, InkPoint { x: b.x, y: b.y, p: a.p }];
    s.mesh = None;
    s
}

fn line_error(stroke: &InkStroke) -> f32 {
    let a = stroke.points[0].pos();
    let b = stroke.points.last().unwrap().pos();
    let mut e = 0.0f32;
    for p in &stroke.points {
        e = e.max(dist2_seg(p.pos(), a, b).sqrt());
    }
    e
}

fn fit_circle(stroke: &InkStroke) -> Option<InkStroke> {
    let n = stroke.points.len() as f32;
    let c = stroke
        .points
        .iter()
        .fold(Vec2::ZERO, |acc, p| acc + p.pos().to_vec2())
        / n;
    let c = Pos2::new(c.x, c.y);
    let r = stroke.points.iter().map(|p| p.pos().distance(c)).sum::<f32>() / n;
    if r < 12.0 {
        return None;
    }
    let err = stroke
        .points
        .iter()
        .map(|p| (p.pos().distance(c) - r).abs())
        .fold(0.0f32, f32::max);
    if err > r * 0.18 {
        return None;
    }
    let mut s = stroke.clone();
    s.id = Uuid::new_v4();
    s.points = circle_pts(c, r, 48)
        .into_iter()
        .map(|p| InkPoint::new(p, 1.0))
        .collect();
    s.mesh = None;
    Some(s)
}

fn fit_rect(stroke: &InkStroke) -> Option<InkStroke> {
    let (min, max) = stroke.bbox()?;
    let w = max.x - min.x;
    let h = max.y - min.y;
    if w < 20.0 || h < 20.0 {
        return None;
    }
    let corners = [
        Pos2::new(min.x, min.y),
        Pos2::new(max.x, min.y),
        Pos2::new(max.x, max.y),
        Pos2::new(min.x, max.y),
    ];
    let mut err = 0.0f32;
    for p in &stroke.points {
        let d = corners
            .windows(2)
            .chain(std::iter::once([corners[3], corners[0]].as_slice()))
            .map(|s| dist2_seg(p.pos(), s[0], s[1]).sqrt())
            .fold(f32::MAX, f32::min);
        err = err.max(d);
    }
    if err > 14.0 {
        return None;
    }
    let mut pts = Vec::new();
    let seq = [corners[0], corners[1], corners[2], corners[3], corners[0]];
    for w in seq.windows(2) {
        for i in 0..=8 {
            let t = i as f32 / 8.0;
            pts.push(InkPoint::new(w[0].lerp(w[1], t), 1.0));
        }
    }
    let mut s = stroke.clone();
    s.id = Uuid::new_v4();
    s.points = pts;
    s.mesh = None;
    Some(s)
}

pub fn erase_area(stroke: &InkStroke, center: Pos2, radius: f32) -> Vec<InkStroke> {
    let r2 = radius * radius;
    let mut fragments = Vec::new();
    let mut cur: Vec<InkPoint> = Vec::new();
    let flush = |cur: &mut Vec<InkPoint>, fragments: &mut Vec<InkStroke>, proto: &InkStroke| {
        if cur.len() >= 2 {
            let mut s = proto.clone();
            s.id = Uuid::new_v4();
            s.points = std::mem::take(cur);
            s.mesh = None;
            fragments.push(s);
        } else {
            cur.clear();
        }
    };
    for p in &stroke.points {
        if p.pos().distance_sq(center) > r2 {
            cur.push(*p);
        } else {
            flush(&mut cur, &mut fragments, stroke);
        }
    }
    flush(&mut cur, &mut fragments, stroke);
    fragments
}

pub fn draw_ants(painter: &egui::Painter, min: Pos2, max: Pos2, t: f32, color: Color32) {
    let rect = egui::Rect::from_min_max(min, max).expand(4.0);
    let phase = (t * 18.0) % 12.0;
    painter.rect_stroke(
        rect,
        0.0,
        EStroke::new(1.0_f32, color.gamma_multiply(0.85)),
        egui::StrokeKind::Outside,
    );
    let _ = phase;
}

pub fn default_width(nib: Nib) -> f32 {
    match nib {
        Nib::Fineliner => 2.2,
        Nib::Brush => 6.5,
        Nib::Pencil => 3.4,
        Nib::Highlighter => 16.0,
    }
}
