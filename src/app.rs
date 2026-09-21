#![allow(float_literal_f32_fallback)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::*;
use uuid::Uuid;

use crate::camera::{page_at_y, page_origin, Camera, ZOOM_STOPS};
use crate::document::{ImageObj, Note, PaperKind, TextBox, PAGE_H, PAGE_SPAN, PAGE_W};
use crate::export::{self, MediaLoader};
use crate::ink::{
    default_width, draw_ants, erase_area, map_mesh, maybe_snap_shape, mixed_pressure, InkPoint,
    InkStroke, Nib, Tool,
};
use crate::library::{ensure_png, image_size, DockEdge, Library, NoteMode};
use crate::look::Look;
use crate::pressure::Pressure;
use crate::seed;
use crate::tablet::{PenSnapshot, TabletBridge};
use crate::undo::UndoStack;

const DOS_W: f32 = 128.0;
const DOS_H: f32 = 172.0;
const DOS_PAD: f32 = 14.0;
const PAPER_PEEK: f32 = 28.0;
const TITLE_BAND: f32 = 44.0;
const ICON_ROW: f32 = 30.0;

#[derive(Clone)]
enum Scene {
    Shelf { query: String },
    Desk,
}

#[derive(Clone, Copy)]
enum DosAct {
    Open,
    Dup,
    Pin,
    Del,
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
    /// Page à cadrer en vue d’écriture (le dézoom montre la feuille entière).
    land_page: Option<usize>,
    title_buf: String,
    dock_edge: DockEdge,
    dock_float: Option<Pos2>,
    dock_grab: Vec2,
    dock_moved: bool,
    canvas_rect: Rect,
    tablet: TabletBridge,
    live_from_pen: bool,
    /// Évite de rebasculer en tablette tant que le stylet reste en proximité.
    prox_flipped: bool,
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
            land_page: None,
            title_buf: String::new(),
            dock_edge,
            dock_float: None,
            dock_grab: Vec2::ZERO,
            dock_moved: false,
            canvas_rect: Rect::ZERO,
            tablet: TabletBridge::new(),
            live_from_pen: false,
            prox_flipped: false,
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
        self.land_page = None;
        self.need_fit = true;
    }

    fn add_page(&mut self) {
        self.push_snapshot();
        let next = self.note.as_ref().map(|n| n.pages.len()).unwrap_or(0);
        if let Some(n) = &mut self.note {
            n.add_page();
            self.mark_dirty();
            self.land_page = Some(next);
            self.need_fit = false;
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
        if let Some(mut n) = self.lib.load_note(id) {
            let sheet = (n.page_w - PAGE_W).abs() > 0.5 || (n.page_h - PAGE_H).abs() > 0.5;
            n.page_w = PAGE_W;
            n.page_h = PAGE_H;
            self.title_buf = n.title.clone();
            self.note = Some(n);
            self.scene = Scene::Desk;
            self.undo.clear();
            self.sel.clear();
            self.live = None;
            self.lasso.clear();
            self.editing_text = None;
            self.need_fit = false;
            self.land_page = Some(0);
            self.textures.clear();
            if sheet {
                self.mark_dirty();
            }
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
        let n = Note::blank("Untitled", cover);
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

    /// Choisit une gomme ; re-tap sur la même = retour à l’encre.
    fn pick_eraser(&mut self, kind: Tool) {
        if !kind.is_eraser() {
            return;
        }
        if self.tool == kind {
            self.tool = self.last_ink;
            self.width = self.last_ink_width;
        } else {
            self.remember_ink();
            self.last_eraser = kind;
            self.tool = kind;
        }
    }

    fn mode(&self) -> NoteMode {
        self.lib.index.mode
    }

    fn is_tablette(&self) -> bool {
        self.mode().is_tablette()
    }

    fn slot(&self) -> f32 {
        if self.is_tablette() {
            44.0
        } else {
            34.0
        }
    }

    fn set_mode(&mut self, mode: NoteMode) {
        if self.lib.index.mode == mode {
            return;
        }
        self.lib.index.mode = mode;
        self.lib.save_index();
    }

    fn toggle_mode(&mut self) {
        self.set_mode(self.mode().other());
    }

    fn tool_hover(&self, tool: Tool) -> String {
        let lab = tool.label();
        if self.is_tablette() {
            return lab.to_string();
        }
        let key = match tool {
            Tool::Fineliner => Some("p"),
            Tool::Brush => Some("b"),
            Tool::Pencil => Some("c"),
            Tool::Highlighter => Some("h"),
            Tool::EraserStroke => Some("e"),
            Tool::EraserArea => Some("shift+e"),
            Tool::Lasso => Some("l"),
            Tool::Text => Some("t"),
            Tool::Image => Some("i"),
        };
        match key {
            Some(k) => format!("{lab}  ·  {k}"),
            None => lab.to_string(),
        }
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
            if matches!(self.scene, Scene::Desk)
                && !self.prox_flipped
                && !self.is_tablette()
            {
                self.set_mode(NoteMode::Tablette);
                self.prox_flipped = true;
                let t = ctx.input(|i| i.time);
                self.toast(NoteMode::Tablette.hint(), t);
            }
        } else {
            self.prox_flipped = false;
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
                self.toast(format!("Theme · {}", label), t);
            }
        }
        let wants_pen = self.tablet.wants_repaint();
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
        let focused = ctx.input(|i| i.focused);
        let busy = self.live.is_some()
            || !self.lasso.is_empty()
            || self.dirty
            || !self.sel.is_empty()
            || self.dock_float.is_some()
            || wants_pen
            || self.toast.is_some();
        if busy && focused {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
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
                    .fill(self.look.paper)
                    .corner_radius(3)
                    .stroke(Stroke::new(1.0_f32, self.look.ink.gamma_multiply(0.18)))
                    .inner_margin(Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(msg)
                                .font(self.look.serif(15.0))
                                .color(self.look.ink),
                        );
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
        let mut erase_pick: Option<Tool> = None;
        let mut toggle_fiche = false;
        let mut toggle_mode = false;

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
                    erase_pick = Some(if sh {
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
                if i.key_pressed(Key::K) {
                    toggle_mode = true;
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
                    self.toast("Saved", t);
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
                self.add_page();
            }
            if width_delta != 0.0 {
                self.width = (self.width + width_delta).clamp(0.8, 48.0);
                if self.tool.is_ink() {
                    self.last_ink_width = self.width;
                }
            }
            if let Some(kind) = erase_pick {
                self.pick_eraser(kind);
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
        if toggle_mode {
            if let Scene::Desk = self.scene {
                self.toggle_mode();
                let t = ctx.input(|i| i.time);
                self.toast(self.mode().hint(), t);
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
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    let Scene::Shelf { query } = &mut self.scene else {
                        return;
                    };
                    let search_w = (ui.available_width() - 88.0).clamp(200.0, 720.0);
                    Frame::NONE
                        .fill(self.look.desk_deep)
                        .corner_radius(22)
                        .inner_margin(Margin::symmetric(18, 11))
                        .show(ui, |ui| {
                            ui.set_width(search_w);
                            let te = TextEdit::singleline(query)
                                .hint_text("Search")
                                .font(self.look.mono(15.0))
                                .frame(false);
                            ui.add(te);
                        });
                });
                ui.add_space(28.0);

                let query = match &self.scene {
                    Scene::Shelf { query } => query.to_lowercase(),
                    _ => String::new(),
                };
                let notes: Vec<_> = self
                    .lib
                    .index
                    .notes
                    .iter()
                    .filter(|m| query.is_empty() || m.title.to_lowercase().contains(&query))
                    .cloned()
                    .collect();
                if notes.is_empty() {
                    ui.add_space(48.0);
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        ui.label(
                            RichText::new("No notes yet")
                                .font(self.look.serif(22.0))
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
                    let card_w = DOS_W + DOS_PAD * 2.0;
                    let gap = 6.0;
                    let cols = ((available + gap) / (card_w + gap)).floor().max(1.0) as usize;
                    let mut i = 0;
                    while i < notes.len() {
                        ui.horizontal(|ui| {
                            ui.add_space(28.0);
                            for _ in 0..cols {
                                if i >= notes.len() {
                                    break;
                                }
                                let meta = notes[i].clone();
                                i += 1;
                                match self.cahier_dos(ui, &meta) {
                                    Some(DosAct::Open) => open = Some(meta.id),
                                    Some(DosAct::Dup) => dup = Some(meta.id),
                                    Some(DosAct::Pin) => pin = Some(meta.id),
                                    Some(DosAct::Del) => del = Some(meta.id),
                                    None => {}
                                }
                                ui.add_space(gap);
                            }
                        });
                        ui.add_space(8.0);
                    }
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
                if self
                    .inkwell(ui)
                    .on_hover_text("New  ·  N")
                    .clicked()
                {
                    self.new_note();
                }
            });

        let fiche_ouverte = !self.lib.index.fiche_pliee;
        Area::new(Id::new("shelf-tuto-btn"))
            .anchor(Align2::RIGHT_TOP, vec2(-28.0, -22.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                if self
                    .round_well(ui, 44.0, false, paint_help)
                    .on_hover_text(if fiche_ouverte { "Close" } else { "Help  ·  /" })
                    .clicked()
                {
                    self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
                    self.lib.save_index();
                }
            });
        if !self.lib.index.fiche_pliee {
            Area::new(Id::new("shelf-tuto-fiche"))
                .anchor(Align2::RIGHT_TOP, vec2(-28.0, -78.0))
                .order(Order::Foreground)
                .show(ctx, |ui| {
                    self.fiche_pupitre(ui);
                });
        }
    }

    fn fiche_pupitre(&mut self, ui: &mut Ui) {
        let w = 480.0_f32.min(ui.ctx().screen_rect().width() - 72.0).max(280.0);
        let h = 176.0;
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
        for i in 0..3 {
            let t = i as f32 / 2.0;
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
        p.text(
            pos2(rect.min.x + 36.0, rect.min.y + 10.0),
            Align2::LEFT_TOP,
            "Shortcuts",
            self.look.serif(17.0),
            ink,
        );
        let y0 = rect.min.y + 38.0;
        let col2 = (rect.min.x + 36.0 + (w - 52.0) * 0.48).min(rect.max.x - 220.0);
        let left = [
            "souris      écrit",
            "espace      panorama",
            "e / shift+e sélection / zone",
            "p b c h     plumes",
            "ctrl+z      annuler",
        ];
        for (i, line) in left.iter().enumerate() {
            p.text(
                pos2(rect.min.x + 36.0, y0 + i as f32 * 16.5),
                Align2::LEFT_TOP,
                *line,
                self.look.mono(12.0),
                mute,
            );
        }
        if col2 > rect.min.x + 180.0 {
            let right = [
                "stylet      écrit",
                "doigt       pousse",
                "2 doigts    tap = annuler",
                "bouton 1    dernière gomme",
                "bouton 2    lasso",
            ];
            for (i, line) in right.iter().enumerate() {
                p.text(
                    pos2(col2, y0 + i as f32 * 16.5),
                    Align2::LEFT_TOP,
                    *line,
                    self.look.mono(12.0),
                    mute,
                );
            }
        }
        if resp.hovered() {
            p.rect_stroke(
                rect,
                CornerRadius::same(4),
                Stroke::new(1.2_f32, ink.gamma_multiply(0.45)),
                StrokeKind::Outside,
            );
        }
        if resp.clicked() {
            self.lib.index.fiche_pliee = true;
            self.lib.save_index();
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Close");
    }

    fn cahier_dos(&mut self, ui: &mut Ui, meta: &crate::library::NoteMeta) -> Option<DosAct> {
        let slot = vec2(
            DOS_W + DOS_PAD * 2.0,
            TITLE_BAND + DOS_H + DOS_PAD * 2.0 + PAPER_PEEK + ICON_ROW,
        );
        let (slot_rect, resp) = ui.allocate_exact_size(slot, Sense::click());
        let id = Id::new("cahier-dos").with(meta.id);
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = pointer.is_some_and(|p| slot_rect.contains(p));
        if over {
            ui.ctx().request_repaint();
        }
        let lift_t = ui.ctx().animate_bool_with_time(id.with("peek"), over, 0.36);
        let e = lift_t * lift_t * (3.0 - 2.0 * lift_t);
        let cloth = self.look.cloth_at(meta.cover);
        let cloth_deep = shade_rgb(cloth, 0.70);
        let cloth_edge = shade_rgb(cloth, 0.48);
        let paper = self.look.paper;

        let face = Rect::from_min_size(
            pos2(
                slot_rect.center().x - DOS_W * 0.5,
                slot_rect.min.y + DOS_PAD + TITLE_BAND,
            ),
            vec2(DOS_W, DOS_H),
        );
        let painter = if e > 0.02 {
            ui.ctx()
                .layer_painter(LayerId::new(Order::Foreground, id))
                .with_clip_rect(slot_rect.expand(4.0).intersect(ui.clip_rect().expand(4.0)))
        } else {
            ui.painter_at(slot_rect)
        };

        painter.rect_filled(
            face.translate(vec2(3.0, 5.0 + 1.5 * e)),
            CornerRadius::same(8),
            self.look.shadow.gamma_multiply(0.40 + 0.18 * e),
        );

        // Feuille sous le titre (ne dépasse pas la ligne du titre)
        let title_baseline = face.min.y - 24.0;
        let peek_max = (face.min.y - title_baseline - 6.0).max(8.0);
        let peek = (e * PAPER_PEEK).min(peek_max);
        if peek > 1.0 {
            let sheet = Rect::from_min_max(
                pos2(face.min.x + 18.0, face.min.y - peek),
                pos2(face.max.x - 8.0, face.min.y + 10.0),
            );
            painter.rect_filled(
                sheet,
                CornerRadius {
                    nw: 2,
                    ne: 2,
                    sw: 0,
                    se: 0,
                },
                paper,
            );
            if e > 0.25 {
                let mut y = sheet.min.y + 8.0;
                while y < face.min.y - 2.0 {
                    painter.line_segment(
                        [pos2(sheet.min.x + 6.0, y), pos2(sheet.max.x - 6.0, y)],
                        Stroke::new(1.0_f32, self.look.paper_rule),
                    );
                    y += 9.0;
                }
            }
        }

        painter.rect_filled(face, CornerRadius::same(8), cloth);
        let spine = Rect::from_min_max(face.min, pos2(face.min.x + 13.0, face.max.y));
        painter.rect_filled(
            spine,
            CornerRadius {
                nw: 8,
                ne: 0,
                sw: 8,
                se: 0,
            },
            cloth_deep,
        );
        painter.line_segment(
            [
                pos2(spine.max.x, face.min.y + 10.0),
                pos2(spine.max.x, face.max.y - 10.0),
            ],
            Stroke::new(1.1_f32, cloth_edge),
        );
        painter.rect_stroke(
            face,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, cloth_edge),
            StrokeKind::Inside,
        );
        for i in 0..3 {
            let o = i as f32 * 1.5;
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(face.max.x - 1.0 + o, face.min.y + 10.0),
                    pos2(face.max.x + 2.2 + o, face.max.y - 10.0),
                ),
                CornerRadius::same(1),
                shade_rgb(paper, 0.94 - i as f32 * 0.04),
            );
        }
        if meta.pinned {
            let x = face.max.x - 16.0;
            let ribbon = [
                pos2(x - 3.4, face.min.y),
                pos2(x + 3.4, face.min.y),
                pos2(x + 3.4, face.min.y + 26.0),
                pos2(x, face.min.y + 31.0),
                pos2(x - 3.4, face.min.y + 26.0),
            ];
            painter.add(Shape::convex_polygon(
                ribbon.to_vec(),
                self.look.accent,
                Stroke::NONE,
            ));
        }

        // Titre en dernier — au-dessus de la feuille, plus lisible
        let title: String = {
            let t = meta.title.as_str();
            if t.chars().count() > 18 {
                format!("{}…", t.chars().take(16).collect::<String>())
            } else {
                t.to_string()
            }
        };
        painter.text(
            pos2(face.center().x, title_baseline),
            Align2::CENTER_BOTTOM,
            title,
            self.look.serif(19.0),
            self.look.fg,
        );

        let mut act = None;
        let show_icons = e > 0.08;
        if show_icons {
            let trash_col = self
                .look
                .inks
                .get(2)
                .copied()
                .unwrap_or(Color32::from_rgb(0xe2, 0x4b, 0x4a));
            let a = (40.0 + 215.0 * e).clamp(0.0, 255.0) as u8;
            let ink = Color32::from_rgba_unmultiplied(
                self.look.fg.r(),
                self.look.fg.g(),
                self.look.fg.b(),
                a,
            );
            let trash = Color32::from_rgba_unmultiplied(trash_col.r(), trash_col.g(), trash_col.b(), a);
            let pin_col = if meta.pinned {
                Color32::from_rgba_unmultiplied(
                    self.look.accent.r(),
                    self.look.accent.g(),
                    self.look.accent.b(),
                    a,
                )
            } else {
                ink
            };
            let icon = 24.0;
            let gap = 10.0;
            let strip = 3.0 * icon + 2.0 * gap;
            let origin = pos2(face.center().x - strip * 0.5, face.max.y + 6.0);
            let dup_r = Rect::from_min_size(origin, vec2(icon, icon));
            let pin_r = Rect::from_min_size(origin + vec2(icon + gap, 0.0), vec2(icon, icon));
            let del_r = Rect::from_min_size(origin + vec2(2.0 * (icon + gap), 0.0), vec2(icon, icon));

            if self
                .dos_icon(ui, &painter, id.with("dup"), dup_r, ink, paint_copy_pages)
                .on_hover_text("Duplicate")
                .clicked()
            {
                act = Some(DosAct::Dup);
            }
            if self
                .dos_icon(ui, &painter, id.with("pin"), pin_r, pin_col, paint_pin)
                .on_hover_text(if meta.pinned { "Unpin" } else { "Pin" })
                .clicked()
            {
                act = Some(DosAct::Pin);
            }
            if self
                .dos_icon(ui, &painter, id.with("del"), del_r, trash, paint_bin)
                .on_hover_text("Delete")
                .clicked()
            {
                act = Some(DosAct::Del);
            }
        }

        if act.is_none() && resp.clicked() {
            act = Some(DosAct::Open);
        }
        resp.on_hover_cursor(CursorIcon::PointingHand);
        act
    }

    fn dos_icon(
        &self,
        ui: &mut Ui,
        painter: &Painter,
        id: Id,
        rect: Rect,
        fg: Color32,
        paint: impl FnOnce(&Painter, Pos2, Color32),
    ) -> Response {
        let resp = ui.interact(rect, id, Sense::click());
        let c = rect.center();
        let col = if resp.hovered() {
            Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), fg.a().max(220))
        } else {
            fg
        };
        paint(painter, c, col);
        resp.on_hover_cursor(CursorIcon::PointingHand)
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

    fn inkwell(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let well = if resp.hovered() {
            self.look.desk_edge
        } else {
            self.look.desk_deep
        };
        p.circle_filled(c, 25.0, well);
        p.circle_stroke(
            c,
            25.0,
            Stroke::new(1.2_f32, self.look.muted.gamma_multiply(0.7)),
        );
        p.circle_filled(c, 16.5, shade_rgb(well, 0.72));
        p.circle_stroke(
            c,
            16.5,
            Stroke::new(1.0_f32, self.look.ink.gamma_multiply(0.35)),
        );
        paint_plus(p, c, self.look.paper);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn ui_desk(&mut self, ctx: &Context) {
        self.handle_drops_and_paste(ctx);

        TopBottomPanel::top("rule")
            .exact_height(44.0)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                ui.painter().hline(
                    bar.x_range(),
                    bar.max.y - 0.5,
                    Stroke::new(1.0_f32, self.look.desk_deep),
                );
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    if self
                        .round_well(ui, 40.0, false, paint_back)
                        .on_hover_text("Library")
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
                            .on_hover_text("More");
                        let mut act = None;
                        Popup::menu(&more).show(|ui| {
                            ui.set_min_width(128.0);
                            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                            if self.objet_btn(ui, "PNG", false) {
                                act = Some(0);
                            }
                            if self.objet_btn(ui, "PDF", false) {
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
                            .on_hover_text("Fit")
                            .clicked()
                        {
                            self.fit_to_screen();
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_zoom_in)
                            .on_hover_text("Zoom in")
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
                            ui.painter().rect_filled(r, CornerRadius::same(8), self.look.desk_deep);
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                pct,
                                self.look.mono(12.0),
                                color,
                            );
                            if lab
                                .on_hover_text("Fit")
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.fit_to_screen();
                            }
                        }
                        if self
                            .round_well(ui, 36.0, false, paint_zoom_out)
                            .on_hover_text("Zoom out")
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
                                    .unwrap_or("Paper"),
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
                            .on_hover_text("Page")
                            .clicked()
                        {
                            self.add_page();
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

        let slot = self.slot();
        if self
            .round_well(ui, slot, false, paint_undo)
            .on_hover_text(if self.is_tablette() {
                "Undo  ·  2-finger tap"
            } else {
                "Undo  ·  Ctrl+Z"
            })
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
            .round_well(ui, slot, false, paint_redo)
            .on_hover_text(if self.is_tablette() {
                "Redo"
            } else {
                "Redo  ·  Ctrl+Y"
            })
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
        for kind in [Tool::EraserStroke, Tool::EraserArea] {
            let held = pen.eraser && self.last_eraser == kind;
            let on = self.tool == kind || held;
            let tip = match kind {
                Tool::EraserStroke => {
                    if self.is_tablette() {
                        "sélection · efface le trait entier"
                    } else {
                        "sélection · efface le trait entier  ·  e"
                    }
                }
                Tool::EraserArea => {
                    if self.is_tablette() {
                        "zone · efface sous le doigt / stylet"
                    } else {
                        "zone · efface sous le curseur  ·  shift+e"
                    }
                }
                _ => kind.label(),
            };
            if self
                .paint_tool_well(ui, kind, on)
                .on_hover_text(tip)
                .clicked()
            {
                self.pick_eraser(kind);
            }
        }
        let lasso_resp = self
            .paint_tool_well(ui, Tool::Lasso, self.tool == Tool::Lasso || pen.lasso_btn)
            .on_hover_text(self.tool_hover(Tool::Lasso));
        if lasso_resp.clicked() {
            self.tool = Tool::Lasso;
        }
        self.dock_gap(ui, vertical);
        let mode = self.mode();
        if self
            .round_well(ui, slot, mode.is_tablette(), |p, c, fg| {
                paint_mode(p, c, mode, fg)
            })
            .on_hover_text(format!("{}  ·  k", mode.label()))
            .clicked()
        {
            self.toggle_mode();
            let t = ui.input(|i| i.time);
            self.toast(self.mode().hint(), t);
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
            .on_hover_text("Move toolbar")
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
            .on_hover_text("Ink")
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
            .on_hover_text("Width")
            .clicked()
    }

    fn tool_glyph(&self, ui: &mut Ui, tool: Tool) -> bool {
        self.paint_tool_well(
            ui,
            tool,
            self.tool == tool,
        )
        .on_hover_text(self.tool_hover(tool))
        .clicked()
    }

    fn paint_tool_well(&self, ui: &mut Ui, tool: Tool, active: bool) -> Response {
        let s = self.slot();
        let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click());
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
            p.circle_filled(c, s * 0.43, bg);
        }
        let cut = if active {
            self.look.accent
        } else if resp.hovered() {
            self.look.desk_edge
        } else {
            self.look.desk_deep
        };
        paint_tool(p, tool, c, fg, cut);
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn ui_canvas(&mut self, ui: &mut Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas_rect = rect;
        if rect.width() > 10.0 {
            let (pw, ph) = self
                .note
                .as_ref()
                .map(|n| n.page_size())
                .unwrap_or((PAGE_W, PAGE_H));
            if let Some(page) = self.land_page.take() {
                let n = self.note.as_ref().map(|n| n.pages.len()).unwrap_or(1).max(1);
                self.camera
                    .show_slice(rect, page.min(n - 1), pw, ph, PAGE_SPAN);
            } else if self.need_fit {
                let page = self.page_in_view(rect);
                self.camera.fit_page(rect, page, pw, ph);
                self.need_fit = false;
            }
        }

        self.handle_camera(ui, &resp, rect);
        self.handle_tool(ui, &resp, rect);
        self.paint_world(&painter, rect);
        self.paint_overlays(ui, rect);
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
            if self.is_tablette() && time - t0 < 0.22 && travel < 14.0 && zdev < 0.05 {
                if let Some(n) = &mut self.note {
                    if self.undo.undo(n) {
                        self.mark_dirty();
                    }
                }
            }
        }

        let pen_busy = pen.down || pen.pressed || (self.live.is_some() && self.live_from_pen);
        let finger_pan = if self.is_tablette() {
            self.finger_alive(ui)
        } else {
            ui.input(|i| i.any_touches())
        };
        if finger_pan
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
        let screen_touch = ui.input(|i| i.any_touches());
        if !pen_ink && self.live.is_none() {
            if self.is_tablette() {
                if self.palm_guard(&pen) || self.finger_alive(ui) {
                    return;
                }
            } else if screen_touch {
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
            tool = self.last_eraser;
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
                self.toast("Shape", time);
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

    fn paint_world(&mut self, painter: &Painter, rect: Rect) {
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
            let sheet_r = CornerRadius::same(5);
            painter.rect_filled(
                paper.translate(vec2(3.0, 4.0)),
                sheet_r,
                self.look.shadow,
            );
            let fill = if note.paper == PaperKind::Slate {
                self.look.desk_deep
            } else {
                self.look.paper
            };
            painter.rect_filled(paper, sheet_r, fill);
            self.paint_template(painter, paper, note.paper, cam.zoom);
            if note.paper == PaperKind::Lined {
                self.paint_punches(painter, paper);
            }
            if note.paper != PaperKind::Slate {
                let fold_c = self.look.paper_rule_strong;
                let fold = [
                    pos2(paper.max.x, paper.min.y),
                    pos2(paper.max.x - 16.0 * cam.zoom, paper.min.y),
                    pos2(paper.max.x, paper.min.y + 16.0 * cam.zoom),
                ];
                painter.add(Shape::convex_polygon(fold.to_vec(), fold_c, Stroke::NONE));
            }

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
            }

            painter.text(
                pos2(paper.center().x, paper.max.y - 14.0 * cam.zoom.min(1.2)),
                Align2::CENTER_BOTTOM,
                format!("{}", pi + 1),
                self.look.serif(12.0),
                self.look.ink.gamma_multiply(0.38),
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
                        draw_ants(painter, map(a + o), map(b + o), self.look.accent);
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
                    Stroke::new(1.15, {
                        let red = self
                            .look
                            .inks
                            .get(2)
                            .copied()
                            .unwrap_or(self.look.ink);
                        Color32::from_rgb(
                            (self.look.paper.r() as f32 * 0.62 + red.r() as f32 * 0.38) as u8,
                            (self.look.paper.g() as f32 * 0.62 + red.g() as f32 * 0.38) as u8,
                            (self.look.paper.b() as f32 * 0.62 + red.b() as f32 * 0.38) as u8,
                        )
                    }),
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

    fn paint_overlays(&mut self, ui: &mut Ui, rect: Rect) {
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
    }

    fn cursor_for_tool(&self, ui: &Ui, resp: &Response) {
        if !resp.hovered() {
            return;
        }
        let space = ui.input(|i| i.key_down(Key::Space) || i.pointer.middle_down());
        if space {
            ui.ctx().set_cursor_icon(if ui.input(|i| i.pointer.any_down()) {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
            return;
        }
        let pen = self.tablet.snapshot();
        if self.is_tablette() && pen.in_proximity {
            ui.ctx().set_cursor_icon(CursorIcon::None);
            return;
        }
        if self.is_tablette() && self.finger_alive(ui) {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
            return;
        }
        let icon = match self.tool {
            Tool::Text => CursorIcon::Text,
            Tool::Lasso => CursorIcon::Crosshair,
            Tool::Image => CursorIcon::Copy,
            Tool::EraserStroke | Tool::EraserArea => CursorIcon::NotAllowed,
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
                    self.toast("Exported", t);
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
                self.toast("Exported", t);
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
    if s.is_empty() { "note".into() } else { s }
}

fn next_width(w: f32) -> f32 {
    const WIDTHS: [f32; 3] = [2.2, 5.0, 11.0];
    let i = WIDTHS
        .iter()
        .position(|&p| (w - p).abs() < 1.6)
        .unwrap_or(0);
    WIDTHS[(i + 1) % WIDTHS.len()]
}

fn shaft(p: &egui::Painter, a: Pos2, b: Pos2, thick: f32, col: Color32) {
    p.line_segment([a, b], Stroke::new(thick, col));
    p.circle_filled(a, thick * 0.48, col);
    p.circle_filled(b, thick * 0.48, col);
}

fn shade_rgb(c: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.12, 1.7);
    Color32::from_rgba_unmultiplied(
        (c.r() as f32 * k).min(255.0) as u8,
        (c.g() as f32 * k).min(255.0) as u8,
        (c.b() as f32 * k).min(255.0) as u8,
        c.a(),
    )
}

fn paint_copy_pages(p: &egui::Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(1.55_f32, fg);
    let a = Rect::from_center_size(c + vec2(-2.2, 1.6), vec2(11.4, 13.2));
    let b = Rect::from_center_size(c + vec2(2.4, -1.8), vec2(11.4, 13.2));
    p.rect_stroke(b, 2.4, st, StrokeKind::Inside);
    p.rect_stroke(a, 2.4, st, StrokeKind::Inside);
}

fn paint_pin(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.circle_filled(pos2(c.x, c.y - 3.6), 3.5, fg);
    p.circle_stroke(pos2(c.x, c.y - 3.6), 3.5, Stroke::new(1.2_f32, fg));
    p.line_segment(
        [pos2(c.x, c.y - 0.4), pos2(c.x, c.y + 8.0)],
        Stroke::new(1.7_f32, fg),
    );
    p.circle_filled(pos2(c.x, c.y + 8.0), 1.15, fg);
}

fn paint_bin(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.add(egui::Shape::convex_polygon(
        vec![
            pos2(c.x - 5.5, c.y - 3.0),
            pos2(c.x + 5.5, c.y - 3.0),
            pos2(c.x + 4.3, c.y + 7.5),
            pos2(c.x - 4.3, c.y + 7.5),
        ],
        fg.gamma_multiply(0.88),
        Stroke::NONE,
    ));
    let st = Stroke::new(1.65_f32, fg);
    p.line_segment([pos2(c.x - 6.8, c.y - 3.2), pos2(c.x + 6.8, c.y - 3.2)], st);
    p.line_segment([pos2(c.x - 2.3, c.y - 5.8), pos2(c.x + 2.3, c.y - 5.8)], st);
    p.line_segment(
        [pos2(c.x, c.y - 5.8), pos2(c.x, c.y - 3.2)],
        Stroke::new(1.45_f32, fg),
    );
    p.line_segment(
        [pos2(c.x - 2.2, c.y - 1.2), pos2(c.x - 1.6, c.y + 5.4)],
        Stroke::new(1.35_f32, shade_rgb(fg, 0.55)),
    );
    p.line_segment(
        [pos2(c.x + 2.2, c.y - 1.2), pos2(c.x + 1.6, c.y + 5.4)],
        Stroke::new(1.35_f32, shade_rgb(fg, 0.55)),
    );
}

fn paint_help(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.circle_stroke(c + vec2(0.0, -1.2), 8.2, Stroke::new(1.7_f32, fg));
    // "?"
    p.circle_filled(pos2(c.x, c.y + 5.6), 1.35, fg);
    let st = Stroke::new(1.85_f32, fg);
    p.line_segment([pos2(c.x - 3.2, c.y - 4.2), pos2(c.x - 1.2, c.y - 5.8)], st);
    p.line_segment([pos2(c.x - 1.2, c.y - 5.8), pos2(c.x + 2.8, c.y - 5.4)], st);
    p.line_segment([pos2(c.x + 2.8, c.y - 5.4), pos2(c.x + 2.4, c.y - 2.2)], st);
    p.line_segment([pos2(c.x + 2.4, c.y - 2.2), pos2(c.x, c.y + 0.4)], st);
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

fn paint_mode(p: &egui::Painter, c: Pos2, mode: NoteMode, fg: Color32) {
    match mode {
        NoteMode::Pupitre => {
            let body = Rect::from_center_size(pos2(c.x, c.y + 1.4), vec2(9.2, 13.4));
            p.rect_stroke(body, 4.5, Stroke::new(1.7_f32, fg), StrokeKind::Inside);
            p.line_segment(
                [pos2(c.x, c.y - 4.6), pos2(c.x, c.y + 0.6)],
                Stroke::new(1.6_f32, fg),
            );
            p.circle_filled(pos2(c.x, c.y - 4.6), 1.15, fg);
        }
        NoteMode::Tablette => {
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
    }
}

fn paint_tool(p: &egui::Painter, tool: Tool, c: Pos2, fg: Color32, cut: Color32) {
    let tilt = -0.96_f32;
    match tool {
        Tool::Fineliner => {
            capsule(p, at(c, tilt, -9.15, 0.0), at(c, tilt, 5.25, 0.0), 1.46, fg);
            paint_tri(p, c, tilt, 11.35, 4.65, 1.02, fg);
            capsule(
                p,
                at(c, tilt, -6.9, 2.42),
                at(c, tilt, -0.35, 2.42),
                0.46,
                fg,
            );
            capsule(p, at(c, tilt, -0.85, 0.0), at(c, tilt, 2.15, 0.0), 0.44, cut);
        }
        Tool::Brush => {
            paint_leaf(p, c, tilt, 11.2, 2.45, 2.22, fg);
            capsule(p, at(c, tilt, -9.15, 0.0), at(c, tilt, 1.7, 0.0), 1.52, fg);
            capsule(p, at(c, tilt, 0.85, 0.0), at(c, tilt, 2.75, 0.0), 2.22, fg);
            p.circle_filled(at(c, tilt, 5.25, 0.0), 0.92, cut);
            p.line_segment(
                [at(c, tilt, 5.25, 0.0), at(c, tilt, 8.45, 0.0)],
                Stroke::new(1.02_f32, cut),
            );
        }
        Tool::Pencil => {
            capsule(p, at(c, tilt, -9.85, 0.0), at(c, tilt, -7.45, 0.0), 1.62, fg);
            capsule(p, at(c, tilt, -4.05, 0.0), at(c, tilt, 5.2, 0.0), 1.78, fg);
            paint_tri(p, c, tilt, 11.15, 4.25, 2.18, fg);
            capsule(p, at(c, tilt, -7.7, 0.0), at(c, tilt, -4.45, 0.0), 2.16, fg);
            groove(p, c, tilt, -6.95, 1.35, 0.62, cut);
            groove(p, c, tilt, -5.85, 1.35, 0.62, cut);
            groove(p, c, tilt, 7.85, 0.72, 1.15, cut);
        }
        Tool::Highlighter => {
            paint_round_box(p, c, tilt, -9.7, 2.15, 3.05, 2.05, fg);
            p.add(egui::Shape::convex_polygon(
                vec![
                    at(c, tilt, 0.55, 3.28),
                    at(c, tilt, 9.15, 4.42),
                    at(c, tilt, 7.35, -4.42),
                    at(c, tilt, 0.55, -3.28),
                ],
                fg,
                Stroke::NONE,
            ));
            groove(p, c, tilt, -4.35, 1.7, 0.85, cut);
        }
        Tool::EraserStroke => {
            let ang = -0.38_f32;
            paint_round_box(p, c, ang, -8.15, 8.15, 3.72, 2.15, fg);
            groove(p, c, ang, -1.15, 2.35, 2.45, cut);
        }
        Tool::EraserArea => {
            let ang = -0.5_f32;
            paint_round_box(p, c, ang, -4.35, 4.35, 2.55, 1.45, fg);
            groove(p, c, ang, -0.7, 1.45, 1.7, cut);
            paint_brackets(p, c, 8.05, 2.55, fg);
        }
        Tool::Lasso => {
            let rot = -0.42_f32;
            let (rs, rc) = rot.sin_cos();
            let n = 30;
            let span = std::f32::consts::TAU * 0.92;
            let start = 1.05_f32;
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = start + span * (i as f32 / n as f32);
                let breathe = 6.35 + 0.7 * (t * 2.0).sin();
                let x = t.cos() * breathe;
                let y = t.sin() * breathe * 0.8;
                pts.push(pos2(
                    c.x - 0.15 + x * rc - y * rs,
                    c.y - 0.2 + x * rs + y * rc,
                ));
            }
            p.add(egui::Shape::line(pts.clone(), Stroke::new(2.15_f32, fg)));
            p.circle_filled(pts[0], 1.2, fg);
            if let Some(end) = pts.last() {
                p.circle_filled(*end, 1.2, fg);
            }
        }
        Tool::Text => {
            let o = c + vec2(0.0, 0.35);
            p.rect_filled(
                Rect::from_center_size(pos2(o.x, o.y - 6.55), vec2(14.8, 2.2)),
                0.7,
                fg,
            );
            for sx in [-6.45_f32, 6.45] {
                p.rect_filled(
                    Rect::from_center_size(pos2(o.x + sx, o.y - 5.35), vec2(1.85, 4.15)),
                    0.55,
                    fg,
                );
            }
            p.rect_filled(
                Rect::from_center_size(pos2(o.x, o.y + 0.45), vec2(2.15, 12.7)),
                0.55,
                fg,
            );
            p.rect_filled(
                Rect::from_center_size(pos2(o.x, o.y + 6.85), vec2(6.7, 2.05)),
                0.55,
                fg,
            );
        }
        Tool::Image => {
            let fr = Rect::from_center_size(c + vec2(0.15, 0.2), vec2(15.2, 12.2));
            p.rect_stroke(fr, 2.15, Stroke::new(1.8_f32, fg), StrokeKind::Inside);
            let crease = Stroke::new(1.45_f32, fg);
            let x1 = fr.right() - 2.05;
            let y0 = fr.top() + 1.85;
            p.line_segment([pos2(x1 - 3.15, y0), pos2(x1, y0 + 3.15)], crease);
            p.circle_filled(pos2(fr.left() + 4.35, fr.top() + 4.15), 1.42, fg);
            let base = fr.bottom() - 2.55;
            p.add(egui::Shape::convex_polygon(
                vec![
                    pos2(fr.left() + 2.35, base),
                    pos2(c.x - 1.7, c.y - 0.15),
                    pos2(c.x + 0.55, base),
                ],
                fg,
                Stroke::NONE,
            ));
            p.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x - 0.85, base),
                    pos2(c.x + 2.85, c.y + 1.35),
                    pos2(fr.right() - 2.25, base),
                ],
                fg,
                Stroke::NONE,
            ));
        }
    }
}

fn at(c: Pos2, ang: f32, along: f32, side: f32) -> Pos2 {
    let (s, co) = ang.sin_cos();
    pos2(c.x + co * along - s * side, c.y + s * along + co * side)
}

fn capsule(p: &egui::Painter, a: Pos2, b: Pos2, rad: f32, col: Color32) {
    if a.distance(b) < 0.35 {
        p.circle_filled(a, rad, col);
        return;
    }
    p.line_segment([a, b], Stroke::new(rad * 2.0, col));
    p.circle_filled(a, rad, col);
    p.circle_filled(b, rad, col);
}

fn paint_tri(p: &egui::Painter, c: Pos2, ang: f32, tip: f32, base: f32, half: f32, col: Color32) {
    p.add(egui::Shape::convex_polygon(
        vec![
            at(c, ang, tip, 0.0),
            at(c, ang, base, half),
            at(c, ang, base, -half),
        ],
        col,
        Stroke::NONE,
    ));
}

fn paint_leaf(p: &egui::Painter, c: Pos2, ang: f32, tip: f32, base: f32, half: f32, col: Color32) {
    let n = 7;
    let mut pts = Vec::with_capacity(n * 2 + 1);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let w = half * (1.0 - (1.0 - t) * (1.0 - t));
        let along = tip + (base - tip) * t;
        pts.push(at(c, ang, along, w));
    }
    for i in (1..=n).rev() {
        let t = i as f32 / n as f32;
        let w = half * (1.0 - (1.0 - t) * (1.0 - t));
        let along = tip + (base - tip) * t;
        pts.push(at(c, ang, along, -w));
    }
    p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
}

fn paint_round_box(
    p: &egui::Painter,
    c: Pos2,
    ang: f32,
    a0: f32,
    a1: f32,
    half: f32,
    rad: f32,
    col: Color32,
) {
    let rad = rad.min((a1 - a0) * 0.48).min(half * 0.98);
    let mut pts = Vec::with_capacity(20);
    let n = 4;
    let corners = [
        (a1 - rad, half - rad, 0.0, std::f32::consts::FRAC_PI_2),
        (
            a0 + rad,
            half - rad,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
        ),
        (
            a0 + rad,
            -half + rad,
            std::f32::consts::PI,
            std::f32::consts::PI * 1.5,
        ),
        (
            a1 - rad,
            -half + rad,
            std::f32::consts::PI * 1.5,
            std::f32::consts::TAU,
        ),
    ];
    for (cx, cy, a_from, a_to) in corners {
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let a = a_from + (a_to - a_from) * t;
            pts.push(at(c, ang, cx + a.cos() * rad, cy + a.sin() * rad));
        }
    }
    p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
}

fn groove(p: &egui::Painter, c: Pos2, ang: f32, along: f32, half: f32, width: f32, col: Color32) {
    p.line_segment(
        [at(c, ang, along, -half), at(c, ang, along, half)],
        Stroke::new(width, col),
    );
}

fn paint_brackets(p: &egui::Painter, c: Pos2, reach: f32, arm: f32, fg: Color32) {
    let st = Stroke::new(1.7_f32, fg);
    for sx in [-1.0_f32, 1.0] {
        for sy in [-1.0_f32, 1.0] {
            let o = pos2(c.x + sx * reach, c.y + sy * reach);
            p.line_segment([o, pos2(o.x - sx * arm, o.y)], st);
            p.line_segment([o, pos2(o.x, o.y - sy * arm)], st);
            p.circle_filled(o, 0.85, fg);
        }
    }
}
