use std::io::Write;

use crate::document::{Note, PaperKind};
use crate::ink::{ribbon_outline, InkStroke};
use crate::look::Look;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use fontdue::{Font, FontSettings};
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap, Stroke as SkStroke, Transform};

pub fn raster_page(note: &Note, page_i: usize, look: &Look, scale: f32, media: &MediaLoader) -> Option<Pixmap> {
    let page = note.pages.get(page_i)?;
    let (page_w, page_h) = note.page_size();
    let w = (page_w * scale).round() as u32;
    let h = (page_h * scale).round() as u32;
    let mut pm = Pixmap::new(w, h)?;
    let paper = if note.paper == PaperKind::Slate {
        rgb(look.desk_deep)
    } else {
        rgb(look.paper)
    };
    pm.fill(paper);
    let t = Transform::from_scale(scale, scale);
    draw_template(&mut pm, note.paper, look, t, page_w, page_h);
    for im in &page.images {
        if let Some((iw, ih, rgba)) = media.rgba(&im.file) {
            blit(&mut pm, im.pos[0] * scale, im.pos[1] * scale, im.size[0] * scale, im.size[1] * scale, iw, ih, &rgba);
        }
    }
    for s in &page.strokes {
        fill_stroke(&mut pm, s, t);
    }
    if let Some(font) = serif_font() {
        let ink = if note.paper == PaperKind::Slate {
            rgb(look.fg)
        } else {
            rgb(look.ink)
        };
        for tx in &page.texts {
            draw_text(
                &mut pm,
                &font,
                &tx.text,
                tx.pos[0] * scale,
                tx.pos[1] * scale,
                tx.size[0] * scale,
                tx.size_pt * scale,
                ink,
            );
        }
    }
    Some(pm)
}

pub fn pixmap_png(pm: &Pixmap) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(pm.width(), pm.height(), pm.data().to_vec())?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

pub fn pages_pdf(note: &Note, look: &Look, media: &MediaLoader) -> Option<Vec<u8>> {
    let mut images = Vec::new();
    for i in 0..note.pages.len() {
        let pm = raster_page(note, i, look, 1.6, media)?;
        images.push((pm.width(), pm.height(), rgb_from_rgba(pm.data())));
    }
    Some(simple_pdf(&images))
}

pub struct MediaLoader {
    pub root: std::path::PathBuf,
}

impl MediaLoader {
    fn rgba(&self, file: &str) -> Option<(u32, u32, Vec<u8>)> {
        let img = image::open(self.root.join(file)).ok()?.into_rgba8();
        let (w, h) = img.dimensions();
        Some((w, h, img.into_raw()))
    }
}

fn fill_stroke(pm: &mut Pixmap, stroke: &InkStroke, t: Transform) {
    let outline = ribbon_outline(&stroke.points, stroke.width, stroke.nib);
    let Some(path) = poly_path(&outline) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(stroke.color[0], stroke.color[1], stroke.color[2], stroke.color[3]);
    paint.anti_alias = true;
    pm.fill_path(&path, &paint, FillRule::Winding, t, None);
}

fn poly_path(pts: &[egui::Pos2]) -> Option<Path> {
    if pts.len() < 3 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].x, pts[0].y);
    for p in pts.iter().skip(1) {
        pb.line_to(p.x, p.y);
    }
    pb.close();
    pb.finish()
}

