#![allow(float_literal_f32_fallback)]

use std::collections::HashMap;
use std::time::Instant;

use eframe::egui::*;
use uuid::Uuid;

use crate::camera::{page_at_y, page_origin, Camera};
use crate::document::{ImageObj, Note, PaperKind, TextBox, PAGE_GAP, PAGE_H, PAGE_W};
use crate::export::{self, MediaLoader};
use crate::ink::{
    default_width, draw_ants, erase_area, map_mesh, maybe_snap_shape, mixed_pressure, InkPoint,
    InkStroke, Nib, Tool,
};
use crate::library::{ensure_png, image_size, Library};
use crate::look::Look;
use crate::pressure::Pressure;
use crate::seed;
use crate::undo::UndoStack;

#[derive(Clone)]
enum Scene {
    Shelf { query: String },
    Desk,
}

struct Toast {
    msg: String,
    until: f64,
}

#[derive(Clone, Copy)]
enum SelKind {
    Stroke,
    Text,
    Image,
}

#[derive(Clone, Copy)]
struct Sel {
    page: usize,
    id: Uuid,
    kind: SelKind,
}

pub struct CahierApp {
    look: Look,
    lib: Library,
    scene: Scene,
    note: Option<Note>,
    tool: Tool,
    color_i: usize,
    width: f32,
    stylus_only: bool,
    camera: Camera,
    undo: UndoStack,
    live: Option<(usize, InkStroke)>,
    lasso: Vec<Pos2>,
    sel: Vec<Sel>,
    drag_last: Option<Pos2>,
    editing_text: Option<(usize, Uuid)>,
    dirty: bool,
    last_change: Instant,
    textures: HashMap<String, TextureHandle>,
    pressure: Pressure,
    last_ptr: Option<(Pos2, Instant)>,
    toast: Option<Toast>,
    need_fit: bool,
    title_buf: String,
}

impl CahierApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let look = Look::load();
        look.apply(&cc.egui_ctx);
        let mut lib = Library::open();
        seed::seed_if_needed(&mut lib);
        let width = default_width(Nib::Fineliner);
        let mut app = Self {
            look,
            lib,
            scene: Scene::Shelf {
                query: String::new(),
            },
            note: None,
            tool: Tool::Fineliner,
            color_i: 0,
            width,
            stylus_only: false,
            camera: Camera::default(),
            undo: UndoStack::default(),
            live: None,
            lasso: Vec::new(),
            sel: Vec::new(),
            drag_last: None,
            editing_text: None,
            dirty: false,
            last_change: Instant::now(),
            textures: HashMap::new(),
            pressure: Pressure::start(),
            last_ptr: None,
            toast: None,
            need_fit: true,
            title_buf: String::new(),
        };
        if let Ok(q) = std::env::var("CAHIER_OPEN") {
            let q = q.to_lowercase();
            if let Some(id) = app
                .lib
                .index
                .notes
                .iter()
                .find(|m| m.title.to_lowercase().contains(&q))
                .map(|m| m.id)
            {
                app.open_note(id);
            }
        }
        app
    }

    fn toast(&mut self, msg: impl Into<String>, t: f64) {
        self.toast = Some(Toast {
            msg: msg.into(),
            until: t + 2.4,
        });
    }

    fn ink_color(&self) -> Color32 {
        if self.tool == Tool::Highlighter {
            self.look.highs[self.color_i % self.look.highs.len()]
        } else {
            self.look.inks[self.color_i % self.look.inks.len()]
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_change = Instant::now();
        if let Some(n) = &mut self.note {
            n.touch();
        }
    }

    fn autosave(&mut self) {
        if !self.dirty {
            return;
        }
        if self.last_change.elapsed().as_millis() < 700 {
            return;
        }
        if let Some(n) = &self.note {
            self.lib.save_note(n);
            self.dirty = false;
        }
    }

    fn open_note(&mut self, id: Uuid) {
        self.autosave();
        if let Some(n) = self.lib.load_note(id) {
            self.title_buf = n.title.clone();
            self.note = Some(n);
            self.scene = Scene::Desk;
            self.undo.clear();
            self.sel.clear();
            self.live = None;
            self.lasso.clear();
            self.editing_text = None;
            self.need_fit = true;
            self.textures.clear();
        }
    }

    fn close_desk(&mut self) {
        self.autosave();
        if let Some(n) = self.note.take() {
            self.lib.save_note(&n);
        }
        self.scene = Scene::Shelf {
            query: String::new(),
        };
        self.undo.clear();
        self.sel.clear();
    }

    fn new_note(&mut self) {
        let cover = (self.lib.index.notes.len() as u8).wrapping_add(3);
        let n = Note::blank("Sans titre", cover);
        let id = n.id;
        self.lib.insert_new(&n);
        self.open_note(id);
    }
}

impl eframe::App for CahierApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if self.look.drifted() {
            let next = Look::load();
            if next.stamp != self.look.stamp {
                let label = next.name.clone();
                self.look = next;
                self.look.apply(ctx);
                crate::fonts::install(ctx);
                let t = ctx.input(|i| i.time);
                self.toast(format!("thème {}", label.to_lowercase()), t);
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        let t = ctx.input(|i| i.time);
        self.shortcuts(ctx);
        self.ingest_touch_pressure(ctx);

        match self.scene.clone() {
            Scene::Shelf { .. } => self.ui_shelf(ctx),
            Scene::Desk => self.ui_desk(ctx),
        }

        self.autosave();
        if let Some(toast) = &self.toast {
            if t < toast.until {
                let msg = toast.msg.clone();
                ShowToast { look: &self.look }.show(ctx, &msg);
            } else {
                self.toast = None;
            }
        }
        if self.live.is_some() || !self.lasso.is_empty() || self.dirty {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(n) = &self.note {
            self.lib.save_note(n);
        }
        self.lib.save_index();
    }
}

struct ShowToast<'a> {
    look: &'a Look,
}

impl ShowToast<'_> {
    fn show(&self, ctx: &Context, msg: &str) {
        Area::new(Id::new("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -28.0))
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(self.look.desk_deep)
                    .stroke(Stroke::new(1.0, self.look.accent))
                    .inner_margin(Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).font(self.look.mono(13.0)).color(self.look.fg));
                    });
            });
    }
}

impl CahierApp {
    fn ingest_touch_pressure(&self, ctx: &Context) {
        ctx.input(|i| {
            for ev in &i.events {
                if let Event::Touch { force: Some(f), .. } = ev {
                    self.pressure.push_touch(*f);
                }
            }
        });
    }

