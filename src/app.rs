#![allow(float_literal_f32_fallback)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::*;
use uuid::Uuid;

use crate::camera::{page_at_y, page_origin, Camera, ZOOM_STOPS};
use crate::document::{ImageObj, Note, PaperKind, TextBox, PAGE_H, PAGE_W};
use crate::export::{self, MediaLoader};
use crate::ink::{
    default_width, draw_ants, erase_area, map_mesh, maybe_snap_shape, mixed_pressure, InkPoint,
    InkStroke, Nib, Tool,
};
use crate::library::{ensure_png, image_size, DockEdge, HandMode, Library};
use crate::look::Look;
use crate::pressure::Pressure;
use crate::seed;
use crate::tablet::{PenSnapshot, TabletBridge};
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
    last_ink: Tool,
    last_ink_width: f32,
    last_eraser: Tool,
    palm_grace_until: Option<Instant>,
    touch_grace_until: Option<Instant>,
    two_finger: Option<(f64, f32, f32)>,
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
    dock_edge: DockEdge,
    dock_float: Option<Pos2>,
    dock_grab: Vec2,
    dock_moved: bool,
    canvas_rect: Rect,
    tablet: TabletBridge,
    live_from_pen: bool,
}

impl CahierApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let look = Look::load();
        look.apply(&cc.egui_ctx);
        let mut lib = Library::open();
        seed::seed_if_needed(&mut lib);
        let width = default_width(Nib::Fineliner);
        let dock_edge = lib.index.dock;
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
            last_ink: Tool::Fineliner,
            last_ink_width: width,
            last_eraser: Tool::EraserStroke,
            palm_grace_until: None,
            touch_grace_until: None,
            two_finger: None,
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
            dock_edge,
            dock_float: None,
            dock_grab: Vec2::ZERO,
            dock_moved: false,
            canvas_rect: Rect::ZERO,
            tablet: TabletBridge::new(),
            live_from_pen: false,
        };
        if let Ok(q) = std::env::var("CAHIER_OPEN") {
            let q = q.trim().to_lowercase();
            if !q.is_empty() {
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

    fn ph(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.page_h.max(1.0))
            .unwrap_or(PAGE_H)
    }

    fn page_in_view(&self, rect: Rect) -> usize {
        let n = self.note.as_ref().map(|n| n.pages.len()).unwrap_or(1);
        page_at_y(self.camera.to_paper(rect.center(), rect).y, n, self.ph())
    }

    fn fit_zoom_now(&self) -> f32 {
        let rect = self.canvas_rect;
        if rect.width() < 10.0 {
            return 1.0;
        }
        let (pw, ph) = self
            .note
            .as_ref()
            .map(|n| n.page_size())
            .unwrap_or((PAGE_W, PAGE_H));
        Camera::fit_zoom(rect, pw, ph)
    }

    /// 100 = feuille collée à l’écran.
    fn zoom_percent(&self) -> i32 {
        let fit = self.fit_zoom_now();
        ((self.camera.zoom / fit.max(0.001)) * 100.0).round() as i32
    }

    fn zoom_level(&self) -> f32 {
        self.camera.zoom / self.fit_zoom_now().max(0.001)
    }

    fn set_zoom_level(&mut self, level: f32) {
        let rect = self.canvas_rect;
        if rect.width() < 10.0 {
            return;
        }
        let target = (self.fit_zoom_now() * level).max(0.001);
        self.camera.set_zoom_at(rect.center(), rect, target);
    }

    fn zoom_in(&mut self) {
        let cur = self.zoom_level();
        let next = ZOOM_STOPS
            .iter()
            .copied()
            .find(|&s| s > cur + 0.03)
            .unwrap_or(*ZOOM_STOPS.last().unwrap());
        self.set_zoom_level(next);
    }

    fn zoom_out(&mut self) {
        let cur = self.zoom_level();
        let next = ZOOM_STOPS
            .iter()
            .copied()
            .rev()
            .find(|&s| s < cur - 0.03)
            .unwrap_or(*ZOOM_STOPS.first().unwrap());
        self.set_zoom_level(next);
    }

    fn fit_to_screen(&mut self) {
        self.need_fit = true;
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

    fn palm_guard(&self, pen: &PenSnapshot) -> bool {
        pen.in_proximity
            || self
                .palm_grace_until
                .map(|t| Instant::now() < t)
                .unwrap_or(false)
    }

    fn finger_alive(&self, ui: &Ui) -> bool {
        ui.input(|i| i.any_touches())
            || self
                .touch_grace_until
                .map(|t| Instant::now() < t)
                .unwrap_or(false)
    }

    fn note_touches(&mut self, ui: &Ui) {
        let hit = ui.input(|i| {
            i.any_touches()
                || i.events.iter().any(|e| matches!(e, Event::Touch { .. }))
        });
        if hit {
            self.touch_grace_until = Some(Instant::now() + Duration::from_millis(120));
        }
    }

    fn remember_ink(&mut self) {
        if self.tool.is_ink() {
            self.last_ink = self.tool;
            self.last_ink_width = self.width;
        }
    }

    fn toggle_eraser(&mut self) {
        if self.tool.is_eraser() {
            self.tool = self.last_ink;
            self.width = self.last_ink_width;
        } else {
            self.remember_ink();
            self.tool = self.last_eraser;
        }
    }

    fn cycle_eraser_kind(&mut self) {
        let now = if self.tool.is_eraser() {
            self.tool
        } else {
            self.last_eraser
        };
        self.last_eraser = if now == Tool::EraserStroke {
            Tool::EraserArea
        } else {
            Tool::EraserStroke
        };
        self.remember_ink();
        self.tool = self.last_eraser;
    }

    fn cycle_hand(&mut self) {
        self.lib.index.hand = self.lib.index.hand.cycle();
        self.lib.save_index();
    }
}

impl eframe::App for CahierApp {
    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        self.tablet.pump(frame);
        let pen = self.tablet.snapshot();
        if let Some(p) = pen.pressure {
            self.pressure.push_touch(p);
        } else if !pen.in_proximity {
            self.pressure.clear();
        }
        if pen.in_proximity {
            self.palm_grace_until = Some(Instant::now() + Duration::from_millis(180));
        }
        if matches!(self.scene, Scene::Desk) && pen.air_toggle {
            self.toggle_eraser();
        }
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
        if matches!(self.scene, Scene::Desk) || self.tablet.wants_repaint() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
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
            .anchor(Align2::CENTER_TOP, vec2(0.0, 56.0))
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(self.look.desk_deep)
                    .corner_radius(20)
                    .stroke(Stroke::new(1.0_f32, self.look.accent))
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
        let mut zoom_in = false;
        let mut zoom_out = false;
        let mut close = false;
        let mut delete_sel = false;
        let mut dup = false;
        let mut add_page = false;
        let mut width_delta: f32 = 0.0;
        let mut tool: Option<Tool> = None;
        let mut color: Option<usize> = None;
        let mut cycle_paper = false;
        let mut erase_toggle = false;
        let mut toggle_fiche = false;

        let typing = ctx.wants_keyboard_input() || self.editing_text.is_some();
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
            if (c && i.key_pressed(Key::Num0))
                || (!typing && !c && i.key_pressed(Key::Num0))
            {
                fit = true;
            }
            if !typing
                && (i.key_pressed(Key::Plus)
                    || i.key_pressed(Key::Equals)
                    || (c && i.key_pressed(Key::Equals)))
            {
                zoom_in = true;
            }
            if !typing && i.key_pressed(Key::Minus) {
                zoom_out = true;
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
                    if sh {
                        tool = Some(Tool::EraserArea);
                    } else {
                        erase_toggle = true;
                    }
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
                    if !typing && (i.key_pressed(Key::Slash) || i.key_pressed(Key::F1)) {
                        toggle_fiche = true;
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
                self.fit_to_screen();
            }
            if zoom_in {
                self.zoom_in();
            }
            if zoom_out {
                self.zoom_out();
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
                if self.tool.is_ink() {
                    self.last_ink_width = self.width;
                }
            }
            if erase_toggle {
                self.toggle_eraser();
            } else if let Some(t) = tool {
                if t.is_ink() {
                    self.last_ink = t;
                    self.last_ink_width = default_width(t.nib().unwrap());
                }
                if t.is_eraser() {
                    self.remember_ink();
                    self.last_eraser = t;
                }
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
        if toggle_fiche {
            self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
            self.lib.save_index();
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
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    let Scene::Shelf { query } = &mut self.scene else {
                        return;
                    };
                    Frame::NONE
                        .fill(self.look.desk_deep)
                        .corner_radius(22)
                        .inner_margin(Margin::symmetric(16, 9))
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
                ui.add_space(18.0);
                self.fiche_pupitre(ui);
                ui.add_space(22.0);

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
        Area::new(Id::new("fab-nouveau"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-28.0, -28.0))
            .show(ctx, |ui| {
                if self.round_action(ui, 56.0, true, paint_plus) {
                    self.new_note();
                }
            });
    }

    fn fiche_pupitre(&mut self, ui: &mut Ui) {
        let folded = self.lib.index.fiche_pliee;
        let max_w = ui.available_width() - 72.0;
        let w = max_w.clamp(280.0, 640.0);
        let h = if folded { 46.0 } else { 172.0 };
        ui.horizontal(|ui| {
            ui.add_space(36.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            let p = ui.painter_at(rect);
            let ink = self.look.ink;
            let mute = ink.gamma_multiply(0.52);
            p.rect_filled(
                rect.translate(vec2(3.0, 4.0)),
                CornerRadius::same(4),
                self.look.shadow,
            );
            p.rect_filled(rect, CornerRadius::same(4), self.look.paper);
            p.rect_stroke(
                rect,
                CornerRadius::same(4),
                Stroke::new(1.0_f32, ink.gamma_multiply(0.18)),
                StrokeKind::Inside,
            );
            let holes = if folded { 1 } else { 3 };
            for i in 0..holes {
                let t = if holes == 1 {
                    0.5
                } else {
                    i as f32 / (holes - 1) as f32
                };
                let y = rect.min.y + 14.0 + t * (h - 28.0);
                let c = pos2(rect.min.x + 13.0, y);
                p.circle_filled(c, 3.4, self.look.punch);
                p.circle_stroke(c, 3.4, Stroke::new(1.0_f32, ink.gamma_multiply(0.28)));
            }
            p.line_segment(
                [
                    pos2(rect.min.x + 24.0, rect.min.y + 8.0),
                    pos2(rect.min.x + 24.0, rect.max.y - 8.0),
                ],
                Stroke::new(1.0_f32, self.look.paper_rule_strong),
            );
            if folded {
                p.text(
                    pos2(rect.min.x + 36.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "sur le pupitre  ·  stylet écrit · doigt pousse · tap 2 doigts = annuler",
                    self.look.serif(15.0),
                    ink,
                );
            } else {
                p.text(
                    pos2(rect.min.x + 36.0, rect.min.y + 10.0),
                    Align2::LEFT_TOP,
                    "sur le pupitre",
                    self.look.serif(18.0),
                    ink,
                );
                let left = [
                    "stylet     écrit",
                    "doigt      pousse la feuille",
                    "2 doigts   panorama  ·  tap = annuler",
                    "pincement  zoom",
                ];
                let right = [
                    "bouton 1      gomme (tenir)",
                    "clic en l'air plume ↔ gomme",
                    "bouton 2      lasso (tenir)",
                    "gomme trousse coller / recoller",
                ];
                let y0 = rect.min.y + 38.0;
                for (i, line) in left.iter().enumerate() {
                    p.text(
                        pos2(rect.min.x + 36.0, y0 + i as f32 * 18.0),
                        Align2::LEFT_TOP,
                        *line,
                        self.look.mono(12.0),
                        mute,
                    );
                }
                let col2 = (rect.min.x + 36.0 + (w - 48.0) * 0.50).min(rect.max.x - 220.0);
                if col2 > rect.min.x + 200.0 {
                    for (i, line) in right.iter().enumerate() {
                        p.text(
                            pos2(col2, y0 + i as f32 * 18.0),
                            Align2::LEFT_TOP,
                            *line,
                            self.look.mono(12.0),
                            mute,
                        );
                    }
                }
                p.text(
                    pos2(rect.min.x + 36.0, rect.max.y - 14.0),
                    Align2::LEFT_CENTER,
                    "appui long gomme : trait ↔ zone   ·   puits main / chiffon   ·   e plume ↔ gomme",
                    self.look.mono(11.0),
                    mute.gamma_multiply(0.85),
                );
            }
            if resp.hovered() {
                p.rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.4_f32, self.look.accent),
                    StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                self.lib.index.fiche_pliee = !folded;
                self.lib.save_index();
            }
            resp.on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(if folded { "déplier" } else { "replier" });
        });
    }

    fn cahier_dos(&mut self, ui: &mut Ui, meta: &crate::library::NoteMeta) -> Response {
        let size = vec2(168.0, 236.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let cloth = self.look.cloth_at(meta.cover);
        let painter = ui.painter_at(rect);
        let r = CornerRadius::same(22);
        painter.rect_filled(rect.translate(vec2(4.0, 5.0)), r, self.look.shadow);
        painter.rect_filled(rect, r, cloth);
        let spine = Rect::from_min_max(rect.min, pos2(rect.min.x + 16.0, rect.max.y));
        painter.rect_filled(
            spine,
            CornerRadius {
                nw: 22,
                ne: 0,
                sw: 22,
                se: 0,
            },
            self.look.desk_deep.gamma_multiply(0.55),
        );
        for y in [rect.min.y + 28.0, rect.center().y, rect.max.y - 28.0] {
            painter.circle_filled(pos2(rect.min.x + 8.0, y), 3.2, self.look.punch);
            painter.circle_stroke(pos2(rect.min.x + 8.0, y), 3.2, Stroke::new(1.0_f32, self.look.desk_deep));
        }
        let inner = Rect::from_min_max(
            pos2(rect.min.x + 22.0, rect.min.y + 18.0),
            pos2(rect.max.x - 12.0, rect.max.y - 18.0),
        );
        painter.rect_filled(inner, CornerRadius::same(14), self.look.paper);
        painter.line_segment(
            [pos2(inner.min.x + 8.0, inner.min.y + 36.0), pos2(inner.max.x - 8.0, inner.min.y + 38.0)],
            Stroke::new(1.4_f32, self.look.ink.gamma_multiply(0.55)),
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
            painter.rect_stroke(rect, r, Stroke::new(1.5_f32, self.look.accent), StrokeKind::Outside);
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
        ui.painter().rect_filled(rect, CornerRadius::same(17), fill);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            self.look.mono(13.0),
            if accent { self.look.desk_deep } else { self.look.fg },
        );
        resp.clicked()
    }

    fn round_action(
        &self,
        ui: &mut Ui,
        size: f32,
        accent: bool,
        paint: impl FnOnce(&Painter, Pos2, Color32),
    ) -> bool {
        self.round_well(ui, size, accent, paint).clicked()
    }

    fn round_well(
        &self,
        ui: &mut Ui,
        size: f32,
        accent: bool,
        paint: impl FnOnce(&Painter, Pos2, Color32),
    ) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let fill = if accent {
            self.look.accent
        } else if resp.hovered() {
            self.look.desk_edge
        } else {
            Color32::TRANSPARENT
        };
        let fg = if accent {
            self.look.desk_deep
        } else {
            self.look.fg
        };
        let p = ui.painter();
        if fill.a() > 0 {
            p.circle_filled(rect.center(), size * 0.46, fill);
        }
        paint(p, rect.center(), fg);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn ui_desk(&mut self, ctx: &Context) {
        self.handle_drops_and_paste(ctx);

        TopBottomPanel::top("rule")
            .exact_height(44.0)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    if self
                        .round_well(ui, 40.0, false, paint_back)
                        .on_hover_text("étagère")
                        .clicked()
                    {
                        self.close_desk();
                        return;
                    }
                    ui.add_space(6.0);
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
                        ui.add_space(8.0);
                        let more = self
                            .round_well(ui, 36.0, false, paint_more)
                            .on_hover_text("plus");
                        let mut act = None;
                        Popup::menu(&more).show(|ui| {
                            ui.set_min_width(128.0);
                            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                            if self.objet_btn(ui, "png", false) {
                                act = Some(0);
                            }
                            if self.objet_btn(ui, "pdf", false) {
                                act = Some(1);
                            }
                        });
                        match act {
                            Some(0) => self.export_png(ctx),
                            Some(1) => self.export_pdf(ctx),
                            _ => {}
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_fit)
                            .on_hover_text("taille écran")
                            .clicked()
                        {
                            self.fit_to_screen();
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_zoom_in)
                            .on_hover_text("zoomer  +")
                            .clicked()
                        {
                            self.zoom_in();
                        }
                        {
                            let pct = format!("{}%", self.zoom_percent());
                            let near_fit = (self.zoom_percent() - 100).abs() <= 2;
                            let color = if near_fit {
                                self.look.fg
                            } else {
                                self.look.muted
                            };
                            let (r, lab) = ui.allocate_exact_size(vec2(46.0, 28.0), Sense::click());
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                pct,
                                self.look.mono(12.0),
                                color,
                            );
                            if lab
                                .on_hover_text("taille écran")
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.fit_to_screen();
                            }
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_zoom_out)
                            .on_hover_text("dézoomer  −")
                            .clicked()
                        {
                            self.zoom_out();
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_paper_icon)
                            .on_hover_text(
                                self.note
                                    .as_ref()
                                    .map(|n| n.paper.label())
                                    .unwrap_or("papier"),
                            )
                            .clicked()
                        {
                            if let Some(n) = &mut self.note {
                                n.paper = n.paper.cycle();
                                self.mark_dirty();
                            }
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_plus)
                            .on_hover_text("page")
                            .clicked()
                        {
                            self.push_snapshot();
                            if let Some(n) = &mut self.note {
                                n.add_page();
                                self.mark_dirty();
                            }
                        }
                    });
                });
            });

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                self.ui_canvas(ui);
            });

        self.ui_floating_dock(ctx);
    }

    fn ui_floating_dock(&mut self, ctx: &Context) {
        let screen = ctx.screen_rect();
        let vertical = self.dock_edge.vertical();
        let mut area = Area::new(Id::new("cahier-dock"))
            .order(Order::Foreground)
            .interactable(true);
        if let Some(pos) = self.dock_float {
            let pos = pos.clamp(screen.min, pos2(screen.max.x - 48.0, screen.max.y - 48.0));
            area = area.current_pos(pos);
        } else {
            let (align, off) = match self.dock_edge {
                DockEdge::Bottom => (Align2::CENTER_BOTTOM, vec2(0.0, -12.0)),
                DockEdge::Top => (Align2::CENTER_TOP, vec2(0.0, 50.0)),
                DockEdge::Left => (Align2::LEFT_CENTER, vec2(10.0, 0.0)),
                DockEdge::Right => (Align2::RIGHT_CENTER, vec2(-10.0, 0.0)),
            };
            area = area.anchor(align, off);
        }
        let inner = area.show(ctx, |ui| {
            Frame::NONE
                .fill(self.look.desk_deep)
                .stroke(Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.38)))
                .corner_radius(28)
                .inner_margin(if vertical {
                    Margin::symmetric(6, 10)
                } else {
                    Margin::symmetric(10, 6)
                })
                .show(ui, |ui| {
                    if vertical {
                        ui.set_max_height((screen.height() - 80.0).max(120.0));
                        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                        ui.vertical(|ui| {
                            self.dock_inner(ui, true);
                        });
                    } else {
                        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                        ui.horizontal(|ui| {
                            self.dock_inner(ui, false);
                        });
                    }
                });
        });
        ctx.data_mut(|d| d.insert_temp(Id::new("dock-rect"), inner.response.rect));
        if self.dock_float.is_some() {
            ctx.request_repaint();
        }
    }

    fn dock_inner(&mut self, ui: &mut Ui, vertical: bool) {
        let grip = self.dock_grip(ui, vertical);
        if grip.drag_started() {
            let bar = ui
                .ctx()
                .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
                .unwrap_or(grip.rect);
            let pointer = grip.interact_pointer_pos().unwrap_or(bar.min);
            self.dock_grab = pointer - bar.min;
            self.dock_float = Some(bar.min);
            self.dock_moved = false;
        }
        if grip.dragged() {
            if grip.drag_delta().length() > 0.3 {
                self.dock_moved = true;
            }
            if let Some(p) = grip.interact_pointer_pos() {
                self.dock_float = Some(p - self.dock_grab);
            }
        }
        if grip.drag_stopped() {
            if self.dock_moved {
                let p = grip
                    .interact_pointer_pos()
                    .or(self.dock_float)
                    .unwrap_or_else(|| ui.ctx().screen_rect().center());
                self.dock_edge = snap_dock(p, ui.ctx().screen_rect());
                self.lib.index.dock = self.dock_edge;
                self.lib.save_index();
            }
            self.dock_float = None;
            self.dock_moved = false;
        }

        if self
            .round_well(ui, 36.0, false, paint_undo)
            .on_hover_text("annuler")
            .clicked()
        {
            let mut ok = false;
            if let Some(n) = &mut self.note {
                ok = self.undo.undo(n);
            }
            if ok {
                self.mark_dirty();
            }
        }
        if self
            .round_well(ui, 36.0, false, paint_redo)
            .on_hover_text("rétablir")
            .clicked()
        {
            let mut ok = false;
            if let Some(n) = &mut self.note {
                ok = self.undo.redo(n);
            }
            if ok {
                self.mark_dirty();
            }
        }
        self.dock_gap(ui, vertical);
        for t in [
            Tool::Fineliner,
            Tool::Brush,
            Tool::Pencil,
            Tool::Highlighter,
        ] {
            if self.tool_glyph(ui, t) {
                if self.tool == t {
                    self.width = next_width(self.width);
                    self.last_ink_width = self.width;
                } else {
                    self.tool = t;
                    self.last_ink = t;
                    if let Some(nib) = t.nib() {
                        self.width = default_width(nib);
                        self.last_ink_width = self.width;
                    }
                }
            }
        }
        let pen = self.tablet.snapshot();
        let erase_shown = if self.tool.is_eraser() {
            self.tool
        } else {
            self.last_eraser
        };
        let erase_resp = self
            .paint_tool_well(
                ui,
                erase_shown,
                self.tool.is_eraser() || pen.eraser,
            )
            .on_hover_text(if self.tool.is_eraser() {
                "re-tap : reprise de l'encre · appui long : trait / zone"
            } else {
                "gomme · appui long : trait / zone"
            });
        if erase_resp.secondary_clicked() {
            self.cycle_eraser_kind();
        } else if erase_resp.clicked() {
            self.toggle_eraser();
        }
        let lasso_resp = self
            .paint_tool_well(ui, Tool::Lasso, self.tool == Tool::Lasso || pen.lasso_btn)
            .on_hover_text("lasso");
        if lasso_resp.clicked() {
            self.tool = Tool::Lasso;
        }
        self.dock_gap(ui, vertical);
        let hand = self.lib.index.hand;
        if self
            .round_well(ui, 36.0, hand != HandMode::Stylus, |p, c, fg| {
                paint_hand(p, c, hand, fg)
            })
            .on_hover_text(format!("{} — {}", hand.label(), hand.hint()))
            .clicked()
        {
            self.cycle_hand();
            let t = ui.input(|i| i.time);
            self.toast(self.lib.index.hand.hint(), t);
        }
        self.dock_gap(ui, vertical);
        let high = self.tool == Tool::Highlighter;
        if high {
            let n = self.look.highs.len();
            for (i, c) in self.look.highs.iter().take(5).enumerate() {
                let swatch = Color32::from_rgb(c.r(), c.g(), c.b());
                if self.color_dot(ui, swatch, i == self.color_i % n, vertical) {
                    self.color_i = i;
                }
            }
        } else {
            const DOTS: [usize; 5] = [0, 2, 4, 5, 7];
            for &i in &DOTS {
                let Some(c) = self.look.inks.get(i).copied() else {
                    continue;
                };
                if self.color_dot(ui, c, self.color_i == i, vertical) {
                    self.color_i = i;
                }
            }
        }
        if self.tool.is_ink() || self.tool.is_eraser() {
            ui.add_space(6.0);
            for &(w, r) in &[(2.2_f32, 3.0), (5.0, 4.4), (11.0, 6.0)] {
                let on = (self.width - w).abs() < 1.6;
                if self.thick_dot(ui, r, on, vertical) {
                    self.width = w;
                    if self.tool.is_ink() {
                        self.last_ink_width = w;
                    }
                }
            }
        }
        self.dock_gap(ui, vertical);
        if self.tool_glyph(ui, Tool::Text) {
            self.tool = Tool::Text;
        }
        if self.tool_glyph(ui, Tool::Image) {
            self.tool = Tool::Image;
        }
    }

    fn dock_grip(&self, ui: &mut Ui, vertical: bool) -> Response {
        let size = if vertical {
            vec2(38.0, 18.0)
        } else {
            vec2(18.0, 38.0)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let p = ui.painter();
        let c = rect.center();
        let fg = self.look.fg.gamma_multiply(0.55);
        if vertical {
            for i in 0..3 {
                let x = c.x - 6.0 + i as f32 * 6.0;
                p.circle_filled(pos2(x, c.y - 2.4), 1.55, fg);
                p.circle_filled(pos2(x, c.y + 2.4), 1.55, fg);
            }
        } else {
            for i in 0..3 {
                let y = c.y - 6.0 + i as f32 * 6.0;
                p.circle_filled(pos2(c.x - 2.4, y), 1.55, fg);
                p.circle_filled(pos2(c.x + 2.4, y), 1.55, fg);
            }
        }
        let cursor = if resp.dragged() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        };
        resp.on_hover_cursor(cursor)
            .on_hover_text("glisser la trousse")
    }

    fn dock_gap(&self, ui: &mut Ui, vertical: bool) {
        if vertical {
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(vec2(22.0, 2.0), Sense::hover());
            ui.painter().line_segment(
                [
                    pos2(rect.min.x + 1.0, rect.center().y),
                    pos2(rect.max.x - 1.0, rect.center().y),
                ],
                Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.55)),
            );
            ui.add_space(4.0);
        } else {
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(vec2(2.0, 18.0), Sense::hover());
            ui.painter().line_segment(
                [
                    pos2(rect.center().x, rect.min.y + 1.0),
                    pos2(rect.center().x, rect.max.y - 1.0),
                ],
                Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.55)),
            );
            ui.add_space(4.0);
        }
    }

    fn color_dot(&self, ui: &mut Ui, color: Color32, on: bool, vertical: bool) -> bool {
        let size = if vertical {
            vec2(38.0, 26.0)
        } else {
            vec2(28.0, 38.0)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let r = if on { 8.4 } else { 7.2 };
        p.circle_filled(c, r + 1.4, self.look.fg.gamma_multiply(0.55));
        p.circle_filled(c, r, color);
        p.circle_stroke(
            c,
            r,
            Stroke::new(1.05_f32, self.look.fg.gamma_multiply(0.55)),
        );
        if on {
            p.circle_stroke(c, r + 3.8, Stroke::new(1.7_f32, self.look.fg));
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("encre")
            .clicked()
    }

    fn thick_dot(&self, ui: &mut Ui, r: f32, on: bool, vertical: bool) -> bool {
        let size = if vertical {
            vec2(38.0, 22.0)
        } else {
            vec2(20.0, 38.0)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let fill = if on {
            self.ink_color()
        } else {
            self.look.fg.gamma_multiply(0.42)
        };
        p.circle_filled(c, r, fill);
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("épaisseur")
            .clicked()
    }

    fn tool_glyph(&self, ui: &mut Ui, tool: Tool) -> bool {
        self.paint_tool_well(
            ui,
            tool,
            self.tool == tool || (tool.is_eraser() && self.tool.is_eraser()),
        )
        .on_hover_text(tool.label())
        .clicked()
    }

    fn paint_tool_well(&self, ui: &mut Ui, tool: Tool, active: bool) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
        let bg = if active {
            self.look.accent
        } else if resp.hovered() {
            self.look.desk_edge
        } else {
            Color32::TRANSPARENT
        };
        let fg = if active {
            self.look.desk_deep
        } else {
            self.look.fg
        };
        let p = ui.painter();
        let c = rect.center();
        if bg.a() > 0 {
            p.circle_filled(c, 16.5, bg);
        }
        paint_tool(p, tool, c, fg);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn ui_canvas(&mut self, ui: &mut Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas_rect = rect;
        if rect.width() > 10.0 {
            let grew = self
                .note
                .as_mut()
                .map(|n| {
                    n.grow_to_view(vec2(
                        (rect.width() - 16.0).max(120.0),
                        (rect.height() - 16.0).max(160.0),
                    ))
                })
                .unwrap_or(false);
            if grew {
                self.mark_dirty();
            }
            if self.need_fit {
                let (pw, ph) = self
                    .note
                    .as_ref()
                    .map(|n| n.page_size())
                    .unwrap_or((PAGE_W, PAGE_H));
                let page = self.page_in_view(rect);
                self.camera.fit_page(rect, page, pw, ph);
                self.need_fit = false;
            }
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
        let pen = self.tablet.snapshot();
        let (egui_zoom, pinch_center, ctrl, point_scroll, line_scroll, mt_pan) = ui.input(|i| {
            let mt = i.multi_touch();
            let mut point = Vec2::ZERO;
            let mut line = Vec2::ZERO;
            for e in &i.events {
                if let Event::MouseWheel { unit, delta, .. } = e {
                    match unit {
                        MouseWheelUnit::Point => point += *delta,
                        MouseWheelUnit::Line | MouseWheelUnit::Page => line += *delta,
                    }
                }
            }
            (
                i.zoom_delta(),
                mt.map(|m| m.center_pos),
                i.modifiers.command,
                point,
                line,
                mt.map(|m| m.translation_delta).unwrap_or(Vec2::ZERO),
            )
        });
        let focus = pinch_center
            .or(resp.hover_pos())
            .unwrap_or(rect.center());

        if pen.pinching || (pen.pinch_zoom - 1.0).abs() > 0.0005 {
            self.camera.zoom_at(focus, rect, pen.pinch_zoom);
            self.camera.pan += pen.pinch_pan;
        } else if (egui_zoom - 1.0).abs() > 0.0005 {
            self.camera.zoom_at(focus, rect, egui_zoom);
        } else if !ctrl && point_scroll != Vec2::ZERO {
            // Pavé tactile : deux doigts. Vertical = zoom, horizontal = panorama.
            if point_scroll.y.abs() >= point_scroll.x.abs() * 0.35 {
                let f = (1.0 + point_scroll.y * 0.004).clamp(0.55, 1.85);
                self.camera.zoom_at(focus, rect, f);
            } else {
                self.camera.pan += point_scroll;
            }
        } else if mt_pan != Vec2::ZERO {
            self.camera.pan += mt_pan;
        } else if !ctrl && resp.hovered() && line_scroll != Vec2::ZERO {
            self.camera.pan += line_scroll;
        }

        let pan = space || middle;
        if pan && resp.dragged() {
            self.camera.pan += resp.drag_delta();
        }

        let time = ui.input(|i| i.time);
        self.note_touches(ui);
        let mt = ui.input(|i| i.multi_touch());
        if let Some(m) = mt {
            if m.num_touches >= 2 {
                let g = self.two_finger.get_or_insert((time, 0.0, 0.0));
                g.1 += m.translation_delta.length();
                g.2 += (m.zoom_delta - 1.0).abs();
            }
        } else if let Some((t0, travel, zdev)) = self.two_finger.take() {
            if time - t0 < 0.22 && travel < 14.0 && zdev < 0.05 {
                if let Some(n) = &mut self.note {
                    if self.undo.undo(n) {
                        self.mark_dirty();
                    }
                }
            }
        }

        let pen_busy = pen.down || pen.pressed || (self.live.is_some() && self.live_from_pen);
        if self.lib.index.hand == HandMode::Stylus
            && self.finger_alive(ui)
            && mt.is_none()
            && !pen.pinching
            && !pen_busy
            && !self.palm_guard(&pen)
            && !pan
            && resp.dragged()
        {
            self.camera.pan += resp.drag_delta();
        }
    }

    fn handle_tool(&mut self, ui: &Ui, resp: &Response, rect: Rect) {
        let space = ui.input(|i| i.key_down(Key::Space));
        if space
            || ui.input(|i| i.pointer.middle_down())
            || ui.input(|i| i.multi_touch().is_some())
            || self.tablet.snapshot().pinching
        {
            return;
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let pen = self.tablet.snapshot();
        let pen_ink =
            pen.down || pen.pressed || pen.released || (self.live.is_some() && self.live_from_pen);
        let any_touch = self.finger_alive(ui);
        if !pen_ink && self.live.is_none() {
            if self.palm_guard(&pen) {
                return;
            }
            if self.lib.index.hand == HandMode::Stylus && any_touch {
                return;
            }
        }

        let time = ui.input(|i| i.time);
        let extra: Vec<Pos2>;
        let (screen, primary_down, primary_pressed, primary_released, secondary) =
            if pen_ink {
                if let Some(screen) = pen.pos {
                    extra = pen.samples;
                    (
                        screen,
                        pen.down,
                        pen.pressed,
                        pen.released,
                        pen.eraser,
                    )
                } else {
                    if self.live.is_some() && self.live_from_pen && (pen.released || !pen.down) {
                        self.finish_live(shift, time);
                    }
                    return;
                }
            } else {
                let pos = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
                let Some(screen) = pos else {
                    return;
                };
                extra = ui.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            Event::PointerMoved(sp) => Some(*sp),
                            Event::Touch {
                                phase: TouchPhase::Move | TouchPhase::Start,
                                pos,
                                ..
                            } => Some(*pos),
                            _ => None,
                        })
                        .collect()
                });
                (
                    screen,
                    ui.input(|i| i.pointer.primary_down()),
                    ui.input(|i| i.pointer.primary_pressed()),
                    ui.input(|i| i.pointer.primary_released()),
                    ui.input(|i| i.pointer.secondary_down()),
                )
            };

        if !rect.contains(screen) && !(self.live.is_some() && (primary_released || !primary_down)) {
            return;
        }
        if ui.ctx().data(|d| {
            d.get_temp::<Rect>(Id::new("dock-rect"))
                .map(|r| r.expand(4.0).contains(screen))
                .unwrap_or(false)
        }) {
            if self.live.is_some() && (primary_released || !primary_down) {
                self.finish_live(shift, time);
            }
            return;
        }
        let paper = self.camera.to_paper(screen, rect);
        let n_pages = self.note.as_ref().map(|n| n.pages.len()).unwrap_or(1);
        let page = page_at_y(paper.y, n_pages, self.ph());
        let local = paper - page_origin(page, self.ph());

        let mut tool = self.tool;
        if pen.lasso_btn {
            tool = Tool::Lasso;
        } else if pen.eraser || secondary {
            tool = Tool::EraserStroke;
        }
        if !pen_ink && self.lib.index.hand == HandMode::Chiffon {
            tool = Tool::EraserArea;
        }

        if tool.is_ink() && (primary_down || primary_pressed) && !resp.dragged_by(PointerButton::Middle) {
            if self.live.is_none() {
                self.push_snapshot();
                let nib = tool.nib().unwrap();
                let mut s = InkStroke::new(nib, self.ink_color(), self.width);
                s.push(InkPoint::new(local, 0.7));
                self.live = Some((page, s));
                self.live_from_pen = pen_ink;
            }
            let speed = self.speed(local);
            let hw = self.pressure.latest();
            if let Some((p, s)) = &mut self.live {
                if *p == page {
                    let pr = mixed_pressure(s.nib, hw, speed);
                    s.push(InkPoint::new(local, pr));
                }
            }
            let cam = self.camera;
            let ph = self.ph();
            let pressure = self.pressure.latest();
            if let Some((pgi, live)) = &mut self.live {
                for sp in &extra {
                    let pp = cam.to_paper(*sp, rect);
                    if page_at_y(pp.y, n_pages, ph) == *pgi {
                        let loc = pp - page_origin(*pgi, ph);
                        live.push(InkPoint::new(loc, mixed_pressure(live.nib, pressure, 200.0)));
                    }
                }
            }
        }
        if self.live.is_some() && (primary_released || !primary_down) && !(tool.is_ink() && primary_down) {
            self.finish_live(shift, time);
        }

        if tool.is_eraser() && (primary_down || secondary) {
            if primary_pressed || ui.input(|i| i.pointer.secondary_pressed()) || (pen_ink && pen.pressed && secondary)
            {
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
            if primary_pressed {
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
            } else if primary_down {
                if let Some(last) = self.drag_last {
                    let d = paper - last;
                    self.translate_sel(d);
                    self.drag_last = Some(paper);
                    self.mark_dirty();
                } else if self.lasso.last().map(|p| p.distance(paper) > 2.0).unwrap_or(true) {
                    self.lasso.push(paper);
                }
            } else if primary_released {
                self.drag_last = None;
                if self.lasso.len() > 2 {
                    self.sel = self.hits_lasso();
                    self.lasso.clear();
                }
            }
        } else if !primary_down {
            self.drag_last = None;
        }

        let tap = resp.clicked() || (pen_ink && primary_pressed);
        if tool == Tool::Text && tap {
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

        if tool == Tool::Image && tap {
            self.pick_image(page, local);
        }
    }

    fn finish_live(&mut self, shift: bool, time: f64) {
        self.live_from_pen = false;
        if let Some((p, mut s)) = self.live.take() {
            if let Some(snapped) = maybe_snap_shape(&s, shift) {
                s = snapped;
                self.toast("forme", time);
            }
            if !s.points.is_empty() {
                if let Some(n) = &mut self.note {
                    if let Some(page) = n.pages.get_mut(p) {
                        page.strokes.push(s);
                    }
                }
                self.mark_dirty();
            }
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
            let o = page_origin(s.page, self.ph());
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
            let o = page_origin(pi, self.ph());
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
            let origin = page_origin(pi, note.page_h);
            let min = map(Pos2::new(origin.x, origin.y));
            let max = map(Pos2::new(origin.x + note.page_w, origin.y + note.page_h));
            let paper = Rect::from_min_max(min, max);
            painter.rect_filled(paper.translate(vec2(8.0, 10.0)), CornerRadius::same(22), self.look.shadow);
            let fill = if note.paper == PaperKind::Slate {
                self.look.desk_deep
            } else {
                self.look.paper
            };
            painter.rect_filled(paper, CornerRadius::same(22), fill);
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

        let ph = self
            .note
            .as_ref()
            .map(|n| n.page_h.max(1.0))
            .unwrap_or(PAGE_H);
        if let Some((pi, live)) = &mut self.live {
            let origin = page_origin(*pi, ph);
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
                    let o = page_origin(s.page, self.ph());
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
                        let origin = page_origin(pg, note.page_h.max(1.0));
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
        let _ = (ui, painter, rect);
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

fn snap_dock(pos: Pos2, screen: Rect) -> DockEdge {
    let dl = (pos.x - screen.left()).abs();
    let dr = (pos.x - screen.right()).abs();
    let dt = (pos.y - screen.top()).abs();
    let db = (pos.y - screen.bottom()).abs();
    let m = dl.min(dr).min(dt).min(db);
    if m == dt {
        DockEdge::Top
    } else if m == db {
        DockEdge::Bottom
    } else if m == dl {
        DockEdge::Left
    } else {
        DockEdge::Right
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

fn next_width(w: f32) -> f32 {
    const WIDTHS: [f32; 3] = [2.2, 5.0, 11.0];
    let i = WIDTHS
        .iter()
        .position(|&p| (w - p).abs() < 1.6)
        .unwrap_or(0);
    WIDTHS[(i + 1) % WIDTHS.len()]
}

fn polar(c: Pos2, ang: f32, r: f32) -> Pos2 {
    pos2(c.x + ang.cos() * r, c.y + ang.sin() * r)
}

fn shaft(p: &egui::Painter, a: Pos2, b: Pos2, thick: f32, col: Color32) {
    p.line_segment([a, b], Stroke::new(thick, col));
    p.circle_filled(a, thick * 0.48, col);
    p.circle_filled(b, thick * 0.48, col);
}

fn paint_plus(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_filled(Rect::from_center_size(c, vec2(16.0, 3.0)), 2.0, fg);
    p.rect_filled(Rect::from_center_size(c, vec2(3.0, 16.0)), 2.0, fg);
}

fn paint_back(p: &egui::Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(2.15_f32, fg);
    p.line_segment([pos2(c.x + 3.5, c.y - 7.2), pos2(c.x - 6.5, c.y)], st);
    p.line_segment([pos2(c.x - 6.5, c.y), pos2(c.x + 3.5, c.y + 7.2)], st);
    p.circle_filled(pos2(c.x - 6.5, c.y), 1.1, fg);
}

fn paint_more(p: &egui::Painter, c: Pos2, fg: Color32) {
    for dy in [-6.2, 0.0, 6.2] {
        p.circle_filled(pos2(c.x, c.y + dy), 1.7, fg);
    }
}

fn paint_fit(p: &egui::Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.8_f32, fg);
    let s = 6.4;
    let m = 2.2;
    p.line_segment([pos2(c.x - s, c.y - m), pos2(c.x - s, c.y - s)], st);
    p.line_segment([pos2(c.x - s, c.y - s), pos2(c.x - m, c.y - s)], st);
    p.line_segment([pos2(c.x + s, c.y - m), pos2(c.x + s, c.y - s)], st);
    p.line_segment([pos2(c.x + s, c.y - s), pos2(c.x + m, c.y - s)], st);
    p.line_segment([pos2(c.x - s, c.y + m), pos2(c.x - s, c.y + s)], st);
    p.line_segment([pos2(c.x - s, c.y + s), pos2(c.x - m, c.y + s)], st);
    p.line_segment([pos2(c.x + s, c.y + m), pos2(c.x + s, c.y + s)], st);
    p.line_segment([pos2(c.x + s, c.y + s), pos2(c.x + m, c.y + s)], st);
}

fn paint_zoom_in(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.circle_stroke(c, 8.4, Stroke::new(1.7_f32, fg));
    p.rect_filled(Rect::from_center_size(c, vec2(9.0, 2.15)), 1.2, fg);
    p.rect_filled(Rect::from_center_size(c, vec2(2.15, 9.0)), 1.2, fg);
}

fn paint_zoom_out(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.circle_stroke(c, 8.4, Stroke::new(1.7_f32, fg));
    p.rect_filled(Rect::from_center_size(c, vec2(9.0, 2.15)), 1.2, fg);
}

fn paint_paper_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_stroke(
        Rect::from_center_size(c, vec2(12.5, 15.5)),
        3.6,
        Stroke::new(1.6_f32, fg),
        StrokeKind::Inside,
    );
    for dy in [-3.2, 0.0, 3.2] {
        p.line_segment(
            [pos2(c.x - 3.6, c.y + dy), pos2(c.x + 3.6, c.y + dy)],
            Stroke::new(1.35_f32, fg),
        );
    }
}

fn paint_curved_arrow(p: &egui::Painter, c: Pos2, fg: Color32, flip: f32) {
    let mut pts = Vec::with_capacity(15);
    let start = -2.55_f32;
    let end = 0.42_f32;
    let n = 14;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let a = start + (end - start) * t;
        pts.push(pos2(
            c.x + flip * a.cos() * 8.1,
            c.y + a.sin() * 8.1 + 1.1,
        ));
    }
    p.add(egui::Shape::line(pts.clone(), Stroke::new(2.1_f32, fg)));
    if pts.len() >= 2 {
        let tip = pts[0];
        let nxt = pts[1];
        let dir = (tip - nxt).normalized();
        let nrm = vec2(-dir.y, dir.x);
        p.add(egui::Shape::convex_polygon(
            vec![
                tip + dir * 1.1,
                tip - dir * 5.4 + nrm * 4.0,
                tip - dir * 5.4 - nrm * 4.0,
            ],
            fg,
            Stroke::NONE,
        ));
    }
}

fn paint_undo(p: &egui::Painter, c: Pos2, fg: Color32) {
    paint_curved_arrow(p, c, fg, 1.0);
}

fn paint_redo(p: &egui::Painter, c: Pos2, fg: Color32) {
    paint_curved_arrow(p, c, fg, -1.0);
}

fn paint_hand(p: &egui::Painter, c: Pos2, mode: HandMode, fg: Color32) {
    match mode {
        HandMode::Stylus => {
            let tip = pos2(c.x + 1.4, c.y - 9.2);
            let tail = pos2(c.x - 1.4, c.y + 8.6);
            shaft(p, tail, tip, 2.7, fg);
            p.circle_filled(tail, 2.0, fg);
            let n = (tip - tail).normalized();
            let side = vec2(-n.y, n.x);
            p.add(egui::Shape::convex_polygon(
                vec![
                    tip + n * 3.2,
                    tip - n * 0.4 + side * 2.0,
                    tip - n * 0.4 - side * 2.0,
                ],
                fg,
                Stroke::NONE,
            ));
        }
        HandMode::Main => {
            p.circle_filled(pos2(c.x - 5.4, c.y + 2.6), 3.15, fg);
            p.circle_filled(pos2(c.x, c.y - 4.8), 3.35, fg);
            p.circle_filled(pos2(c.x + 5.4, c.y + 2.6), 3.15, fg);
        }
        HandMode::Chiffon => {
            p.circle_stroke(c, 7.8, Stroke::new(1.7_f32, fg));
            p.line_segment(
                [pos2(c.x - 5.2, c.y + 1.2), pos2(c.x, c.y - 2.4)],
                Stroke::new(1.7_f32, fg),
            );
            p.line_segment(
                [pos2(c.x, c.y - 2.4), pos2(c.x + 5.4, c.y + 1.6)],
                Stroke::new(1.7_f32, fg),
            );
            p.circle_filled(pos2(c.x + 5.4, c.y + 1.6), 1.4, fg);
        }
    }
}

fn paint_tool(p: &egui::Painter, tool: Tool, c: Pos2, fg: Color32) {
    let ang = -0.92_f32;
    let pi = std::f32::consts::PI;
    match tool {
        Tool::Fineliner => {
            let tip = polar(c, ang, 9.6);
            let tail = polar(c, ang + pi, 8.0);
            shaft(p, tail, tip, 3.2, fg);
            p.circle_filled(tail, 2.3, fg);
            let n = (tip - tail).normalized();
            let side = vec2(-n.y, n.x);
            p.add(egui::Shape::convex_polygon(
                vec![
                    tip + n * 3.4,
                    tip - n * 0.2 + side * 2.2,
                    tip - n * 0.2 - side * 2.2,
                ],
                fg,
                Stroke::NONE,
            ));
        }
        Tool::Brush => {
            let tip = polar(c, ang, 8.8);
            let tail = polar(c, ang + pi, 8.2);
            shaft(p, tail, c, 2.5, fg);
            p.circle_filled(tail, 1.8, fg);
            p.circle_filled(c.lerp(tip, 0.15), 3.8, fg);
            p.circle_filled(c.lerp(tip, 0.55), 5.4, fg);
            p.circle_filled(tip, 3.6, fg);
        }
        Tool::Pencil => {
            let tip = polar(c, ang, 10.2);
            let tail = polar(c, ang + pi, 8.2);
            shaft(p, tail.lerp(tip, 0.18), tail.lerp(tip, 0.68), 4.0, fg);
            p.circle_filled(tail, 2.7, fg);
            let n = (tip - tail).normalized();
            let side = vec2(-n.y, n.x);
            let base = tail.lerp(tip, 0.66);
            p.add(egui::Shape::convex_polygon(
                vec![tip, base + side * 2.4, base - side * 2.4],
                fg,
                Stroke::NONE,
            ));
        }
        Tool::Highlighter => {
            let dir = vec2(ang.cos(), ang.sin());
            let nrm = vec2(-dir.y, dir.x);
            let tail = c - dir * 7.4;
            let mid = c + dir * 1.2;
            let tip = c + dir * 8.6;
            shaft(p, tail, mid, 6.6, fg);
            p.circle_filled(tail, 3.1, fg);
            let a = tip + nrm * 4.4;
            let b = tip - nrm * 4.4;
            p.line_segment([a, b], Stroke::new(3.4_f32, fg));
            p.circle_filled(a, 1.7, fg);
            p.circle_filled(b, 1.7, fg);
            shaft(p, mid, tip, 5.2, fg);
        }
        Tool::EraserStroke => {
            let a = polar(c, ang, 6.8);
            let b = polar(c, ang + pi, 6.8);
            shaft(p, a, b, 8.6, fg);
        }
        Tool::EraserArea => {
            p.circle_stroke(c, 8.0, Stroke::new(1.8_f32, fg));
            p.circle_filled(c, 2.1, fg);
        }
        Tool::Lasso => {
            let n = 18;
            let mut pts = Vec::with_capacity(n);
            for i in 0..n {
                let t = i as f32 / n as f32 * std::f32::consts::TAU;
                pts.push(pos2(c.x + t.cos() * 7.6, c.y + t.sin() * 6.0 - 0.6));
            }
            for i in 0..n {
                if i % 2 == 0 {
                    p.line_segment(
                        [pts[i], pts[(i + 1) % n]],
                        Stroke::new(1.7_f32, fg),
                    );
                }
            }
            let h0 = pos2(c.x + 6.2, c.y + 3.4);
            let h1 = pos2(c.x + 10.0, c.y + 7.2);
            p.line_segment([h0, h1], Stroke::new(1.7_f32, fg));
            p.circle_filled(h1, 1.6, fg);
        }
        Tool::Text => {
            p.rect_filled(
                Rect::from_center_size(pos2(c.x, c.y - 6.0), vec2(14.0, 2.5)),
                1.4,
                fg,
            );
            p.rect_filled(
                Rect::from_center_size(pos2(c.x, c.y + 1.4), vec2(2.5, 14.0)),
                1.4,
                fg,
            );
            p.circle_filled(pos2(c.x - 7.0, c.y - 6.0), 1.35, fg);
            p.circle_filled(pos2(c.x + 7.0, c.y - 6.0), 1.35, fg);
            p.rect_filled(
                Rect::from_center_size(pos2(c.x, c.y + 8.0), vec2(6.4, 2.1)),
                1.1,
                fg,
            );
        }
        Tool::Image => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(16.5, 13.0)),
                5.0,
                Stroke::new(1.6_f32, fg),
                StrokeKind::Inside,
            );
            p.circle_filled(pos2(c.x - 3.8, c.y - 2.5), 1.85, fg);
            p.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x - 7.2, c.y + 4.6),
                    pos2(c.x - 1.6, c.y - 1.0),
                    pos2(c.x + 2.1, c.y + 2.3),
                    pos2(c.x + 4.0, c.y + 0.5),
                    pos2(c.x + 7.4, c.y + 4.6),
                ],
                fg,
                Stroke::NONE,
            ));
        }
    }
}