fn draw_template(pm: &mut Pixmap, kind: PaperKind, look: &Look, t: Transform, page_w: f32, page_h: f32) {
    let mut paint = Paint::default();
    paint.anti_alias = true;
    let mut stroke = SkStroke::default();
    match kind {
        PaperKind::Blank | PaperKind::Slate => {}
        PaperKind::Lined => {
            paint.set_color(rgb(look.paper_rule));
            stroke.width = 0.8;
            let mut y = 88.0;
            while y < page_h - 24.0 {
                line(pm, 56.0, y, page_w - 24.0, y, &paint, &stroke, t);
                y += 28.0;
            }
            paint.set_color(rgb(look.accent));
            line(pm, 64.0, 24.0, 64.0, page_h - 24.0, &paint, &stroke, t);
        }
        PaperKind::Grid => {
            paint.set_color(rgb(look.paper_rule));
            stroke.width = 0.6;
            let mut x = 24.0;
            while x < page_w {
                line(pm, x, 24.0, x, page_h - 24.0, &paint, &stroke, t);
                x += 24.0;
            }
            let mut y = 24.0;
            while y < page_h {
                line(pm, 24.0, y, page_w - 24.0, y, &paint, &stroke, t);
                y += 24.0;
            }
        }
        PaperKind::Dots => {
            paint.set_color(rgb(look.paper_rule_strong));
            let mut y = 32.0;
            while y < page_h - 16.0 {
                let mut x = 32.0;
                while x < page_w - 16.0 {
                    if let Some(path) = {
                        let mut pb = PathBuilder::new();
                        pb.push_circle(x, y, 0.9);
                        pb.finish()
                    } {
                        pm.fill_path(&path, &paint, FillRule::Winding, t, None);
                    }
                    x += 22.0;
                }
                y += 22.0;
            }
        }
        PaperKind::Millimetre => {
            stroke.width = 0.45;
            let mut y = 40.0;
            let mut i = 0i32;
            while y < page_h - 20.0 {
                paint.set_color(if i % 5 == 0 {
                    rgb(look.paper_rule_strong)
                } else {
                    rgb(look.paper_rule)
                });
                line(pm, 48.0, y, page_w - 20.0, y, &paint, &stroke, t);
                y += 8.0;
                i += 1;
            }
            let mut x = 48.0;
            let mut i = 0i32;
            while x < page_w - 20.0 {
                paint.set_color(if i % 5 == 0 {
                    rgb(look.paper_rule_strong)
                } else {
                    rgb(look.paper_rule)
                });
                line(pm, x, 40.0, x, page_h - 20.0, &paint, &stroke, t);
                x += 8.0;
                i += 1;
            }
        }
    }
}

fn line(
    pm: &mut Pixmap,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    paint: &Paint,
    stroke: &SkStroke,
    t: Transform,
) {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, paint, stroke, t, None);
    }
}

fn rgb(c: egui::Color32) -> Color {
    Color::from_rgba8(c.r(), c.g(), c.b(), 255)
}

fn blit(pm: &mut Pixmap, x: f32, y: f32, dw: f32, dh: f32, iw: u32, ih: u32, rgba: &[u8]) {
    let Some(src) = Pixmap::from_vec(
        rgba.to_vec(),
        tiny_skia::IntSize::from_wh(iw, ih).unwrap_or(tiny_skia::IntSize::from_wh(1, 1).unwrap()),
    ) else {
        return;
    };
    let sx = dw / iw as f32;
    let sy = dh / ih as f32;
    pm.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::from_translate(x, y).pre_scale(sx, sy),
        None,
    );
}

fn draw_text(pm: &mut Pixmap, font: &Font, text: &str, x: f32, y: f32, max_w: f32, size: f32, color: Color) {
    let mut cx = x;
    let mut cy = y + size;
    let line_h = size * 1.28;
    for ch in text.chars() {
        if ch == '\n' {
            cx = x;
            cy += line_h;
            continue;
        }
        let (metrics, bitmap) = font.rasterize(ch, size);
        if cx + metrics.advance_width > x + max_w && ch != ' ' {
            cx = x;
            cy += line_h;
        }
        stamp_glyph(pm, &bitmap, metrics.width, metrics.height, cx + metrics.xmin as f32, cy - metrics.height as f32 - metrics.ymin as f32, color);
        cx += metrics.advance_width;
    }
}