    fn shortcuts(&mut self, ctx: &Context) {
        let mut new_note = false;
        let mut undo = false;
        let mut redo = false;
        let mut save = false;
        let mut export_png = false;
        let mut export_pdf = false;
        let mut fit = false;
        let mut close = false;
        let mut delete_sel = false;
        let mut dup = false;
        let mut add_page = false;
        let mut width_delta: f32 = 0.0;
        let mut tool: Option<Tool> = None;
        let mut color: Option<usize> = None;
        let mut cycle_paper = false;

        ctx.input(|i| {
            let c = i.modifiers.command;
            let sh = i.modifiers.shift;
            if i.key_pressed(Key::Escape) {
                close = true;
            }
            if c && i.key_pressed(Key::N) {
                new_note = true;
            }
            if c && i.key_pressed(Key::Z) && !sh {
                undo = true;
            }
            if c && (i.key_pressed(Key::Y) || (sh && i.key_pressed(Key::Z))) {
                redo = true;
            }
            if c && i.key_pressed(Key::S) {
                save = true;
            }
            if c && sh && i.key_pressed(Key::E) {
                export_pdf = true;
            } else if c && i.key_pressed(Key::E) {
                export_png = true;
            }
            if c && i.key_pressed(Key::Num0) {
                fit = true;
            }
            if c && i.key_pressed(Key::D) {
                dup = true;
            }
            if c && i.key_pressed(Key::Enter) {
                add_page = true;
            }
            if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                if self.editing_text.is_none() {
                    delete_sel = true;
                }
            }
            if i.key_pressed(Key::OpenBracket) {
                width_delta = -0.8;
            }
            if i.key_pressed(Key::CloseBracket) {
                width_delta = 0.8;
            }
            if !c && self.editing_text.is_none() {
                if i.key_pressed(Key::P) {
                    tool = Some(Tool::Fineliner);
                }
                if i.key_pressed(Key::B) {
                    tool = Some(Tool::Brush);
                }
                if i.key_pressed(Key::C) {
                    tool = Some(Tool::Pencil);
                }
                if i.key_pressed(Key::H) {
                    tool = Some(Tool::Highlighter);
                }
                if i.key_pressed(Key::E) {
                    tool = Some(if sh {
                        Tool::EraserArea
                    } else {
                        Tool::EraserStroke
                    });
                }
                if i.key_pressed(Key::L) {
                    tool = Some(Tool::Lasso);
                }
                if i.key_pressed(Key::T) {
                    tool = Some(Tool::Text);
                }
                if i.key_pressed(Key::I) {
                    tool = Some(Tool::Image);
                }
                if i.key_pressed(Key::M) {
                    cycle_paper = true;
                }
                for (k, idx) in [
                    (Key::Num1, 0),
                    (Key::Num2, 1),
                    (Key::Num3, 2),
                    (Key::Num4, 3),
                    (Key::Num5, 4),
                    (Key::Num6, 5),
                    (Key::Num7, 6),
                    (Key::Num8, 7),
                    (Key::Num9, 8),
                ] {
                    if i.key_pressed(k) {
                        color = Some(idx);
                    }
                }
            }
            if !c {
                if let Scene::Shelf { .. } = self.scene {
                    if i.key_pressed(Key::N) {
                        new_note = true;
                    }
                }
            }
        });

