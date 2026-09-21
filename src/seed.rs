use crate::document::{stroke_from_polyline, squiggle, Note, PaperKind, TextBox, PAGE_W};
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
    let brouillon = Note::blank("Brouillon", 2);
    lib.insert_new(&mentalite);
    lib.insert_new(&da);
    lib.insert_new(&brouillon);
    lib.mark_seeded();
}

fn note_mentalite() -> Note {
    let mut n = Note::blank("Mentalité", 0);
    n.paper = PaperKind::Lined;
    n.pinned = true;
    let ink = Color32::from_rgb(0x1c, 0x18, 0x14);
    let accent = Color32::from_rgb(0x7f, 0xbb, 0xb3);
    let body = concat!(
        "Omarchy n'est pas un ricing kit. C'est une machine finie : Arch + Hyprland, des choix déjà faits, du beau par défaut.\n\n",
        "Opinionated — tu n'as pas à décider de 40 polices avant d'écrire.\n",
        "Local — tes notes restent sur le disque (~/.local/share/omacourses). Pas de compte, pas de sync.\n",
        "Clavier d'abord, main ensuite — le stylet quand la pensée a besoin d'un croquis, pas d'un formulaire.\n",
        "Moins d'options, plus de métier."
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
    let mut n = Note::blank("Direction artistique", 3);
    n.paper = PaperKind::Millimetre;
    n.pinned = true;
    let ink = Color32::from_rgb(0x1c, 0x18, 0x14);
    let red = Color32::from_rgb(0xe6, 0x7e, 0x80);
    let body = concat!(
        "Deux familles, pas une palette Material.\n\n",
        "1. Le pupitre — chrome Hyprland, couleurs du thème actif (ici, le colors.toml Omarchy).\n",
        "2. La feuille — objet physique. Crème, millimétré, trous de reliure. Le dark mode assombrit le bureau, pas le papier.\n\n",
        "Mono pour l'OS, serif pour le titre du cahier.\n",
        "Une scène par geste : étagère de dos toilés, trousse, règle de laiton, page arrachée à l'export.\n\n",
        "Si deux écrans se ressemblent en plissant les yeux, l'un des deux n'a pas d'objet."
    );
    n.pages[0].texts.push(text(72.0, 64.0, 17.0, body, ink));
    // petit plan : trois rectangles d'espacement (gaps Hyprland)
    n.pages[0].strokes.push(rect_stroke(520.0, 720.0, 200.0, 120.0, red, 2.4));
    n.pages[0].strokes.push(rect_stroke(530.0, 730.0, 80.0, 100.0, ink, 1.6));
    n.pages[0].strokes.push(rect_stroke(620.0, 730.0, 90.0, 100.0, ink, 1.6));
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
    stroke_from_polyline(&squiggle(Pos2::new(x, y), Pos2::new(x + w, y + 3.0), 1.6, 18), Nib::Brush, color, 3.2)
}

fn rect_stroke(x: f32, y: f32, w: f32, h: f32, color: Color32, width: f32) -> InkStroke {
    let pts = [
        [x, y],
        [x + w, y],
        [x + w, y + h],
        [x, y + h],
        [x, y],
    ];
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