fn stamp_glyph(pm: &mut Pixmap, bitmap: &[u8], gw: usize, gh: usize, x: f32, y: f32, color: Color) {
    let cr = color.red();
    let cg = color.green();
    let cb = color.blue();
    let width = pm.width();
    let height = pm.height();
    let data = pm.data_mut();
    for gy in 0..gh {
        for gx in 0..gw {
            let a = bitmap[gy * gw + gx] as f32 / 255.0;
            if a < 0.02 {
                continue;
            }
            let px = x as i32 + gx as i32;
            let py = y as i32 + gy as i32;
            if px < 0 || py < 0 || px as u32 >= width || py as u32 >= height {
                continue;
            }
            let i = ((py as u32 * width + px as u32) * 4) as usize;
            let inv = 1.0 - a;
            let dr = data[i] as f32 / 255.0;
            let dg = data[i + 1] as f32 / 255.0;
            let db = data[i + 2] as f32 / 255.0;
            let da = data[i + 3] as f32 / 255.0;
            data[i] = ((cr * a + dr * inv) * 255.0) as u8;
            data[i + 1] = ((cg * a + dg * inv) * 255.0) as u8;
            data[i + 2] = ((cb * a + db * inv) * 255.0) as u8;
            data[i + 3] = ((a + da * inv) * 255.0) as u8;
        }
    }
}

fn serif_font() -> Option<Font> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/TTF/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    ];
    for p in CANDIDATES {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(f);
            }
        }
    }
    None
}

fn rgb_from_rgba(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for px in data.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    out
}

fn simple_pdf(pages: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    let mut offsets = vec![0u32];
    let write_obj = |id: u32, payload: &[u8], body: &mut Vec<u8>, offsets: &mut Vec<u32>| {
        while offsets.len() <= id as usize {
            offsets.push(0);
        }
        offsets[id as usize] = body.len() as u32;
        body.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        body.extend_from_slice(payload);
        if !payload.ends_with(b"\n") {
            body.push(b'\n');
        }
        body.extend_from_slice(b"endobj\n");
    };

    let n = pages.len().max(1);
    let mut kids = String::from("[");
    // object layout: 1 catalog, 2 pages, 3..2+n page objs, then image+content pairs
    let pages_id = 2u32;
    let first_page_id = 3u32;
    let mut next = first_page_id + n as u32;
    let mut page_ids = Vec::new();
    let mut image_ids = Vec::new();
    let mut content_ids = Vec::new();
    for i in 0..n {
        page_ids.push(first_page_id + i as u32);
        kids.push_str(&format!("{} 0 R ", first_page_id + i as u32));
        image_ids.push(next);
        next += 1;
        content_ids.push(next);
        next += 1;
    }
    kids.push(']');

    body.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    write_obj(
        1,
        format!("<< /Type /Catalog /Pages {pages_id} 0 R >>\n").as_bytes(),
        &mut body,
        &mut offsets,
    );
    write_obj(
        pages_id,
        format!("<< /Type /Pages /Count {n} /Kids {kids} >>\n").as_bytes(),
        &mut body,
        &mut offsets,
    );

    for i in 0..n {
        let (w, h, rgb) = if pages.is_empty() {
            (595, 842, vec![255; 595 * 842 * 3])
        } else {
            let (w, h, ref rgb) = pages[i];
            (w, h, rgb.clone())
        };
        let pw = 595.0f32;
        let ph = pw * (h as f32 / w as f32);
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        let _ = enc.write_all(&rgb);
        let deflated = enc.finish().unwrap_or_default();
        let img_id = image_ids[i];
        let content_id = content_ids[i];
        let page_id = page_ids[i];
        write_obj(
            page_id,
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im{i} {img_id} 0 R >> >> /Contents {content_id} 0 R >>\n"
            )
            .as_bytes(),
            &mut body,
            &mut offsets,
        );
        let header = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            deflated.len()
        );
        let mut payload = header.into_bytes();
        payload.extend_from_slice(&deflated);
        payload.extend_from_slice(b"\nendstream\n");
        write_obj(img_id, &payload, &mut body, &mut offsets);
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im{i} Do Q\n");
        let mut cpay = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        cpay.extend_from_slice(content.as_bytes());
        cpay.extend_from_slice(b"endstream\n");
        write_obj(content_id, &cpay, &mut body, &mut offsets);
    }

    let xref_at = body.len();
    let max_id = offsets.len() - 1;
    body.extend_from_slice(format!("xref\n0 {}\n", max_id + 1).as_bytes());
    body.extend_from_slice(b"0000000000 65535 f \n");
    for id in 1..=max_id {
        let off = offsets.get(id).copied().unwrap_or(0);
        body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    body.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", max_id + 1)
            .as_bytes(),
    );
    body
}