        if new_note {
            self.new_note();
        }
        if close {
            match &self.scene {
                Scene::Desk => {
                    if self.editing_text.is_some() {
                        self.editing_text = None;
                    } else if !self.sel.is_empty() {
                        self.sel.clear();
                    } else {
                        self.close_desk();
                    }
                }
                Scene::Shelf { .. } => {}
            }
        }
        if let Scene::Desk = self.scene {
            if undo {
                if let Some(n) = &mut self.note {
                    if self.undo.undo(n) {
                        self.mark_dirty();
                    }
                }
            }
            if redo {
                if let Some(n) = &mut self.note {
                    if self.undo.redo(n) {
                        self.mark_dirty();
                    }
                }
            }
            if save {
                if let Some(n) = &self.note {
                    self.lib.save_note(n);
                    self.dirty = false;
                    let t = ctx.input(|i| i.time);
                    self.toast("cahier cousu", t);
                }
            }
            if export_png {
                self.export_png(ctx);
            }
            if export_pdf {
                self.export_pdf(ctx);
            }
            if fit {
                self.need_fit = true;
            }
            if delete_sel {
                self.delete_selection();
            }
            if dup {
                self.duplicate_selection();
            }
            if add_page {
                self.push_snapshot();
                if let Some(n) = &mut self.note {
                    n.add_page();
                    self.mark_dirty();
                }
            }
            if width_delta != 0.0 {
                self.width = (self.width + width_delta).clamp(0.8, 48.0);
            }
            if let Some(t) = tool {
                self.tool = t;
                if let Some(nib) = t.nib() {
                    self.width = default_width(nib);
                }
            }
            if let Some(i) = color {
                self.color_i = i;
            }
            if cycle_paper {
                if let Some(n) = &mut self.note {
                    n.paper = n.paper.cycle();
                    self.mark_dirty();
                }
            }
        }
    }

    fn push_snapshot(&mut self) {
        if let Some(n) = &self.note {
            self.undo.push(n);
        }
    }

    fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.push_snapshot();
        let sel = self.sel.clone();
        if let Some(n) = &mut self.note {
            for s in sel {
                if let Some(page) = n.pages.get_mut(s.page) {
                    match s.kind {
                        SelKind::Stroke => page.strokes.retain(|x| x.id != s.id),
                        SelKind::Text => page.texts.retain(|x| x.id != s.id),
                        SelKind::Image => page.images.retain(|x| x.id != s.id),
                    }
                }
            }
        }
        self.sel.clear();
        self.mark_dirty();
    }

    fn duplicate_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.push_snapshot();
        let sel = self.sel.clone();
        let mut neu = Vec::new();
        if let Some(n) = &mut self.note {
            for s in sel {
                let Some(page) = n.pages.get_mut(s.page) else {
                    continue;
                };
                match s.kind {
                    SelKind::Stroke => {
                        if let Some(st) = page.strokes.iter().find(|x| x.id == s.id).cloned() {
                            let mut st = st;
                            st.id = Uuid::new_v4();
                            st.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: st.id,
                                kind: SelKind::Stroke,
                            });
                            page.strokes.push(st);
                        }
                    }
                    SelKind::Text => {
                        if let Some(mut tx) = page.texts.iter().find(|x| x.id == s.id).cloned() {
                            tx.id = Uuid::new_v4();
                            tx.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: tx.id,
                                kind: SelKind::Text,
                            });
                            page.texts.push(tx);
                        }
                    }
                    SelKind::Image => {
                        if let Some(mut im) = page.images.iter().find(|x| x.id == s.id).cloned() {
                            im.id = Uuid::new_v4();
                            im.translate(vec2(16.0, 16.0));
                            neu.push(Sel {
                                page: s.page,
                                id: im.id,
                                kind: SelKind::Image,
                            });
                            page.images.push(im);
                        }
                    }
                }
            }
        }
        self.sel = neu;
        self.mark_dirty();
    }

    fn ui_shelf(&mut self, ctx: &Context) {
        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                ui.add_space(28.0);
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Cahier")
                                .font(self.look.serif(42.0))
                                .color(self.look.fg),
                        );
                        ui.label(
                            RichText::new("pupitre · notes au stylet · mentalité omarchy")
                                .font(self.look.mono(13.0))
                                .color(self.look.fg_dim),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(36.0);
                        let new = self.objet_btn(ui, "nouveau cahier", true);
                        if new {
                            self.new_note();
                        }
                    });
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    let Scene::Shelf { query } = &mut self.scene else {
                        return;
                    };
                    Frame::NONE
                        .fill(self.look.desk_deep)
                        .inner_margin(Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.set_width(320.0);
                            let te = TextEdit::singleline(query)
                                .hint_text("chercher un dos…")
                                .font(self.look.mono(14.0))
                                .frame(false);
                            ui.add(te);
                        });
                    ui.label(
                        RichText::new(format!("thème {}", self.look.name.to_lowercase()))
                            .font(self.look.mono(12.0))
                            .color(self.look.muted),
                    );
                });
                ui.add_space(28.0);

                let query = match &self.scene {
                    Scene::Shelf { query } => query.to_lowercase(),
                    _ => String::new(),
                };
                let mut notes: Vec<_> = self
                    .lib
                    .index
                    .notes
                    .iter()
                    .filter(|m| query.is_empty() || m.title.to_lowercase().contains(&query))
                    .cloned()
                    .collect();
                if notes.is_empty() {
                    ui.add_space(40.0);
                    ui.horizontal(|ui| {
                        ui.add_space(36.0);
                        ui.label(
                            RichText::new("l'étagère est vide — n pour un cahier")
                                .font(self.look.serif(20.0))
                                .color(self.look.fg_dim),
                        );
                    });
                    return;
                }

                let mut open = None;
                let mut del = None;
                let mut dup = None;
                let mut pin = None;

                ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(4.0);
                    let available = ui.available_width() - 48.0;
                    let card_w = 168.0;
                    let gap = 22.0;
                    let cols = ((available + gap) / (card_w + gap)).floor().max(1.0) as usize;
                    let mut i = 0;
                    while i < notes.len() {
                        ui.horizontal(|ui| {
                            ui.add_space(36.0);
                            for _ in 0..cols {
                                if i >= notes.len() {
                                    break;
                                }
                                let meta = notes[i].clone();
                                i += 1;
                                let r = self.cahier_dos(ui, &meta);
                                if r.clicked() {
                                    open = Some(meta.id);
                                }
                                r.context_menu(|ui| {
                                    if ui.button("ouvrir").clicked() {
                                        open = Some(meta.id);
                                        ui.close();
                                    }
                                    if ui.button("dupliquer").clicked() {
                                        dup = Some(meta.id);
                                        ui.close();
                                    }
                                    if ui.button(if meta.pinned { "détacher" } else { "épingler" }).clicked() {
                                        pin = Some(meta.id);
                                        ui.close();
                                    }
                                    if ui.button("jeter").clicked() {
                                        del = Some(meta.id);
                                        ui.close();
                                    }
                                });
                                ui.add_space(gap);
                            }
                        });
                        ui.add_space(22.0);
                    }
                    let _ = &mut notes;
                });

                if let Some(id) = open {
                    self.open_note(id);
                }
                if let Some(id) = dup {
                    if let Some(n) = self.lib.duplicate(id) {
                        let nid = n.id;
                        self.open_note(nid);
                    }
                }
                if let Some(id) = pin {
                    if let Some(mut n) = self.lib.load_note(id) {
                        n.pinned = !n.pinned;
                        self.lib.save_note(&n);
                    }
                }
                if let Some(id) = del {
                    self.lib.delete_note(id);
                }
            });
    }

    fn cahier_dos(&mut self, ui: &mut Ui, meta: &crate::library::NoteMeta) -> Response {
        let size = vec2(168.0, 236.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let cloth = self.look.cloth_at(meta.cover);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect.translate(vec2(4.0, 5.0)), 0.0, self.look.shadow);
        painter.rect_filled(rect, 0.0, cloth);
        let spine = Rect::from_min_max(rect.min, pos2(rect.min.x + 16.0, rect.max.y));
        painter.rect_filled(spine, 0.0, self.look.desk_deep.gamma_multiply(0.55));
        for y in [rect.min.y + 28.0, rect.center().y, rect.max.y - 28.0] {
            painter.circle_filled(pos2(rect.min.x + 8.0, y), 3.2, self.look.punch);
            painter.circle_stroke(pos2(rect.min.x + 8.0, y), 3.2, Stroke::new(1.0, self.look.desk_deep));
        }
        let inner = Rect::from_min_max(
            pos2(rect.min.x + 22.0, rect.min.y + 18.0),
            pos2(rect.max.x - 12.0, rect.max.y - 18.0),
        );
        painter.rect_filled(inner, 0.0, self.look.paper);
        painter.line_segment(
            [pos2(inner.min.x + 8.0, inner.min.y + 36.0), pos2(inner.max.x - 8.0, inner.min.y + 38.0)],
            Stroke::new(1.4, self.look.ink.gamma_multiply(0.55)),
        );
        painter.text(
            pos2(inner.min.x + 10.0, inner.min.y + 48.0),
            Align2::LEFT_TOP,
            &meta.title,
            self.look.serif(16.0),
            self.look.ink,
        );
        let date = meta.updated.format("%d %b %Y").to_string().to_lowercase();
        painter.text(
            pos2(inner.min.x + 10.0, inner.max.y - 28.0),
            Align2::LEFT_BOTTOM,
            date,
            self.look.mono(10.0),
            self.look.ink.gamma_multiply(0.55),
        );
        if meta.pinned {
            painter.circle_filled(pos2(rect.max.x - 18.0, rect.min.y + 16.0), 4.0, self.look.accent);
        }
        if resp.hovered() {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.5, self.look.accent), StrokeKind::Outside);
        }
        resp
    }

    fn objet_btn(&self, ui: &mut Ui, label: &str, accent: bool) -> bool {
        let galley = ui.painter().layout_no_wrap(
            label.to_string(),
            self.look.mono(13.0),
            if accent { self.look.desk_deep } else { self.look.fg },
        );
        let size = vec2(galley.size().x + 28.0, 34.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let fill = if accent {
            self.look.accent
        } else if resp.hovered() {
            self.look.desk_edge
        } else {
            self.look.desk_deep
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            self.look.mono(13.0),
            if accent { self.look.desk_deep } else { self.look.fg },
        );
        resp.clicked()
    }

    fn ui_desk(&mut self, ctx: &Context) {
        self.handle_drops_and_paste(ctx);

        TopBottomPanel::top("rule")
            .exact_height(52.0)
            .frame(Frame::NONE.fill(self.look.desk_deep))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    if self.objet_btn(ui, "étagère", false) {
                        self.close_desk();
                        return;
                    }
                    ui.add_space(12.0);
                    ui.painter().vline(
                        ui.cursor().left() + 4.0,
                        Rangef::new(ui.max_rect().min.y + 10.0, ui.max_rect().max.y - 10.0),
                        Stroke::new(1.0, self.look.muted),
                    );
                    ui.add_space(16.0);
                    let te = TextEdit::singleline(&mut self.title_buf)
                        .font(self.look.serif(20.0))
                        .desired_width(280.0)
                        .frame(false);
                    if ui.add(te).changed() {
                        if let Some(n) = &mut self.note {
                            n.title = self.title_buf.clone();
                            self.mark_dirty();
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        if self.objet_btn(ui, "pdf", false) {
                            self.export_pdf(ctx);
                        }
                        if self.objet_btn(ui, "png", false) {
                            self.export_png(ctx);
                        }
                        let paper_l = self
                            .note
                            .as_ref()
                            .map(|n| n.paper.label())
                            .unwrap_or("papier");
                        if self.objet_btn(ui, paper_l, false) {
                            if let Some(n) = &mut self.note {
                                n.paper = n.paper.cycle();
                                self.mark_dirty();
                            }
                        }
                        let z = format!("{}%", (self.camera.zoom * 100.0) as i32);
                        if self.objet_btn(ui, &z, false) {
                            self.need_fit = true;
                        }
                        if self.objet_btn(ui, "+ page", false) {
                            self.push_snapshot();
                            if let Some(n) = &mut self.note {
                                n.add_page();
                                self.mark_dirty();
                            }
                        }
                    });
                });
            });

        SidePanel::left("trousse")
            .exact_width(88.0)
            .frame(Frame::NONE.fill(self.look.desk_deep))
            .show(ctx, |ui| {
                self.ui_trousse(ui);
            });

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                self.ui_canvas(ui);
            });
    }

    fn ui_trousse(&mut self, ui: &mut Ui) {
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("trousse")
                    .font(self.look.mono(10.0))
                    .color(self.look.fg_dim),
            );
        });
        ui.add_space(8.0);
        let tools = [
            Tool::Fineliner,
            Tool::Brush,
            Tool::Pencil,
            Tool::Highlighter,
            Tool::EraserStroke,
            Tool::EraserArea,
            Tool::Lasso,
            Tool::Text,
            Tool::Image,
        ];
        for t in tools {
            if self.tool_glyph(ui, t) {
                self.tool = t;
                if let Some(nib) = t.nib() {
                    self.width = default_width(nib);
                }
            }
        }
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("encre")
                    .font(self.look.mono(10.0))
                    .color(self.look.fg_dim),
            );
        });
        let colors: Vec<Color32> = if self.tool == Tool::Highlighter {
            self.look.highs.clone()
        } else {
            self.look.inks.clone()
        };
        ui.add_space(4.0);
        for (i, c) in colors.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 18.0), Sense::click());
                ui.painter().rect_filled(rect, 0.0, *c);
                if i == self.color_i % colors.len() {
                    ui.painter()
                        .rect_stroke(rect, 0.0, Stroke::new(1.5, self.look.fg), StrokeKind::Outside);
                }
                if resp.clicked() {
                    self.color_i = i;
                }
            });
            ui.add_space(3.0);
        }
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(format!("{:.1} mm", self.width * 0.26))
                    .font(self.look.mono(10.0))
                    .color(self.look.fg_dim),
            );
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            if self.objet_btn(ui, if self.stylus_only { "stylet" } else { "main" }, false) {
                self.stylus_only = !self.stylus_only;
            }
        });
    }

    fn tool_glyph(&self, ui: &mut Ui, tool: Tool) -> bool {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(52.0, 36.0), Sense::click());
            let active = self.tool == tool;
            let bg = if active {
                self.look.accent
            } else if resp.hovered() {
                self.look.desk_edge
            } else {
                self.look.desk
            };
            let fg = if active {
                self.look.desk_deep
            } else {
                self.look.fg
            };
            let p = ui.painter();
            p.rect_filled(rect, 0.0, bg);
            let c = rect.center();
            match tool {
                Tool::Fineliner => {
                    p.line_segment([pos2(c.x - 10.0, c.y + 8.0), pos2(c.x + 12.0, c.y - 8.0)], Stroke::new(2.0, fg));
                    p.circle_filled(pos2(c.x + 12.0, c.y - 8.0), 2.0, fg);
                }
                Tool::Brush => {
                    p.add(Shape::convex_polygon(
                        vec![
                            pos2(c.x - 8.0, c.y + 10.0),
                            pos2(c.x - 2.0, c.y - 10.0),
                            pos2(c.x + 4.0, c.y - 10.0),
                            pos2(c.x + 10.0, c.y + 10.0),
                        ],
                        fg,
                        Stroke::NONE,
                    ));
                }
                Tool::Pencil => {
                    p.add(Shape::convex_polygon(
                        vec![
                            pos2(c.x - 12.0, c.y + 8.0),
                            pos2(c.x + 6.0, c.y - 10.0),
                            pos2(c.x + 12.0, c.y - 6.0),
                            pos2(c.x - 6.0, c.y + 12.0),
                        ],
                        fg,
                        Stroke::NONE,
                    ));
                }
                Tool::Highlighter => {
                    p.rect_filled(
                        Rect::from_center_size(c, vec2(22.0, 10.0)),
                        0.0,
                        fg.gamma_multiply(0.55),
                    );
                }
                Tool::EraserStroke => {
                    p.rect_filled(Rect::from_center_size(c, vec2(18.0, 12.0)), 2.0, fg);
                }
                Tool::EraserArea => {
                    p.circle_stroke(c, 10.0, Stroke::new(1.6, fg));
                }
                Tool::Lasso => {
                    p.circle_stroke(c, 9.0, Stroke::new(1.4, fg));
                    p.line_segment([pos2(c.x + 6.0, c.y + 6.0), pos2(c.x + 12.0, c.y + 12.0)], Stroke::new(1.4, fg));
                }
                Tool::Text => {
                    p.text(c, Align2::CENTER_CENTER, "Aa", self.look.serif(16.0), fg);
                }
                Tool::Image => {
                    p.rect_stroke(
                        Rect::from_center_size(c, vec2(20.0, 14.0)),
                        0.0,
                        Stroke::new(1.4, fg),
                        StrokeKind::Inside,
                    );
                    p.circle_filled(pos2(c.x - 4.0, c.y - 2.0), 2.0, fg);
                }
            }
            resp.on_hover_text(tool.label()).clicked()
        })
        .inner
    }

    fn ui_canvas(&mut self, ui: &mut Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        if self.need_fit && rect.width() > 10.0 {
            self.camera.fit_page(rect, 0);
            self.need_fit = false;
        }

        self.handle_camera(ui, &resp, rect);
        self.handle_tool(ui, &resp, rect);
        self.paint_world(&painter, rect, ui.input(|i| i.time) as f32);
        self.paint_overlays(ui, &painter, rect);
        self.cursor_for_tool(ui, &resp);
    }

    fn handle_camera(&mut self, ui: &Ui, resp: &Response, rect: Rect) {
        let space = ui.input(|i| i.key_down(Key::Space));
        let middle = ui.input(|i| i.pointer.middle_down());
        let ctrl = ui.input(|i| i.modifiers.command);
        if let Some(hover) = resp.hover_pos() {
            let scroll = ui.input(|i| i.raw_scroll_delta);
            let zoom_ev = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    Event::Zoom(z) => Some(*z),
                    _ => None,
                })
            });
            if let Some(z) = zoom_ev {
                self.camera.zoom_at(hover, rect, z);
            } else if ctrl && scroll.y.abs() > 0.0 {
                let f = (1.0 + scroll.y * 0.004).clamp(0.5, 1.8);
                self.camera.zoom_at(hover, rect, f);
            } else if !ctrl && scroll != Vec2::ZERO {
                self.camera.pan += scroll;
            }
        }
        let pan = space || middle;
        if pan && resp.dragged() {
            self.camera.pan += resp.drag_delta();
        }
        let mut n_touch = 0;
        ui.input(|i| {
            for e in &i.events {
                if let Event::Touch { phase, .. } = e {
                    if *phase != TouchPhase::End && *phase != TouchPhase::Cancel {
                        n_touch += 1;
                    }
                }
            }
        });
        if n_touch >= 2 && resp.dragged() {
            self.camera.pan += resp.drag_delta();
        }
    }

    fn pointer_ok(&self, ui: &Ui) -> bool {
        if !self.stylus_only {
            return true;
        }
        ui.input(|i| {
            i.events.iter().any(|e| matches!(e, Event::Touch { force: Some(_), .. }))
        })
    }

    fn handle_tool(&mut self, ui: &Ui, resp: &Response, rect: Rect) {
        let space = ui.input(|i| i.key_down(Key::Space));
        if space || ui.input(|i| i.pointer.middle_down()) {
            return;
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let secondary = ui.input(|i| i.pointer.secondary_down());
        let mut tool = self.tool;
        if secondary {
            tool = Tool::EraserStroke;
        }
        let pos = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
        let Some(screen) = pos else {
            return;
        };
        if !rect.contains(screen) {
            return;
        }
        let paper = self.camera.to_paper(screen, rect);
        let n_pages = self.note.as_ref().map(|n| n.pages.len()).unwrap_or(1);
        let page = page_at_y(paper.y, n_pages);
        let local = paper - page_origin(page);

        if tool.is_ink() && self.pointer_ok(ui) && ui.input(|i| i.pointer.primary_down()) && !resp.dragged_by(PointerButton::Middle) {
            if self.live.is_none() && ui.input(|i| i.pointer.primary_pressed()) {
                self.push_snapshot();
                let nib = tool.nib().unwrap();
                let mut s = InkStroke::new(nib, self.ink_color(), self.width);
                s.push(InkPoint::new(local, 0.7));
                self.live = Some((page, s));
            }
            let speed = self.speed(local);
            let hw = self.pressure.latest();
            if let Some((p, s)) = &mut self.live {
                if *p == page {
                    let pr = mixed_pressure(s.nib, hw, speed);
                    s.push(InkPoint::new(local, pr));
                }
            }
            // extra samples
            if let Some((pgi, live)) = &mut self.live {
                ui.input(|i| {
                    for ev in &i.events {
                        if let Event::PointerMoved(sp) = ev {
                            let pp = self.camera.to_paper(*sp, rect);
                            if page_at_y(pp.y, n_pages) == *pgi {
                                let loc = pp - page_origin(*pgi);
                                live.push(InkPoint::new(loc, mixed_pressure(live.nib, self.pressure.latest(), 200.0)));
                            }
                        }
                    }
                });
            }
        } else if self.live.is_some() && ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down()) {
            if let Some((p, mut s)) = self.live.take() {
                if let Some(snapped) = maybe_snap_shape(&s, shift) {
                    s = snapped;
                    let t = ui.input(|i| i.time);
                    self.toast("forme", t);
                }
                if s.points.len() >= 1 {
                    if let Some(n) = &mut self.note {
                        if let Some(page) = n.pages.get_mut(p) {
                            page.strokes.push(s);
                        }
                    }
                    self.mark_dirty();
                }
            }
        }

        if tool.is_eraser() && ui.input(|i| i.pointer.primary_down() || i.pointer.secondary_down()) {
            if ui.input(|i| i.pointer.primary_pressed() || i.pointer.secondary_pressed()) {
                self.push_snapshot();
            }
            let r = if tool == Tool::EraserArea {
                self.width.max(10.0) * 1.8
            } else {
                self.width.max(8.0)
            };
            let mut erased = false;
            if let Some(n) = &mut self.note {
                if let Some(page) = n.pages.get_mut(page) {
                    if tool == Tool::EraserStroke {
                        let hit: Vec<Uuid> = page
                            .strokes
                            .iter()
                            .filter(|s| s.hits(local, r))
                            .map(|s| s.id)
                            .collect();
                        if !hit.is_empty() {
                            page.strokes.retain(|s| !hit.contains(&s.id));
                            erased = true;
                        }
                    } else {
                        let mut neu = Vec::new();
                        let mut changed = false;
                        for s in page.strokes.drain(..) {
                            let parts = erase_area(&s, local, r);
                            if parts.len() != 1 || parts[0].points.len() != s.points.len() {
                                changed = true;
                            }
                            neu.extend(parts);
                        }
                        page.strokes = neu;
                        erased = changed;
                    }
                }
            }
            if erased {
                self.dirty = true;
                self.last_change = Instant::now();
            }
        }

        if tool == Tool::Lasso {
            let pressed = ui.input(|i| i.pointer.primary_pressed());
            let down = ui.input(|i| i.pointer.primary_down());
            let released = ui.input(|i| i.pointer.primary_released());
            if pressed {
                if self.sel_contains(paper) {
                    self.push_snapshot();
                    self.drag_last = Some(paper);
                    self.lasso.clear();
                } else {
                    self.lasso.clear();
                    self.sel.clear();
                    self.lasso.push(paper);
                    self.drag_last = None;
                }
            } else if down {
                if let Some(last) = self.drag_last {
                    let d = paper - last;
                    self.translate_sel(d);
                    self.drag_last = Some(paper);
                    self.mark_dirty();
                } else if self.lasso.last().map(|p| p.distance(paper) > 2.0).unwrap_or(true) {
                    self.lasso.push(paper);
                }
            } else if released {
                self.drag_last = None;
                if self.lasso.len() > 2 {
                    self.sel = self.hits_lasso();
                    self.lasso.clear();
                }
            }
        } else if !ui.input(|i| i.pointer.primary_down()) {
            self.drag_last = None;
        }

        if tool == Tool::Text && resp.clicked() {
            if let Some((pg, id)) = self.hit_text(page, local) {
                self.editing_text = Some((pg, id));
            } else {
                self.push_snapshot();
                let tx = TextBox::new(local, self.ink_color());
                let id = tx.id;
                if let Some(n) = &mut self.note {
                    if let Some(pg) = n.pages.get_mut(page) {
                        pg.texts.push(tx);
                    }
                }
                self.editing_text = Some((page, id));
                self.mark_dirty();
            }
        }

        if tool == Tool::Image && resp.clicked() {
            self.pick_image(page, local);
        }
    }

    fn speed(&mut self, local: Pos2) -> f32 {
        let now = Instant::now();
        let speed = if let Some((prev, t)) = self.last_ptr {
            let dt = now.saturating_duration_since(t).as_secs_f32().max(1e-3);
            prev.distance(local) / dt
        } else {
            200.0
        };
        self.last_ptr = Some((local, now));
        speed
    }

    fn sel_contains(&self, paper: Pos2) -> bool {
        let Some(n) = &self.note else {
            return false;
        };
        for s in &self.sel {
            let Some(page) = n.pages.get(s.page) else {
                continue;
            };
            let o = page_origin(s.page);
            let hit = match s.kind {
                SelKind::Stroke => page
                    .strokes
                    .iter()
                    .find(|x| x.id == s.id)
                    .and_then(|st| st.bbox())
                    .map(|(a, b)| egui::Rect::from_min_max(a + o, b + o).expand(12.0).contains(paper))
                    .unwrap_or(false),
                SelKind::Text => page
                    .texts
                    .iter()
                    .find(|x| x.id == s.id)
                    .map(|t| t.rect().translate(o).contains(paper))
                    .unwrap_or(false),
                SelKind::Image => page
                    .images
                    .iter()
                    .find(|x| x.id == s.id)
                    .map(|t| t.rect().translate(o).contains(paper))
                    .unwrap_or(false),
            };
            if hit {
                return true;
            }
        }
        false
    }

    fn hits_lasso(&self) -> Vec<Sel> {
        let poly = &self.lasso;
        let mut out = Vec::new();
        let Some(n) = &self.note else {
            return out;
        };
        for (pi, page) in n.pages.iter().enumerate() {
            let o = page_origin(pi);
            for s in &page.strokes {
                let world: Vec<Pos2> = s.points.iter().map(|p| p.pos() + o).collect();
                if world.iter().any(|p| crate::ink::point_in_poly(*p, poly)) {
                    out.push(Sel {
                        page: pi,
                        id: s.id,
                        kind: SelKind::Stroke,
                    });
                }
            }
            for t in &page.texts {
                if crate::ink::point_in_poly(t.min() + o, poly) {
                    out.push(Sel {
                        page: pi,
                        id: t.id,
                        kind: SelKind::Text,
                    });
                }
            }
            for im in &page.images {
                if crate::ink::point_in_poly(im.min() + o, poly) {
                    out.push(Sel {
                        page: pi,
                        id: im.id,
                        kind: SelKind::Image,
                    });
                }
            }
        }
        out
    }

    fn translate_sel(&mut self, d: Vec2) {
        let sel = self.sel.clone();
        if let Some(n) = &mut self.note {
            for s in sel {
                if let Some(page) = n.pages.get_mut(s.page) {
                    match s.kind {
                        SelKind::Stroke => {
                            if let Some(st) = page.strokes.iter_mut().find(|x| x.id == s.id) {
                                st.translate(d);
                            }
                        }
                        SelKind::Text => {
                            if let Some(t) = page.texts.iter_mut().find(|x| x.id == s.id) {
                                t.translate(d);
                            }
                        }
                        SelKind::Image => {
                            if let Some(im) = page.images.iter_mut().find(|x| x.id == s.id) {
                                im.translate(d);
                            }
                        }
                    }
                }
            }
        }
    }

    fn hit_text(&self, page: usize, local: Pos2) -> Option<(usize, Uuid)> {
        let n = self.note.as_ref()?;
        let pg = n.pages.get(page)?;
        pg.texts.iter().rev().find(|t| t.contains(local)).map(|t| (page, t.id))
    }

    fn pick_image(&mut self, page: usize, local: Pos2) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("images", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        self.insert_image_bytes(&bytes, page, local);
    }

    fn insert_image_bytes(&mut self, bytes: &[u8], page: usize, local: Pos2) {
        let Some(png) = ensure_png(bytes) else {
            return;
        };
        let Some(note) = &self.note else {
            return;
        };
        let id = note.id;
        let Some(file) = self.lib.write_media(id, &png) else {
            return;
        };
        let path = self.lib.media_path(id, &file);
        let (w, h) = image_size(&path).unwrap_or((400.0, 300.0));
        let scale = (420.0 / w.max(h)).min(1.0);
        self.push_snapshot();
        if let Some(n) = &mut self.note {
            if let Some(pg) = n.pages.get_mut(page) {
                pg.images.push(ImageObj {
                    id: Uuid::new_v4(),
                    pos: [local.x, local.y],
                    size: [w * scale, h * scale],
                    file,
                });
            }
        }
        self.mark_dirty();
        self.textures.clear();
    }

    fn handle_drops_and_paste(&mut self, ctx: &Context) {
        let dropped: Vec<Vec<u8>> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| {
                    if let Some(p) = &f.path {
                        std::fs::read(p).ok()
                    } else {
                        f.bytes.as_ref().map(|b| b.to_vec())
                    }
                })
                .collect()
        });
        if !dropped.is_empty() {
            if let Scene::Desk = self.scene {
                for b in dropped {
                    self.insert_image_bytes(&b, 0, pos2(80.0, 80.0));
                }
            }
        }
        let paste_img = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::V));
        if paste_img {
            if let Ok(mut clip) = arboard::Clipboard::new() {
                if let Ok(img) = clip.get_image() {
                    let mut rgba = Vec::with_capacity(img.bytes.len());
                    rgba.extend_from_slice(&img.bytes);
                    let mut png = Vec::new();
                    if image::RgbaImage::from_raw(img.width as u32, img.height as u32, rgba)
                        .and_then(|im| {
                            image::DynamicImage::ImageRgba8(im)
                                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                                .ok()
                        })
                        .is_some()
                    {
                        if let Scene::Desk = self.scene {
                            self.insert_image_bytes(&png, 0, pos2(90.0, 90.0));
                        }
                    }
                }
            }
        }
    }

    fn paint_world(&mut self, painter: &Painter, rect: Rect, time: f32) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let cam = self.camera;
        let map = |p: Pos2| cam.to_screen(p, rect);
        for (pi, page) in note.pages.iter().enumerate() {
            let origin = page_origin(pi);
            let min = map(Pos2::new(origin.x, origin.y));
            let max = map(Pos2::new(origin.x + PAGE_W, origin.y + PAGE_H));
            let paper = Rect::from_min_max(min, max);
            painter.rect_filled(paper.translate(vec2(8.0, 10.0)), 0.0, self.look.shadow);
            let fill = if note.paper == PaperKind::Slate {
                self.look.desk_deep
            } else {
                self.look.paper
            };
            painter.rect_filled(paper, 0.0, fill);
            self.paint_template(painter, paper, note.paper, cam.zoom);
            self.paint_punches(painter, paper);
            let fold = [
                pos2(paper.max.x, paper.min.y),
                pos2(paper.max.x - 22.0 * cam.zoom, paper.min.y),
                pos2(paper.max.x, paper.min.y + 22.0 * cam.zoom),
            ];
            painter.add(Shape::convex_polygon(
                fold.to_vec(),
                self.look.paper_rule_strong,
                Stroke::NONE,
            ));

            for im in &page.images {
                let r = Rect::from_min_size(
                    map(im.min() + origin),
                    vec2(im.size[0] * cam.zoom, im.size[1] * cam.zoom),
                );
                if let Some(tex) = self.texture_for(painter.ctx(), &note.id, &im.file) {
                    painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    painter.rect_filled(r, 0.0, self.look.paper_rule);
                }
            }

            if let Some(n) = &mut self.note {
                if let Some(pg) = n.pages.get_mut(pi) {
                    for s in &mut pg.strokes {
                        let mesh = s.tessellate().clone();
                        painter.add(Shape::mesh(map_mesh(&mesh, |p| map(p + origin))));
                    }
                }
            }

            let ink_t = if note.paper == PaperKind::Slate {
                self.look.fg
            } else {
                self.look.ink
            };
            for tx in &page.texts {
                if self.editing_text == Some((pi, tx.id)) {
                    continue;
                }
                let pos = map(tx.min() + origin);
                let max_w = (tx.size[0] * cam.zoom).max(8.0);
                let font = FontId::new(
                    tx.size_pt * cam.zoom,
                    FontFamily::Name("serif".into()),
                );
                let galley = painter.layout(tx.text.clone(), font, tx.color32(), max_w);
                painter.galley(pos, galley, tx.color32());
                let _ = ink_t;
            }

            painter.text(
                pos2(paper.center().x, paper.max.y + 18.0 * cam.zoom.min(1.2)),
                Align2::CENTER_TOP,
                format!("{} / {}", pi + 1, note.pages.len()),
                self.look.mono(11.0),
                self.look.fg_dim,
            );
        }

        if let Some((pi, live)) = &mut self.live {
            let origin = page_origin(*pi);
            let mesh = live.tessellate().clone();
            painter.add(Shape::mesh(map_mesh(&mesh, |p| map(p + origin))));
        }
        if self.lasso.len() >= 2 {
            let pts: Vec<_> = self.lasso.iter().copied().map(map).collect();
            painter.add(Shape::closed_line(pts, Stroke::new(1.2, self.look.accent)));
        }
        for s in &self.sel {
            if let Some(n) = &self.note {
                if let Some(page) = n.pages.get(s.page) {
                    let o = page_origin(s.page);
                    let bb = match s.kind {
                        SelKind::Stroke => page.strokes.iter().find(|x| x.id == s.id).and_then(|st| st.bbox()),
                        SelKind::Text => page.texts.iter().find(|x| x.id == s.id).map(|t| {
                            let r = t.rect();
                            (r.min, r.max)
                        }),
                        SelKind::Image => page.images.iter().find(|x| x.id == s.id).map(|t| {
                            let r = t.rect();
                            (r.min, r.max)
                        }),
                    };
                    if let Some((a, b)) = bb {
                        draw_ants(painter, map(a + o), map(b + o), time, self.look.accent);
                    }
                }
            }
        }

        if note.pages.len() == note.pages.len() {
            // keep used
        }
        let gap_y = map(pos2(0.0, note.pages.len() as f32 * (PAGE_H + PAGE_GAP) - PAGE_GAP + 12.0));
        let _ = gap_y;
    }

    fn paint_template(&self, painter: &Painter, paper: Rect, kind: PaperKind, zoom: f32) {
        let z = zoom;
        match kind {
            PaperKind::Blank | PaperKind::Slate => {}
            PaperKind::Lined => {
                let mut y = paper.min.y + 88.0 * z;
                while y < paper.max.y - 24.0 * z {
                    painter.line_segment(
                        [pos2(paper.min.x + 56.0 * z, y), pos2(paper.max.x - 24.0 * z, y)],
                        Stroke::new(1.0, self.look.paper_rule),
                    );
                    y += 28.0 * z;
                }
                painter.line_segment(
                    [
                        pos2(paper.min.x + 64.0 * z, paper.min.y + 24.0 * z),
                        pos2(paper.min.x + 64.0 * z, paper.max.y - 24.0 * z),
                    ],
                    Stroke::new(1.2, self.look.accent.gamma_multiply(0.45)),
                );
            }
            PaperKind::Grid => {
                let mut x = paper.min.x + 24.0 * z;
                while x < paper.max.x {
                    painter.line_segment(
                        [pos2(x, paper.min.y + 24.0 * z), pos2(x, paper.max.y - 24.0 * z)],
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    x += 24.0 * z;
                }
                let mut y = paper.min.y + 24.0 * z;
                while y < paper.max.y {
                    painter.line_segment(
                        [pos2(paper.min.x + 24.0 * z, y), pos2(paper.max.x - 24.0 * z, y)],
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    y += 24.0 * z;
                }
            }
            PaperKind::Dots => {
                let mut y = paper.min.y + 32.0 * z;
                while y < paper.max.y - 16.0 * z {
                    let mut x = paper.min.x + 32.0 * z;
                    while x < paper.max.x - 16.0 * z {
                        painter.circle_filled(pos2(x, y), 1.1 * z.max(0.6), self.look.paper_rule_strong);
                        x += 22.0 * z;
                    }
                    y += 22.0 * z;
                }
            }
            PaperKind::Millimetre => {
                let mut i = 0;
                let mut y = paper.min.y + 40.0 * z;
                while y < paper.max.y - 20.0 * z {
                    let c = if i % 5 == 0 {
                        self.look.paper_rule_strong
                    } else {
                        self.look.paper_rule
                    };
                    painter.line_segment(
                        [pos2(paper.min.x + 48.0 * z, y), pos2(paper.max.x - 20.0 * z, y)],
                        Stroke::new(if i % 5 == 0 { 1.0 } else { 0.6 }, c),
                    );
                    y += 8.0 * z;
                    i += 1;
                }
                let mut i = 0;
                let mut x = paper.min.x + 48.0 * z;
                while x < paper.max.x - 20.0 * z {
                    let c = if i % 5 == 0 {
                        self.look.paper_rule_strong
                    } else {
                        self.look.paper_rule
                    };
                    painter.line_segment(
                        [pos2(x, paper.min.y + 40.0 * z), pos2(x, paper.max.y - 20.0 * z)],
                        Stroke::new(if i % 5 == 0 { 1.0 } else { 0.6 }, c),
                    );
                    x += 8.0 * z;
                    i += 1;
                }
            }
        }
    }

    fn paint_punches(&self, painter: &Painter, paper: Rect) {
        let z = (paper.height() / PAGE_H).max(0.2);
        for k in 0..3 {
            let y = paper.min.y + paper.height() * (0.22 + k as f32 * 0.28);
            let c = pos2(paper.min.x + 22.0 * z, y);
            painter.circle_filled(c, 7.0 * z, self.look.punch);
            painter.circle_stroke(c, 7.0 * z, Stroke::new(1.0, self.look.paper_rule_strong));
            painter.circle_filled(c, 3.2 * z, self.look.desk);
        }
    }

    fn paint_overlays(&mut self, ui: &mut Ui, painter: &Painter, rect: Rect) {
        let mut edited = false;
        if let Some((pg, id)) = self.editing_text {
            let cam = self.camera;
            if let Some(note) = &mut self.note {
                if let Some(page) = note.pages.get_mut(pg) {
                    if let Some(tx) = page.texts.iter_mut().find(|t| t.id == id) {
                        let origin = page_origin(pg);
                        let min = cam.to_screen(tx.min() + origin, rect);
                        let size = vec2(tx.size[0] * cam.zoom, tx.size[1] * cam.zoom);
                        let size_pt = tx.size_pt;
                        let col = tx.color32();
                        Area::new(Id::new(("tx", id)))
                            .fixed_pos(min)
                            .show(ui.ctx(), |ui| {
                                ui.set_min_size(size);
                                let te = TextEdit::multiline(&mut tx.text)
                                    .font(FontId::new(
                                        size_pt * cam.zoom,
                                        FontFamily::Name("serif".into()),
                                    ))
                                    .text_color(col)
                                    .desired_width(size.x)
                                    .frame(false);
                                if ui.add(te).changed() {
                                    edited = true;
                                }
                            });
                    }
                }
            }
        }
        if edited {
            self.dirty = true;
            self.last_change = Instant::now();
        }
        // légende d'atelier
        let msg = format!(
            "{}  ·  {:.1} mm  ·  {}",
            self.tool.label(),
            self.width * 0.26,
            self.look.name.to_lowercase()
        );
        painter.text(
            pos2(rect.min.x + 16.0, rect.max.y - 18.0),
            Align2::LEFT_BOTTOM,
            msg,
            self.look.mono(11.0),
            self.look.fg_dim,
        );
    }

    fn cursor_for_tool(&self, ui: &Ui, resp: &Response) {
        if !resp.hovered() {
            return;
        }
        let icon = match self.tool {
            Tool::Text => CursorIcon::Text,
            Tool::Lasso => CursorIcon::Crosshair,
            Tool::Image => CursorIcon::Copy,
            _ => CursorIcon::Crosshair,
        };
        ui.ctx().set_cursor_icon(icon);
    }

    fn texture_for(&mut self, ctx: &Context, note_id: &Uuid, file: &str) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(file) {
            return Some(t.clone());
        }
        let path = self.lib.media_path(*note_id, file);
        let img = image::open(path).ok()?.into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let eg = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let tex = ctx.load_texture(file.to_string(), eg, TextureOptions::LINEAR);
        self.textures.insert(file.to_string(), tex.clone());
        Some(tex)
    }

    fn export_png(&mut self, ctx: &Context) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let media = MediaLoader {
            root: self.lib.media_path(note.id, "").parent().unwrap_or(&self.lib.root).to_path_buf(),
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{}.png", slug(&note.title)))
            .save_file()
        else {
            return;
        };
        if let Some(pm) = export::raster_page(&note, 0, &self.look, 2.0, &media) {
            if let Some(png) = export::pixmap_png(&pm) {
                if std::fs::write(&path, png).is_ok() {
                    let t = ctx.input(|i| i.time);
                    self.toast("page arrachée", t);
                }
            }
        }
    }

    fn export_pdf(&mut self, ctx: &Context) {
        let Some(note) = self.note.clone() else {
            return;
        };
        let media = MediaLoader {
            root: self.lib.note_dir(note.id).join("media"),
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("{}.pdf", slug(&note.title)))
            .save_file()
        else {
            return;
        };
        if let Some(pdf) = export::pages_pdf(&note, &self.look, &media) {
            if std::fs::write(&path, pdf).is_ok() {
                let t = ctx.input(|i| i.time);
                self.toast("cahier exporté", t);
            }
        }
    }
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "cahier".into() } else { s }
}
