use crate::document::{squiggle, stroke_from_polyline, Note, PaperKind, TextBox, PAGE_W};
use crate::ink::{InkStroke, Nib};
use crate::library::Library;
use egui::{Color32, Pos2};

pub fn seed_if_needed(lib: &mut Library) {
    if lib.index.seeded || !lib.index.notes.is_empty() {
        if !lib.index.seeded {
            lib.mark_seeded();
        }
        return;
    }

    let mentalite = note_mentalite();
    let da = note_direction();
    let brouillon = Note::blank("Draft", 2);
    lib.insert_new(&mentalite);
    lib.insert_new(&da);
    lib.insert_new(&brouillon);
    lib.mark_seeded();
}

fn note_mentalite() -> Note {
    let mut n = Note::blank("Mindset", 0);
    n.paper = PaperKind::Lined;
    n.pinned = true;
    let ink = Color32::from_rgb(0x1c, 0x18, 0x14);
    let accent = Color32::from_rgb(0x7f, 0xbb, 0xb3);
    let body = concat!(
        "Omarchy is not a ricing kit. It is a finished machine: Arch + Hyprland, choices already made, beautiful by default.\n\n",
        "Opinionated — you don't pick 40 fonts before you write.\n",
        "Local — your notes stay on disk (~/.local/share/omacourses). No account, no sync.\n",
        "Keyboard first, hand second — the stylus when a thought needs a sketch, not a form.\n",
        "Fewer options, more craft."
    );
    n.pages[0].texts.push(text(72.0, 72.0, 18.0, body, ink));
    n.pages[0].strokes.push(underline(72.0, 58.0, 280.0, ink));
    n.pages[0].strokes.push(stroke_from_polyline(
        &squiggle(Pos2::new(640.0, 160.0), Pos2::new(720.0, 420.0), 8.0, 28),
        Nib::Brush,
        accent,
        5.0,
    ));
    n
}

fn note_direction() -> Note {
    let mut n = Note::blank("Art direction", 3);
    n.paper = PaperKind::Millimetre;
    n.pinned = true;
    let ink = Color32::from_rgb(0x1c, 0x18, 0x14);
    let red = Color32::from_rgb(0xe6, 0x7e, 0x80);
    let body = concat!(
        "Two families, not a Material palette.\n\n",
        "1. The lectern — Hyprland chrome, colors of the active theme (here, Omarchy colors.toml).\n",
        "2. The page — a physical object. Cream, graph paper, binder holes. Dark mode dims the desk, not the paper.\n\n",
        "Mono for the OS, serif for the notebook title.\n",
        "One scene per gesture: cloth-bound spines, a pencil case, a brass ruler, a page torn off at export.\n\n",
        "If two screens look the same when you squint, one of them has no object."
    );
    n.pages[0].texts.push(text(72.0, 64.0, 17.0, body, ink));
    // petit plan : trois rectangles d'espacement (gaps Hyprland)
    n.pages[0]
        .strokes
        .push(rect_stroke(520.0, 720.0, 200.0, 120.0, red, 2.4));
    n.pages[0]
        .strokes
        .push(rect_stroke(530.0, 730.0, 80.0, 100.0, ink, 1.6));
    n.pages[0]
        .strokes
        .push(rect_stroke(620.0, 730.0, 90.0, 100.0, ink, 1.6));
    n
}

fn text(x: f32, y: f32, size: f32, s: &str, color: Color32) -> TextBox {
    let mut t = TextBox::new(Pos2::new(x, y), color);
    t.size = [PAGE_W - x - 48.0, 640.0];
    t.size_pt = size;
    t.text = s.to_string();
    t
}

fn underline(x: f32, y: f32, w: f32, color: Color32) -> InkStroke {
    stroke_from_polyline(
        &squiggle(Pos2::new(x, y), Pos2::new(x + w, y + 3.0), 1.6, 18),
        Nib::Brush,
        color,
        3.2,
    )
}

fn rect_stroke(x: f32, y: f32, w: f32, h: f32, color: Color32, width: f32) -> InkStroke {
    let pts = [[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]];
    let mut dense = Vec::new();
    for pair in pts.windows(2) {
        for i in 0..=6 {
            let t = i as f32 / 6.0;
            dense.push([
                pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                pair[0][1] + (pair[1][1] - pair[0][1]) * t,
            ]);
        }
    }
    stroke_from_polyline(&dense, Nib::Fineliner, color, width)
}
