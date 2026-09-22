#![allow(float_literal_f32_fallback)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::*;
use uuid::Uuid;

use crate::camera::{page_at_y, page_origin, Camera, ZOOM_STOPS};
use crate::document::{ImageObj, Note, PaperKind, TextBox, PAGE_H, PAGE_SPAN, PAGE_W};
use crate::emoji::{Atlas, FAVORITES};
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
const DOS_H: f32 = 156.0;
const DOS_PAD: f32 = 10.0;
const PAPER_PEEK: f32 = 18.0;
const TITLE_ROW: f32 = 30.0;

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
    Emoji,
    Rename,
    Select,
    Toggle,
    Range,
    Restore,
}

struct Toss {
    id: Uuid,
    from: Pos2,
    cover: u8,
    emoji: String,
    t0: f64,
    delay: f64,
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
    emoji: Atlas,
    emoji_tex: HashMap<char, TextureHandle>,
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
    /// Curseur souris masqué (stylet en proximité).
    cursor_off: bool,
    /// Stylet était en proximité la frame d’avant (pour PointerGone).
    pen_was_prox: bool,
    /// Sélecteur d’emoji ouvert pour ce note.
    emoji_pick: Option<Uuid>,
    /// Filtre dans la casse à caractères.
    emoji_query: String,
    /// Titre de dossier en cours d’édition.
    rename_id: Option<Uuid>,
    rename_buf: String,
    /// Le champ texte vient d’être posé : lui donner le clavier.
    text_focus: bool,
    /// Sélection style explorateur sur l’étagère.
    shelf_sel: Vec<Uuid>,
    shelf_anchor: Option<Uuid>,
    /// Vue corbeille (sinon étagère active).
    shelf_trash: bool,
    /// Faces des dos, pour le rectangle de sélection.
    shelf_slots: Vec<(Uuid, Rect)>,
    /// Origine du rectangle (None = pas de bande).
    shelf_band: Option<Pos2>,
    shelf_band_now: Option<Pos2>,
    shelf_band_add: bool,
    shelf_band_armed: bool,
    shelf_band_base: Vec<Uuid>,
    /// Glisser des dos vers le panier.
    shelf_haul: Option<Pos2>,
    shelf_haul_now: Option<Pos2>,
    shelf_haul_ids: Vec<Uuid>,
    shelf_haul_armed: bool,
    shelf_toss: Vec<Toss>,
    trash_rect: Rect,
    trash_mouth: Pos2,
    shelf_fed: bool,
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
            emoji: Atlas::load(),
            emoji_tex: HashMap::new(),
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
            cursor_off: false,
            pen_was_prox: false,
            emoji_pick: None,
            emoji_query: String::new(),
            rename_id: None,
            rename_buf: String::new(),
            text_focus: false,
            shelf_sel: Vec::new(),
            shelf_anchor: None,
            shelf_trash: false,
            shelf_slots: Vec::new(),
            shelf_band: None,
            shelf_band_now: None,
            shelf_band_add: false,
            shelf_band_armed: false,
            shelf_band_base: Vec::new(),
            shelf_haul: None,
            shelf_haul_now: None,
            shelf_haul_ids: Vec::new(),
            shelf_haul_armed: false,
            shelf_toss: Vec::new(),
            trash_rect: Rect::NOTHING,
            trash_mouth: Pos2::ZERO,
            shelf_fed: false,
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
            i.any_touches() || i.events.iter().any(|e| matches!(e, Event::Touch { .. }))
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
            48.0
        } else {
            40.0
        }
    }

    fn finish_text_edit(&mut self) {
        let Some((pg, id)) = self.editing_text.take() else {
            return;
        };
        self.text_focus = false;
        let empty = self
            .note
            .as_ref()
            .and_then(|n| n.pages.get(pg))
            .and_then(|p| p.texts.iter().find(|t| t.id == id))
            .map(|t| t.text.trim().is_empty())
            .unwrap_or(false);
        if empty {
            if let Some(n) = &mut self.note {
                if let Some(page) = n.pages.get_mut(pg) {
                    page.texts.retain(|t| t.id != id);
                }
            }
            self.mark_dirty();
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
    fn raw_input_hook(&mut self, ctx: &Context, raw_input: &mut egui::RawInput) {
        if self.tablet.ready() {
            self.tablet.pump_events();
            self.inject_pen_pointer(ctx, raw_input);
        }
    }

    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        self.tablet.ensure(frame);
        // Première attach : repaint pour que le hook lise les events au prochain tour.
        if self.tablet.ready() && !self.tablet.wants_repaint() {
            // no-op — wants_repaint couvre proximité
        }
        let pen = self.tablet.snapshot();
        if let Some(p) = pen.pressure {
            self.pressure.push_touch(p);
        } else if !pen.in_proximity {
            self.pressure.clear();
        }
        if pen.in_proximity {
            self.palm_grace_until = Some(Instant::now() + Duration::from_millis(180));
            if matches!(self.scene, Scene::Desk) && !self.prox_flipped && !self.is_tablette() {
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

        self.sync_pen_cursor(ctx);

        let focused = ctx.input(|i| i.focused);
        let busy = self.live.is_some()
            || !self.lasso.is_empty()
            || self.dirty
            || !self.sel.is_empty()
            || self.dock_float.is_some()
            || wants_pen
            || self.toast.is_some()
            || pen.in_proximity
            || pen.down;
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
            if (c && i.key_pressed(Key::Num0)) || (!typing && !c && i.key_pressed(Key::Num0)) {
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
                        self.finish_text_edit();
                    } else if !self.sel.is_empty() {
                        self.sel.clear();
                    } else {
                        self.close_desk();
                    }
                }
                Scene::Shelf { .. } => {
                    if !self.shelf_sel.is_empty() {
                        self.shelf_sel.clear();
                        self.shelf_anchor = None;
                    } else if self.shelf_trash {
                        self.shelf_trash = false;
                    } else if self.emoji_pick.is_some() {
                        self.emoji_pick = None;
                        self.emoji_query.clear();
                    }
                }
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
        // Raccourcis étagère (sélection / corbeille)
        if self.emoji_pick.is_none() && self.rename_id.is_none() {
            self.shelf_keys(ctx);
            self.shelf_pointer_tick(ctx);
            self.shelf_toss_tick(ctx);
            if self.shelf_haul_armed || !self.shelf_toss.is_empty() {
                ctx.request_repaint();
            }
        }

        let in_bin = self.shelf_trash;
        let room = if in_bin {
            mix_col(self.look.desk, self.look.rust(), 0.14)
        } else {
            self.look.desk
        };
        CentralPanel::default()
            .frame(Frame::NONE.fill(room))
            .show(ctx, |ui| {
                ui.add_space(64.0);
                if self.shelf_trash {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        ui.label(
                            RichText::new("Corbeille")
                                .font(self.look.serif(18.0))
                                .color(self.look.fg_dim),
                        );
                        ui.add_space(12.0);
                        if !self.lib.index.trash.is_empty()
                            && ui
                                .add(
                                    Label::new(
                                        RichText::new("vider")
                                            .font(self.look.mono(12.0))
                                            .color(self.look.rust()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .clicked()
                        {
                            self.lib.empty_trash();
                            self.shelf_sel.clear();
                        }
                    });
                }
                ui.add_space(if self.shelf_trash { 12.0 } else { 28.0 });

                let query = match &self.scene {
                    Scene::Shelf { query } => query.to_lowercase(),
                    _ => String::new(),
                };
                let source: Vec<_> = if self.shelf_trash {
                    self.lib.index.trash.clone()
                } else {
                    self.lib.index.notes.clone()
                };
                let notes: Vec<_> = source
                    .into_iter()
                    .filter(|m| query.is_empty() || m.title.to_lowercase().contains(&query))
                    .filter(|m| !self.shelf_toss.iter().any(|t| t.id == m.id))
                    .collect();

                // Clic dans le vide : désélection
                let bg = ui.interact(
                    ui.max_rect(),
                    Id::new("shelf-bg"),
                    Sense::click(),
                );
                if bg.clicked() && !ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                    // ne clear que si le clic n'est pas sur un dos (les dos prennent le focus après)
                }

                if notes.is_empty() {
                    self.shelf_slots.clear();
                    ui.add_space(48.0);
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        let empty = if self.shelf_trash {
                            "Vide"
                        } else {
                            "No notes yet"
                        };
                        ui.label(
                            RichText::new(empty)
                                .font(self.look.serif(22.0))
                                .color(self.look.fg_dim),
                        );
                    });
                } else {
                    let mut open = None;
                    let mut del = None;
                    let mut dup = None;
                    let mut pin = None;
                    let mut emoji_for = None;
                    let mut rename_for = None;
                    let mut restore = None;
                    let mut select = None;
                    let mut toggle = None;
                    let mut range = None;
                    let ids: Vec<Uuid> = notes.iter().map(|n| n.id).collect();
                    self.shelf_slots.clear();

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
                                    let selected = self.shelf_sel.contains(&meta.id);
                                    match self.cahier_dos(ui, &meta, selected) {
                                        Some(DosAct::Open) => open = Some(meta.id),
                                        Some(DosAct::Dup) => dup = Some(meta.id),
                                        Some(DosAct::Pin) => pin = Some(meta.id),
                                        Some(DosAct::Del) => del = Some(meta.id),
                                        Some(DosAct::Emoji) => emoji_for = Some(meta.id),
                                        Some(DosAct::Rename) => rename_for = Some(meta.id),
                                        Some(DosAct::Restore) => restore = Some(meta.id),
                                        Some(DosAct::Select) => select = Some(meta.id),
                                        Some(DosAct::Toggle) => toggle = Some(meta.id),
                                        Some(DosAct::Range) => range = Some(meta.id),
                                        None => {}
                                    }
                                    ui.add_space(gap);
                                }
                            });
                            ui.add_space(8.0);
                        }
                    });

                    if !self.shelf_band_armed {
                    if let Some(id) = select {
                        self.shelf_sel = vec![id];
                        self.shelf_anchor = Some(id);
                    }
                    if let Some(id) = toggle {
                        if let Some(p) = self.shelf_sel.iter().position(|x| *x == id) {
                            self.shelf_sel.remove(p);
                        } else {
                            self.shelf_sel.push(id);
                        }
                        self.shelf_anchor = Some(id);
                    }
                    if let Some(id) = range {
                        let anchor = self.shelf_anchor.or_else(|| self.shelf_sel.last().copied());
                        if let Some(a) = anchor {
                            if let (Some(ia), Some(ib)) = (
                                ids.iter().position(|x| *x == a),
                                ids.iter().position(|x| *x == id),
                            ) {
                                let (lo, hi) = if ia <= ib { (ia, ib) } else { (ib, ia) };
                                self.shelf_sel = ids[lo..=hi].to_vec();
                            }
                        } else {
                            self.shelf_sel = vec![id];
                            self.shelf_anchor = Some(id);
                        }
                    }
                    }
                    if let Some(id) = emoji_for {
                        self.emoji_pick = Some(id);
                        self.rename_id = None;
                    }
                    if let Some(id) = rename_for {
                        if let Some(m) = self.lib.index.notes.iter().find(|n| n.id == id) {
                            self.rename_buf = m.title.clone();
                        }
                        self.rename_id = Some(id);
                        self.emoji_pick = None;
                    }
                    if let Some(id) = open {
                        if !self.shelf_trash && !self.shelf_band_armed {
                            self.emoji_pick = None;
                            self.rename_id = None;
                            self.shelf_sel.clear();
                            self.open_note(id);
                        }
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
                    if let Some(id) = restore {
                        self.lib.restore_note(id);
                        self.shelf_sel.retain(|x| *x != id);
                    }
                    if let Some(id) = del {
                        if self.shelf_trash {
                            self.lib.purge_trashed(id);
                        } else {
                            self.lib.trash_note(id);
                        }
                        self.shelf_sel.retain(|x| *x != id);
                        if self.rename_id == Some(id) {
                            self.rename_id = None;
                        }
                    }
                }
            });

        if let (Some(a), Some(b)) = (self.shelf_band, self.shelf_band_now) {
            if self.shelf_band_armed {
                Area::new(Id::new("shelf-band"))
                    .fixed_pos(ctx.screen_rect().min)
                    .order(Order::Foreground)
                    .interactable(false)
                    .show(ctx, |ui| {
                        let r = Rect::from_two_pos(a, b);
                        let p = ui.painter();
                        let fill = self.look.accent.gamma_multiply(0.16);
                        let stroke = Stroke::new(1.15_f32, self.look.accent.gamma_multiply(0.85));
                        p.rect_filled(r, CornerRadius::same(2), fill);
                        p.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Inside);
                    });
            }
        }

        // Panier à papier — coin du pupitre
        Area::new(Id::new("fab-corbeille"))
            .anchor(Align2::LEFT_BOTTOM, vec2(18.0, -16.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let n = self.lib.index.trash.len() + self.shelf_toss.len();
                let hungry = self.shelf_haul_armed
                    && self
                        .shelf_haul_now
                        .is_some_and(|p| self.trash_rect.expand(18.0).contains(p));
                let resp = self
                    .wastebasket(ui, self.shelf_trash, n, hungry)
                    .on_hover_text(if self.shelf_trash {
                        "Shelf"
                    } else {
                        "Trash"
                    });
                if resp.clicked() && !self.shelf_fed && !self.shelf_haul_armed {
                    self.shelf_trash = !self.shelf_trash;
                    self.shelf_sel.clear();
                    self.shelf_anchor = None;
                    if let Scene::Shelf { query } = &mut self.scene {
                        query.clear();
                    }
                }
            });

        Area::new(Id::new("shelf-search"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 16.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                if let Scene::Shelf { query } = &mut self.scene {
                    let rail = 168.0;
                    let search_w = (ctx.screen_rect().width() - rail * 2.0)
                        .clamp(200.0, 560.0);
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
                }
            });

        let fiche_ouverte = !self.lib.index.fiche_pliee;
        Area::new(Id::new("shelf-top-right"))
            .anchor(Align2::RIGHT_TOP, vec2(-16.0, 10.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                    if self
                        .help_signet(ui, fiche_ouverte)
                        .on_hover_text(if fiche_ouverte { "Tuck away" } else { "Help" })
                        .clicked()
                    {
                        self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
                        self.lib.save_index();
                    }
                    if !self.shelf_trash
                        && self.inkwell(ui).on_hover_text("New  ·  N").clicked()
                    {
                        self.new_note();
                    }
                });
            });

        // Barre d'actions si sélection
        if !self.shelf_sel.is_empty() && self.emoji_pick.is_none() {
            let n = self.shelf_sel.len();
            Area::new(Id::new("shelf-sel-bar"))
                .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -28.0))
                .order(Order::Foreground)
                .show(ctx, |ui| {
                    Frame::NONE
                        .fill(self.look.desk_deep)
                        .stroke(Stroke::new(1.0_f32, self.look.desk_edge))
                        .corner_radius(18)
                        .inner_margin(Margin::symmetric(16, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{n} selected"))
                                        .font(self.look.mono(12.0))
                                        .color(self.look.fg),
                                );
                                ui.add_space(14.0);
                                if self.shelf_trash {
                                    if ui
                                        .add(
                                            Label::new(
                                                RichText::new("restore")
                                                    .font(self.look.mono(12.0))
                                                    .color(self.look.accent),
                                            )
                                            .sense(Sense::click()),
                                        )
                                        .clicked()
                                    {
                                        for id in self.shelf_sel.clone() {
                                            self.lib.restore_note(id);
                                        }
                                        self.shelf_sel.clear();
                                    }
                                    ui.add_space(10.0);
                                    if ui
                                        .add(
                                            Label::new(
                                                RichText::new("delete")
                                                    .font(self.look.mono(12.0))
                                                    .color(
                                                        self.look
                                                            .inks
                                                            .get(2)
                                                            .copied()
                                                            .unwrap_or(self.look.accent),
                                                    ),
                                            )
                                            .sense(Sense::click()),
                                        )
                                        .clicked()
                                    {
                                        for id in self.shelf_sel.clone() {
                                            self.lib.purge_trashed(id);
                                        }
                                        self.shelf_sel.clear();
                                    }
                                } else if ui
                                    .add(
                                        Label::new(
                                            RichText::new("trash")
                                                .font(self.look.mono(12.0))
                                                .color(self.look.rust()),
                                        )
                                        .sense(Sense::click()),
                                    )
                                    .clicked()
                                {
                                    for id in self.shelf_sel.clone() {
                                        self.lib.trash_note(id);
                                    }
                                    self.shelf_sel.clear();
                                }
                            });
                        });
                });
        }

        if fiche_ouverte && !self.shelf_trash {
            Area::new(Id::new("shelf-tuto-fiche"))
                .anchor(Align2::RIGHT_TOP, vec2(-16.0, 88.0))
                .order(Order::Foreground)
                .show(ctx, |ui| {
                    self.fiche_pupitre(ui);
                });
        }
        self.paint_shelf_haul(ctx);
        if let Some(id) = self.emoji_pick {
            self.ui_emoji_picker(ctx, id);
        }
        if ctx.input(|i| i.pointer.primary_released()) {
            self.shelf_band = None;
            self.shelf_band_now = None;
            self.shelf_band_armed = false;
        }
        self.shelf_fed = false;
    }

    fn shelf_keys(&mut self, ctx: &Context) {
        let typing = ctx.wants_keyboard_input();
        let mut clear = false;
        let mut trash_sel = false;
        let mut select_all = false;
        let mut open_one = false;
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                clear = true;
            }
            if !typing && i.modifiers.command && i.key_pressed(Key::A) {
                select_all = true;
            }
            if !typing && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                trash_sel = true;
            }
            if !typing && i.key_pressed(Key::Enter) && self.shelf_sel.len() == 1 {
                open_one = true;
            }
        });
        if clear {
            if !self.shelf_sel.is_empty() {
                self.shelf_sel.clear();
                self.shelf_anchor = None;
            } else if self.shelf_trash {
                self.shelf_trash = false;
            }
        }
        if select_all {
            let src = if self.shelf_trash {
                &self.lib.index.trash
            } else {
                &self.lib.index.notes
            };
            self.shelf_sel = src.iter().map(|m| m.id).collect();
        }
        if trash_sel && !self.shelf_sel.is_empty() {
            let ids = self.shelf_sel.clone();
            if self.shelf_trash {
                for id in ids {
                    self.lib.purge_trashed(id);
                }
            } else {
                for id in ids {
                    self.lib.trash_note(id);
                }
            }
            self.shelf_sel.clear();
        }
        if open_one && !self.shelf_trash {
            if let Some(id) = self.shelf_sel.first().copied() {
                self.shelf_sel.clear();
                self.open_note(id);
            }
        }
    }

    fn shelf_pointer_tick(&mut self, ctx: &Context) {
        let typing = ctx.wants_keyboard_input();
        let (pressed, down, released, pos, add) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.command || i.modifiers.shift,
            )
        });
        let Some(pos) = pos else {
            if released {
                self.shelf_band = None;
                self.shelf_band_now = None;
                self.shelf_band_armed = false;
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
            }
            return;
        };
        let screen = ctx.screen_rect();
        let chrome = pos.y < screen.min.y + 76.0 || pos.x < screen.min.x + 12.0;
        let on_trash = self.trash_rect.expand(12.0).contains(pos);
        let hit = self
            .shelf_slots
            .iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(id, _)| *id);

        if pressed && !typing && !on_trash {
            if let Some(id) = hit {
                if !add && !self.shelf_trash {
                    self.shelf_haul = Some(pos);
                    self.shelf_haul_now = Some(pos);
                    self.shelf_haul_armed = false;
                    self.shelf_haul_ids = if self.shelf_sel.contains(&id) {
                        self.shelf_sel.clone()
                    } else {
                        vec![id]
                    };
                } else {
                    self.shelf_band = Some(pos);
                    self.shelf_band_now = Some(pos);
                    self.shelf_band_add = add;
                    self.shelf_band_armed = false;
                    self.shelf_band_base = self.shelf_sel.clone();
                }
            } else if !chrome && pos.y < screen.max.y - 100.0 {
                self.shelf_band = Some(pos);
                self.shelf_band_now = Some(pos);
                self.shelf_band_add = add;
                self.shelf_band_armed = false;
                self.shelf_band_base = self.shelf_sel.clone();
            }
        }
        if down {
            if let Some(origin) = self.shelf_haul {
                self.shelf_haul_now = Some(pos);
                if origin.distance(pos) > 12.0 {
                    self.shelf_haul_armed = true;
                    if let Some(id) = self.shelf_haul_ids.first().copied() {
                        if !self.shelf_sel.contains(&id) {
                            self.shelf_sel = self.shelf_haul_ids.clone();
                            self.shelf_anchor = Some(id);
                        }
                    }
                    ctx.request_repaint();
                }
            } else if let Some(origin) = self.shelf_band {
                self.shelf_band_now = Some(pos);
                if origin.distance(pos) > 11.0 {
                    self.shelf_band_armed = true;
                    let band = Rect::from_two_pos(origin, pos);
                    let hits: Vec<Uuid> = self
                        .shelf_slots
                        .iter()
                        .filter(|(_, face)| face.intersects(band))
                        .map(|(id, _)| *id)
                        .collect();
                    if self.shelf_band_add {
                        let mut out = self.shelf_band_base.clone();
                        for id in hits {
                            if !out.contains(&id) {
                                out.push(id);
                            }
                        }
                        self.shelf_sel = out;
                    } else {
                        self.shelf_sel = hits;
                    }
                    if let Some(id) = self.shelf_sel.last().copied() {
                        self.shelf_anchor = Some(id);
                    }
                    ctx.request_repaint();
                }
            }
        }
        if released {
            if self.shelf_haul_armed {
                let over = self
                    .shelf_haul_now
                    .is_some_and(|p| self.trash_rect.expand(22.0).contains(p));
                if over && !self.shelf_trash {
                    self.begin_toss(ctx.input(|i| i.time));
                    self.shelf_fed = true;
                }
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
                self.shelf_haul_ids.clear();
            } else if self.shelf_haul.is_some() {
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_ids.clear();
            }
            if !self.shelf_band_armed {
                if let Some(origin) = self.shelf_band {
                    let on_slot = self.shelf_slots.iter().any(|(_, r)| r.contains(origin));
                    if !on_slot && !self.shelf_band_add && !chrome && !on_trash {
                        self.shelf_sel.clear();
                        self.shelf_anchor = None;
                    }
                }
                self.shelf_band = None;
                self.shelf_band_now = None;
            }
        }
    }

    fn begin_toss(&mut self, now: f64) {
        let from = self.shelf_haul_now.unwrap_or(self.trash_mouth);
        let ids = self.shelf_haul_ids.clone();
        for (i, id) in ids.iter().enumerate() {
            let meta = self
                .lib
                .index
                .notes
                .iter()
                .find(|m| m.id == *id)
                .cloned();
            let Some(meta) = meta else {
                continue;
            };
            self.shelf_toss.push(Toss {
                id: *id,
                from: from + vec2(i as f32 * 10.0, i as f32 * 7.0),
                cover: meta.cover,
                emoji: meta.emoji.clone(),
                t0: now,
                delay: i as f64 * 0.055,
            });
        }
        self.shelf_sel.retain(|id| !ids.contains(id));
    }

    fn shelf_toss_tick(&mut self, ctx: &Context) {
        if self.shelf_toss.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        let dur = 0.46;
        let mut done = Vec::new();
        self.shelf_toss.retain(|t| {
            if now - t.t0 - t.delay >= dur {
                done.push(t.id);
                false
            } else {
                true
            }
        });
        for id in done {
            self.lib.trash_note(id);
        }
        ctx.request_repaint();
    }

    fn wastebasket(&mut self, ui: &mut Ui, open: bool, count: usize, hungry: bool) -> Response {
        let hit = 52.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(hit, hit), Sense::click());
        self.trash_rect = rect;
        let id = Id::new("waste-basket");
        let hover_t = ui.ctx().animate_bool_with_time(
            id.with("h"),
            resp.hovered() || hungry || open,
            0.16,
        );
        let e = hover_t * hover_t * (3.0 - 2.0 * hover_t);
        let rust = self.look.rust();
        let col = if open || hungry || count > 0 {
            shade_rgb(rust, 1.1 + 0.08 * e)
        } else {
            rust
        };
        let side = 34.0 + e * 3.0;
        let icon = Rect::from_center_size(rect.center(), vec2(side, side));
        self.trash_mouth = pos2(icon.center().x, icon.min.y + side * 0.28);
        let tex = self.trash_icon(ui.ctx(), col);
        ui.painter().image(
            tex.id(),
            icon,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        resp.on_hover_cursor(if hungry {
            CursorIcon::Move
        } else {
            CursorIcon::PointingHand
        })
    }

    fn trash_icon(&mut self, ctx: &Context, color: Color32) -> TextureHandle {
        let key = format!("icon-trash-{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
        if let Some(tex) = self.textures.get(&key) {
            return tex.clone();
        }
        let img = raster_lucide_trash(color, 128);
        let tex = ctx.load_texture(&key, img, TextureOptions::LINEAR);
        self.textures.insert(key, tex.clone());
        tex
    }

    fn paint_dos_at(
        &mut self,
        ctx: &Context,
        painter: &Painter,
        center: Pos2,
        scale: f32,
        alpha: f32,
        cover: u8,
        emoji: &str,
    ) {
        let w = DOS_W * scale;
        let h = DOS_H * scale;
        let face = Rect::from_center_size(center, vec2(w, h));
        let fade = |c: Color32| {
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
        };
        let cloth = fade(self.look.cloth_at(cover));
        let cloth_deep = fade(shade_rgb(self.look.cloth_at(cover), 0.70));
        let paper = fade(self.look.paper);
        painter.rect_filled(
            face.translate(vec2(2.0 * scale, 3.0 * scale)),
            CornerRadius::same(8),
            fade(self.look.shadow.gamma_multiply(0.35)),
        );
        painter.rect_filled(face, CornerRadius::same(8), cloth);
        let spine = Rect::from_min_max(face.min, pos2(face.min.x + 13.0 * scale, face.max.y));
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
        painter.rect_stroke(
            face,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, fade(shade_rgb(self.look.cloth_at(cover), 0.48))),
            StrokeKind::Inside,
        );
        let logo = Rect::from_center_size(face.center(), vec2(56.0 * scale, 56.0 * scale));
        if emoji.trim().is_empty() || !self.paint_emoji(ctx, painter, logo, emoji) {
            painter.text(
                face.center(),
                Align2::CENTER_CENTER,
                "+",
                self.look.serif(28.0 * scale),
                fade(self.look.paper.gamma_multiply(0.55)),
            );
        }
        let _ = paper;
    }

    fn paint_shelf_haul(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        let mut ghosts: Vec<(Pos2, f32, f32, u8, String)> = Vec::new();
        if self.shelf_haul_armed {
            if let Some(pos) = self.shelf_haul_now {
                for (i, id) in self.shelf_haul_ids.iter().enumerate() {
                    if let Some(m) = self.lib.index.notes.iter().find(|n| n.id == *id) {
                        ghosts.push((
                            pos + vec2(i as f32 * 11.0, i as f32 * 8.0),
                            0.88,
                            1.0,
                            m.cover,
                            m.emoji.clone(),
                        ));
                    }
                }
            }
        }
        let mouth = self.trash_mouth;
        let dur = 0.46;
        for t in &self.shelf_toss {
            let u = ((now - t.t0 - t.delay) / dur).clamp(0.0, 1.0) as f32;
            if now - t.t0 < t.delay {
                ghosts.push((t.from, 0.88, 1.0, t.cover, t.emoji.clone()));
                continue;
            }
            let s = u * u;
            let p = t.from.lerp(mouth, s);
            let dip = (s * std::f32::consts::PI).sin() * 36.0 * (1.0 - s);
            ghosts.push((
                p + vec2(0.0, dip),
                0.88 * (1.0 - 0.78 * s),
                1.0 - 0.55 * s,
                t.cover,
                t.emoji.clone(),
            ));
        }
        if ghosts.is_empty() {
            return;
        }
        Area::new(Id::new("shelf-haul"))
            .fixed_pos(ctx.screen_rect().min)
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let painter = ui.painter().clone();
                for (c, sc, a, cover, em) in ghosts {
                    self.paint_dos_at(ctx, &painter, c, sc, a, cover, &em);
                }
            });
    }

    /// Signet de la fiche : papier, cran en V, astérisque de marge.
    /// Signet papier, à droite de la recherche.
    fn help_signet(&self, ui: &mut Ui, open: bool) -> Response {
        let w = 32.0;
        let h = 66.0;
        // Marge à droite pour l'ombre, à gauche pour la penche.
        let (rect, resp) = ui.allocate_exact_size(vec2(w + 16.0, h + 14.0), Sense::click());
        let p = ui.painter();
        let id = Id::new("help-signet");
        let hover_t = ui
            .ctx()
            .animate_bool_with_time(id.with("h"), resp.hovered() || open, 0.2);
        let e = hover_t * hover_t * (3.0 - 2.0 * hover_t);
        // La queue part vers la page, pas vers le bord de la fenêtre.
        let lean = (1.0 - e) * -0.05;
        let drop = if open { 0.0 } else { e * 5.0 };

        let l = rect.left() + 2.0;
        let r = l + w;
        let t = rect.top();
        let b = t + h;
        let notch = 11.0;
        let shoulder = b - notch;
        let cx = (l + r) * 0.5;
        let origin = pos2(cx, t);

        let xform = |pt: Pos2| -> Pos2 {
            let d = pt - origin;
            let (s, c) = lean.sin_cos();
            origin + vec2(d.x * c - d.y * s, d.x * s + d.y * c) + vec2(0.0, drop)
        };
        let poly = |pts: &[Pos2], color: Color32| {
            p.add(Shape::convex_polygon(
                pts.iter().copied().map(xform).collect(),
                color,
                Stroke::NONE,
            ));
        };
        let seg = |a: Pos2, b: Pos2, stroke: Stroke| {
            p.line_segment([xform(a), xform(b)], stroke);
        };

        let paper = mix_col(self.look.paper, self.look.accent, 0.035);
        let ink = self.look.ink;
        let edge = ink.gamma_multiply(0.20);
        let thread = ink.gamma_multiply(0.28);

        // Le corps s'arrête au-dessus du cran ; les volets le recouvrent.
        // Sinon le bord du rectangle barre l'entrée du V.
        let y0 = shoulder - 2.2;
        let body = [
            pos2(l, t),
            pos2(r, t),
            pos2(r, y0 + 0.8),
            pos2(l, y0 + 0.8),
        ];
        let left_flap = [
            pos2(l, y0 - 0.8),
            pos2(cx, y0 - 0.8),
            pos2(cx, shoulder),
            pos2(l, b),
        ];
        let right_flap = [
            pos2(cx, y0 - 0.8),
            pos2(r, y0 - 0.8),
            pos2(r, b),
            pos2(cx, shoulder),
        ];

        for (dx, dy, a) in [(2.2, 4.2, 0.55_f32), (0.8, 1.6, 0.28_f32)] {
            let sh = |pt: Pos2| xform(pt) + vec2(dx, dy + e * 1.2);
            let shadow = self.look.shadow.gamma_multiply(a);
            for pts in [&body[..], &left_flap[..], &right_flap[..]] {
                p.add(Shape::convex_polygon(
                    pts.iter().copied().map(sh).collect(),
                    shadow,
                    Stroke::NONE,
                ));
            }
        }

        poly(&body, paper);
        poly(&left_flap, paper);
        poly(&right_flap, paper);

        let hair = Stroke::new(1.0_f32, edge);
        seg(pos2(l, t), pos2(r, t), hair);
        seg(pos2(r, t), pos2(r, b), hair);
        seg(pos2(r, b), pos2(cx, shoulder), hair);
        seg(pos2(cx, shoulder), pos2(l, b), hair);
        seg(pos2(l, b), pos2(l, t), hair);

        // Filet de tête, comme l'en-tête de la fiche.
        let rule = Stroke::new(1.15_f32, ink.gamma_multiply(0.55));
        seg(pos2(l + 6.0, t + 9.0), pos2(r - 6.0, t + 9.0), rule);
        seg(
            pos2(cx - 6.0, t + 12.4),
            pos2(cx + 6.0, t + 12.4),
            Stroke::new(0.8_f32, ink.gamma_multiply(0.28)),
        );

        // Trou de ruban : on voit le pupitre au travers.
        let hole = xform(pos2(cx, t + 20.0));
        p.circle_filled(hole, 2.15, self.look.desk_deep);
        p.circle_stroke(hole, 2.15, Stroke::new(0.9_f32, edge));

        // Astérisque de marge — la note, pas un point d'interrogation.
        let mark = mix_col(ink.gamma_multiply(0.62), self.look.accent, e);
        let star = xform(pos2(cx, t + 32.0));
        let arm = 4.6 + e * 0.5;
        for i in 0..3 {
            let a = i as f32 * std::f32::consts::FRAC_PI_3;
            let d = vec2(a.cos(), a.sin()) * arm;
            p.line_segment([star - d, star + d], Stroke::new(1.05_f32, mark));
        }

        let dash = Stroke::new(0.9_f32, thread);
        let mut y = t + 38.0;
        while y < shoulder - 3.0 {
            seg(pos2(cx, y), pos2(cx, (y + 3.2).min(shoulder - 2.0)), dash);
            y += 6.4;
        }

        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Feuille du cahier : même objet pour l'aide et les marques.
    fn paint_cahier_page(&self, p: &Painter, rect: Rect) -> Rect {
        let paper = mix_col(self.look.paper, self.look.accent, 0.028);
        let ink = self.look.ink;
        p.rect_filled(
            rect.translate(vec2(4.0, 7.0)),
            CornerRadius::same(3),
            self.look.shadow.gamma_multiply(0.42),
        );
        p.rect_filled(
            rect.translate(vec2(1.5, 2.5)),
            CornerRadius::same(3),
            self.look.shadow.gamma_multiply(0.18),
        );
        p.rect_filled(rect, CornerRadius::same(3), paper);
        p.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0_f32, ink.gamma_multiply(0.12)),
            StrokeKind::Inside,
        );
        let gutter = 30.0;
        let hx = rect.min.x + 15.0;
        for t in [0.20_f32, 0.50, 0.80] {
            let c = pos2(hx, rect.min.y + rect.height() * t);
            p.circle_filled(c, 3.1, self.look.desk);
            p.circle_stroke(c, 3.1, Stroke::new(0.85_f32, ink.gamma_multiply(0.20)));
        }
        p.line_segment(
            [
                pos2(rect.min.x + gutter, rect.min.y + 18.0),
                pos2(rect.min.x + gutter, rect.max.y - 18.0),
            ],
            Stroke::new(1.05_f32, self.look.paper_rule_strong),
        );
        Rect::from_min_max(
            pos2(rect.min.x + gutter + 18.0, rect.min.y + 20.0),
            pos2(rect.max.x - 22.0, rect.max.y - 18.0),
        )
    }

    fn page_head(&self, p: &Painter, inner: Rect, title: &str, kicker: &str) -> f32 {
        let ink = self.look.ink;
        p.text(
            inner.min,
            Align2::LEFT_TOP,
            title,
            self.look.serif(26.0),
            ink,
        );
        p.text(
            pos2(inner.min.x, inner.min.y + 32.0),
            Align2::LEFT_TOP,
            kicker,
            self.look.mono(11.0),
            ink.gamma_multiply(0.42),
        );
        let y = inner.min.y + 52.0;
        p.line_segment(
            [pos2(inner.min.x, y), pos2(inner.min.x + 52.0, y)],
            Stroke::new(1.7_f32, self.look.accent.gamma_multiply(0.88)),
        );
        p.line_segment(
            [pos2(inner.min.x + 56.0, y), pos2(inner.max.x, y)],
            Stroke::new(0.8_f32, self.look.paper_rule),
        );
        y + 16.0
    }

    fn fiche_pupitre(&mut self, ui: &mut Ui) {
        let w = 400.0_f32
            .min(ui.ctx().screen_rect().width() - 40.0)
            .max(300.0);
        let h = 428.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let p = ui.painter_at(rect);
        let inner = self.paint_cahier_page(&p, rect);
        let mut y = self.page_head(&p, inner, "Cahier", "les gestes");
        let ink = self.look.ink;
        let mute = ink.gamma_multiply(0.46);
        let rule = self.look.paper_rule;

        let sections: [(&str, &[(&str, &str)]); 2] = [
            (
                "étagère",
                &[
                    ("Nouveau", "N"),
                    ("Ouvrir", "double-clic"),
                    ("Choisir", "clic · bande"),
                    ("Corbeille", "glisser"),
                    ("Icône", "menu"),
                ],
            ),
            (
                "pupitre",
                &[
                    ("Dessiner", "stylet"),
                    ("Déplacer", "espace"),
                    ("Effacer", "E"),
                    ("Annuler", "Ctrl+Z"),
                ],
            ),
        ];
        for (label, rows) in sections {
            p.text(
                pos2(inner.min.x, y),
                Align2::LEFT_TOP,
                label,
                self.look.mono(10.0),
                mute,
            );
            y += 20.0;
            for (k, v) in rows {
                p.line_segment(
                    [pos2(inner.min.x, y + 22.0), pos2(inner.max.x, y + 22.0)],
                    Stroke::new(0.7_f32, rule),
                );
                p.text(
                    pos2(inner.min.x, y + 3.0),
                    Align2::LEFT_TOP,
                    *k,
                    self.look.serif(16.0),
                    ink,
                );
                p.text(
                    pos2(inner.max.x, y + 5.0),
                    Align2::RIGHT_TOP,
                    *v,
                    self.look.mono(12.0),
                    mute,
                );
                y += 26.0;
            }
            y += 10.0;
        }
        p.text(
            pos2(inner.center().x, inner.max.y - 2.0),
            Align2::CENTER_BOTTOM,
            "clic pour replier",
            self.look.mono(10.0),
            mute,
        );

        if resp.clicked() {
            self.lib.index.fiche_pliee = true;
            self.lib.save_index();
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Replier");
    }

    /// Peint une icône couleur dans `bounds`. Faux si la bitmap n'est pas là.
    fn paint_emoji(&mut self, ctx: &Context, painter: &Painter, bounds: Rect, emoji: &str) -> bool {
        let Some(ch) = self.emoji.key(emoji) else {
            return false;
        };
        if !self.emoji_tex.contains_key(&ch) {
            let img = if let Some(g) = self.emoji.ensure(ch) {
                ColorImage::from_rgba_unmultiplied([g.width, g.height], &g.rgba)
            } else {
                return false;
            };
            let tex = ctx.load_texture(format!("emoji-{ch}"), img, TextureOptions::LINEAR);
            self.emoji_tex.insert(ch, tex);
        }
        let Some(tex) = self.emoji_tex.get(&ch).cloned() else {
            return false;
        };
        let size = tex.size_vec2();
        let scale = (bounds.width() / size.x).min(bounds.height() / size.y);
        if !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        painter.image(
            tex.id(),
            Rect::from_center_size(bounds.center(), size * scale),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        true
    }

    /// Planche de timbres arrachée d'un catalogue — à détacher et coller sur le dos.
    fn ui_emoji_picker(&mut self, ctx: &Context, note_id: Uuid) {
        let mut chosen: Option<String> = None;
        let mut dismiss = false;
        let screen = ctx.screen_rect();
        let sheet_w = (screen.width() * 0.68).clamp(400.0, 700.0);
        let sheet_h = (screen.height() * 0.80).clamp(440.0, 680.0);

        Area::new(Id::new("emoji-dim"))
            .fixed_pos(screen.min)
            .order(Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::TRANSPARENT);
                if resp.clicked() {
                    dismiss = true;
                }
            });

        Area::new(Id::new("emoji-pick").with(note_id))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let paper = mix_col(self.look.paper, self.look.accent, 0.03);
                let ink = self.look.ink;
                let mute = ink.gamma_multiply(0.42);
                let (outer, _sheet_resp) =
                    ui.allocate_exact_size(vec2(sheet_w, sheet_h), Sense::hover());
                let p = ui.painter_at(outer);

                p.rect_filled(
                    outer.translate(vec2(5.0, 8.0)),
                    CornerRadius::same(2),
                    self.look.shadow.gamma_multiply(0.50),
                );
                p.rect_filled(
                    outer.translate(vec2(2.0, 3.0)),
                    CornerRadius::same(2),
                    self.look.shadow.gamma_multiply(0.22),
                );
                p.rect_filled(outer, CornerRadius::same(2), paper);

                // Bord arraché à gauche
                let mut tear = Vec::new();
                let mut y = outer.min.y;
                let mut flip = true;
                tear.push(pos2(outer.min.x + 10.0, outer.min.y));
                while y < outer.max.y {
                    y += 7.0;
                    let x = outer.min.x + if flip { 3.0 } else { 11.0 };
                    tear.push(pos2(x, y.min(outer.max.y)));
                    flip = !flip;
                }
                for w in tear.windows(2) {
                    p.line_segment([w[0], w[1]], Stroke::new(1.05_f32, ink.gamma_multiply(0.28)));
                }
                p.rect_filled(
                    Rect::from_min_max(outer.min, pos2(outer.min.x + 2.0, outer.max.y)),
                    0.0,
                    self.look.desk,
                );

                // Coin plié
                let ear = [
                    pos2(outer.max.x - 26.0, outer.min.y),
                    pos2(outer.max.x, outer.min.y),
                    pos2(outer.max.x, outer.min.y + 26.0),
                ];
                p.add(Shape::convex_polygon(
                    ear.to_vec(),
                    mix_col(paper, ink, 0.10),
                    Stroke::NONE,
                ));
                p.line_segment(
                    [ear[0], ear[2]],
                    Stroke::new(1.0_f32, ink.gamma_multiply(0.16)),
                );

                p.rect_stroke(
                    outer,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, ink.gamma_multiply(0.14)),
                    StrokeKind::Inside,
                );

                let inner = outer.shrink2(vec2(22.0, 16.0));
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Icons")
                                .font(self.look.serif(24.0))
                                .color(ink),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("for the spine")
                                .font(self.look.mono(11.0))
                                .color(mute),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    Label::new(
                                        RichText::new("close")
                                            .font(self.look.mono(11.0))
                                            .color(mute),
                                    )
                                    .sense(Sense::click()),
                                )
                                .clicked()
                            {
                                dismiss = true;
                            }
                        });
                    });
                    ui.add_space(2.0);
                    let rule_y = ui.cursor().top();
                    ui.painter().line_segment(
                        [pos2(inner.min.x, rule_y), pos2(inner.max.x - 20.0, rule_y)],
                        Stroke::new(1.2_f32, self.look.accent.gamma_multiply(0.75)),
                    );
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("favorites")
                                .font(self.look.mono(10.0))
                                .color(mute),
                        );
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for em in FAVORITES {
                            if self.stamp_cell(ui, ctx, paper, ink, Some(em), &mut chosen) {
                                // chosen
                            }
                        }
                        if self.stamp_cell(ui, ctx, paper, ink, None, &mut chosen) {
                            chosen = Some(String::new());
                        }
                    });
                    ui.add_space(10.0);

                    let line = ui.cursor();
                    ui.painter().line_segment(
                        [
                            pos2(inner.min.x, line.top() + 20.0),
                            pos2(inner.max.x, line.top() + 20.0),
                        ],
                        Stroke::new(0.8_f32, self.look.paper_rule),
                    );
                    let te = TextEdit::singleline(&mut self.emoji_query)
                        .hint_text("paste an icon")
                        .font(self.look.serif(15.0))
                        .text_color(ink)
                        .frame(false);
                    ui.add(te.desired_width(inner.width()));
                    ui.add_space(8.0);

                    let q = self.emoji_query.trim().to_string();
                    let catalog: Vec<char> = {
                        let all = self.emoji.catalog();
                        if q.is_empty() {
                            all.to_vec()
                        } else if q.chars().any(|c| all.contains(&c)) {
                            all.iter().copied().filter(|ch| q.contains(*ch)).collect()
                        } else {
                            all.to_vec()
                        }
                    };

                    let cell = 40.0;
                    let gap = 2.0;
                    let cols = ((inner.width() - 8.0) / (cell + gap)).floor().max(6.0) as usize;
                    let grid_h = (inner.max.y - ui.cursor().top() - 8.0).max(160.0);
                    ScrollArea::vertical().max_height(grid_h).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(gap, gap);
                        let mut i = 0;
                        while i < catalog.len() {
                            ui.horizontal(|ui| {
                                for _ in 0..cols {
                                    if i >= catalog.len() {
                                        break;
                                    }
                                    let em = catalog[i].to_string();
                                    i += 1;
                                    self.stamp_cell(ui, ctx, paper, ink, Some(&em), &mut chosen);
                                }
                            });
                        }
                    });
                });
            });

        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            dismiss = true;
        }
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Paste(_)))) {
            if let Some(s) = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    Event::Paste(t) => Some(t.clone()),
                    _ => None,
                })
            }) {
                let t = s.trim().to_string();
                if self.emoji.key(&t).is_some() {
                    chosen = Some(t);
                } else {
                    self.emoji_query = t;
                }
            }
        }

        if let Some(em) = chosen {
            if let Some(mut n) = self.lib.load_note(note_id) {
                n.emoji = em;
                n.touch();
                self.lib.save_note(&n);
            }
            self.emoji_pick = None;
            self.emoji_query.clear();
        } else if dismiss {
            self.emoji_pick = None;
            self.emoji_query.clear();
        }
    }

    fn stamp_cell(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        paper: Color32,
        ink: Color32,
        em: Option<&str>,
        chosen: &mut Option<String>,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
        let p = ui.painter();
        let edge = ink.gamma_multiply(if resp.hovered() { 0.38 } else { 0.16 });
        p.rect_filled(
            rect,
            CornerRadius::same(1),
            if resp.hovered() {
                mix_col(paper, self.look.accent, 0.10)
            } else {
                paper
            },
        );
        // Dentelure
        let step = 4.0;
        let mut x = rect.min.x + 2.0;
        while x < rect.max.x - 1.0 {
            p.circle_filled(pos2(x, rect.min.y), 0.85, self.look.desk.gamma_multiply(0.35));
            p.circle_filled(pos2(x, rect.max.y), 0.85, self.look.desk.gamma_multiply(0.35));
            x += step;
        }
        let mut y = rect.min.y + 2.0;
        while y < rect.max.y - 1.0 {
            p.circle_filled(pos2(rect.min.x, y), 0.85, self.look.desk.gamma_multiply(0.35));
            p.circle_filled(pos2(rect.max.x, y), 0.85, self.look.desk.gamma_multiply(0.35));
            y += step;
        }
        p.rect_stroke(rect, CornerRadius::same(1), Stroke::new(0.7_f32, edge), StrokeKind::Inside);
        match em {
            Some(em) => {
                self.paint_emoji(ctx, &p, rect.shrink(5.0), em);
                if resp.clicked() {
                    *chosen = Some(em.to_string());
                    return true;
                }
            }
            None => {
                p.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    self.look.mono(15.0),
                    ink.gamma_multiply(0.45),
                );
                if resp.clicked() {
                    *chosen = Some(String::new());
                    return true;
                }
            }
        }
        false
    }

    fn cahier_dos(
        &mut self,
        ui: &mut Ui,
        meta: &crate::library::NoteMeta,
        selected: bool,
    ) -> Option<DosAct> {
        let slot = vec2(
            DOS_W + DOS_PAD * 2.0,
            DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW,
        );
        let (slot_rect, resp) = ui.allocate_exact_size(slot, Sense::click());
        let id = Id::new("cahier-dos").with(meta.id);
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = pointer.is_some_and(|p| slot_rect.contains(p));
        if over {
            ui.ctx().request_repaint();
        }
        let lift_t = ui
            .ctx()
            .animate_bool_with_time(id.with("peek"), over, 0.16);
        let lift = lift_t * lift_t * (3.0 - 2.0 * lift_t);
        let discarded = self.shelf_trash;
        let e = if discarded { lift * 0.2 } else { lift };
        let cloth = if discarded {
            mix_col(self.look.cloth_at(meta.cover), self.look.desk, 0.28)
        } else {
            self.look.cloth_at(meta.cover)
        };
        let cloth_deep = shade_rgb(cloth, 0.70);
        let cloth_edge = shade_rgb(cloth, 0.48);
        let paper = self.look.paper;

        let face = Rect::from_min_size(
            pos2(
                slot_rect.center().x - DOS_W * 0.5,
                slot_rect.min.y + DOS_PAD + PAPER_PEEK,
            ),
            vec2(DOS_W, DOS_H),
        );
        let painter = ui.painter_at(slot_rect);
        let hit = Rect::from_min_max(
            face.min,
            pos2(face.max.x, face.max.y + TITLE_ROW),
        );
        self.shelf_slots.push((meta.id, hit));
        let lifted = self.shelf_haul_armed && self.shelf_haul_ids.contains(&meta.id);
        if lifted {
            painter.rect_filled(
                face.translate(vec2(2.0, 4.0)),
                CornerRadius::same(8),
                self.look.shadow.gamma_multiply(0.18),
            );
            return None;
        }

        if selected {
            let (r, g, b) = if self.look.dark {
                (255_u8, 255, 255)
            } else {
                (self.look.ink.r(), self.look.ink.g(), self.look.ink.b())
            };
            let halo = face.expand(5.0);
            painter.rect_filled(
                halo,
                CornerRadius::same(11),
                Color32::from_rgba_unmultiplied(r, g, b, if self.look.dark { 18 } else { 16 }),
            );
            painter.rect_stroke(
                halo,
                CornerRadius::same(11),
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(r, g, b, if self.look.dark { 52 } else { 36 }),
                ),
                StrokeKind::Inside,
            );
        }

        painter.rect_filled(
            face.translate(vec2(3.0, 5.0 + 1.5 * e)),
            CornerRadius::same(8),
            self.look.shadow.gamma_multiply(0.40 + 0.18 * e),
        );

        let peek = e * (PAPER_PEEK - 3.0);
        if peek > 1.0 {
            let sheet = Rect::from_min_max(
                pos2(face.min.x + 18.0, face.min.y - peek),
                pos2(face.max.x - 14.0, face.min.y + 8.0),
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

        // Centre du cartonnage (le titre est sous le dos).
        let cover = Rect::from_min_max(
            pos2(face.min.x + 13.0, face.min.y + 12.0),
            pos2(face.max.x - 6.0, face.max.y - 12.0),
        );
        let mark = cover.center();
        let logo_r = Rect::from_center_size(mark, vec2(56.0, 56.0));
        let emoji = meta.emoji.trim();
        let painted = !emoji.is_empty() && self.paint_emoji(ui.ctx(), &painter, logo_r, emoji);
        if !painted {
            painter.text(
                mark,
                Align2::CENTER_CENTER,
                "+",
                self.look.serif(28.0),
                paper.gamma_multiply(0.55),
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

        let title_rect = Rect::from_min_size(
            pos2(face.min.x, face.max.y + 4.0),
            vec2(face.width(), TITLE_ROW),
        );
        let renaming = self.rename_id == Some(meta.id) && !self.shelf_trash;
        if renaming {
            let mut commit = false;
            let mut cancel = false;
            ui.scope_builder(UiBuilder::new().max_rect(title_rect), |ui| {
                ui.visuals_mut().override_text_color = Some(self.look.fg);
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.hovered.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.active.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.open.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
                ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
                ui.visuals_mut().widgets.active.bg_stroke = Stroke::NONE;
                ui.visuals_mut().widgets.open.bg_stroke = Stroke::NONE;
                let te = TextEdit::singleline(&mut self.rename_buf)
                    .font(self.look.serif(15.0))
                    .text_color(self.look.fg)
                    .desired_width(title_rect.width())
                    .frame(false)
                    .margin(Margin::ZERO)
                    .horizontal_align(Align::Center)
                    .id(id.with("rename"));
                let r = ui.add(te);
                let armed = id.with("rename-armed");
                let already = ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(armed).unwrap_or(false));
                if !already {
                    r.request_focus();
                    ui.ctx().data_mut(|d| d.insert_temp(armed, true));
                }
                if r.lost_focus() {
                    ui.ctx().data_mut(|d| d.remove::<bool>(armed));
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    } else {
                        commit = true;
                    }
                } else if r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    commit = true;
                }
            });
            if commit {
                let title = self.rename_buf.trim().to_string();
                if !title.is_empty() {
                    if let Some(mut n) = self.lib.load_note(meta.id) {
                        n.title = title;
                        n.touch();
                        self.lib.save_note(&n);
                    }
                }
                self.rename_id = None;
            } else if cancel {
                self.rename_id = None;
            }
        } else {
            let title: String = {
                let t = meta.title.as_str();
                if t.chars().count() > 16 {
                    format!("{}…", t.chars().take(14).collect::<String>())
                } else {
                    t.to_string()
                }
            };
            if selected {
                let (r, g, b) = if self.look.dark {
                    (255_u8, 255, 255)
                } else {
                    (self.look.ink.r(), self.look.ink.g(), self.look.ink.b())
                };
                let name = Rect::from_center_size(
                    title_rect.center(),
                    vec2((title_rect.width() - 8.0).max(48.0), 22.0),
                );
                painter.rect_filled(
                    name,
                    CornerRadius::same(7),
                    Color32::from_rgba_unmultiplied(r, g, b, if self.look.dark { 20 } else { 16 }),
                );
            }
            painter.text(
                title_rect.center(),
                Align2::CENTER_CENTER,
                title,
                self.look.serif(15.0),
                self.look.fg,
            );
            if !self.shelf_trash {
                let title_resp = ui.interact(title_rect, id.with("title"), Sense::click());
                if title_resp.clicked() {
                    self.rename_buf = meta.title.clone();
                    self.rename_id = Some(meta.id);
                }
                title_resp.on_hover_cursor(CursorIcon::Text);
            }
        }

        let mut act = None;
        let mut menu_act = None;
        resp.context_menu(|ui| {
            ui.set_min_width(140.0);
            if !self.shelf_trash {
                if ui.button("Open").clicked() {
                    menu_act = Some(DosAct::Open);
                    ui.close();
                }
                if ui.button("Rename").clicked() {
                    menu_act = Some(DosAct::Rename);
                    ui.close();
                }
                if ui.button("Icon…").clicked() {
                    menu_act = Some(DosAct::Emoji);
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    menu_act = Some(DosAct::Dup);
                    ui.close();
                }
                ui.separator();
                if ui
                    .button(if meta.pinned { "Unpin" } else { "Pin" })
                    .clicked()
                {
                    menu_act = Some(DosAct::Pin);
                    ui.close();
                }
                if ui
                    .button(RichText::new("Trash").color(Color32::from_rgb(0xe2, 0x4b, 0x4a)))
                    .clicked()
                {
                    menu_act = Some(DosAct::Del);
                    ui.close();
                }
            } else {
                if ui.button("Restore").clicked() {
                    menu_act = Some(DosAct::Restore);
                    ui.close();
                }
                if ui
                    .button(
                        RichText::new("Delete").color(Color32::from_rgb(0xe2, 0x4b, 0x4a)),
                    )
                    .clicked()
                {
                    menu_act = Some(DosAct::Del);
                    ui.close();
                }
            }
        });
        if menu_act.is_some() {
            act = menu_act;
        }

        if act.is_none() && !renaming && resp.long_touched() {
            act = Some(DosAct::Select);
        }
        if act.is_none() && !renaming {
            let on_title = pointer.is_some_and(|p| title_rect.contains(p));
            let mods = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            if resp.double_clicked() && !on_title && !self.shelf_trash {
                act = Some(DosAct::Open);
            } else if resp.clicked()
                && !on_title
                && !self.shelf_band_armed
                && !self.shelf_haul_armed
            {
                act = Some(if mods.0 {
                    DosAct::Toggle
                } else if mods.1 {
                    DosAct::Range
                } else {
                    DosAct::Select
                });
            }
        }
        if !renaming {
            resp.clone().on_hover_cursor(CursorIcon::PointingHand);
        }
        act
    }

    fn objet_btn(&self, ui: &mut Ui, label: &str, accent: bool) -> bool {
        let galley = ui.painter().layout_no_wrap(
            label.to_string(),
            self.look.mono(13.0),
            if accent {
                self.look.desk_deep
            } else {
                self.look.fg
            },
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
            if accent {
                self.look.desk_deep
            } else {
                self.look.fg
            },
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
                ctx.data_mut(|d| d.insert_temp(Id::new("top-rect"), bar));
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
                            ui.painter()
                                .rect_filled(r, CornerRadius::same(8), self.look.desk_deep);
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
                self.finish_text_edit();
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
                        "stroke · erases the whole stroke"
                    } else {
                        "stroke · erases the whole stroke  ·  e"
                    }
                }
                Tool::EraserArea => {
                    if self.is_tablette() {
                        "area · erases under finger or stylus"
                    } else {
                        "area · erases under the cursor  ·  shift+e"
                    }
                }
                _ => kind.label(),
            };
            if self
                .paint_tool_well(ui, kind, on)
                .on_hover_text(tip)
                .clicked()
            {
                self.finish_text_edit();
                self.pick_eraser(kind);
            }
        }
        let lasso_resp = self
            .paint_tool_well(ui, Tool::Lasso, self.tool == Tool::Lasso || pen.lasso_btn)
            .on_hover_text(self.tool_hover(Tool::Lasso));
        if lasso_resp.clicked() {
            self.finish_text_edit();
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
            for (i, c) in self.look.highs.iter().enumerate() {
                let swatch = Color32::from_rgb(c.r(), c.g(), c.b());
                if self.color_dot(ui, swatch, i == self.color_i % n.max(1), vertical) {
                    self.color_i = i;
                }
            }
        } else {
            let n = self.look.inks.len().saturating_sub(1).max(1);
            for i in 0..n {
                let c = self.look.inks[i];
                if self.color_dot(ui, c, self.color_i % n == i, vertical) {
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
            self.finish_text_edit();
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
        resp.on_hover_cursor(cursor).on_hover_text("Move toolbar")
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
        let r = if on { 9.4 } else { 8.0 };
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
        self.paint_tool_well(ui, tool, self.tool == tool)
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
        let n_ink = self.look.inks.len().max(1);
        let n_wash = self.look.highs.len().max(1);
        let ink = self
            .look
            .inks
            .get(self.color_i % n_ink)
            .copied()
            .unwrap_or(fg);
        let wash = self
            .look
            .highs
            .get(self.color_i % n_wash)
            .copied()
            .unwrap_or(ink);
        paint_tool(p, tool, c, fg, cut, ink, wash);
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
                let n = self
                    .note
                    .as_ref()
                    .map(|n| n.pages.len())
                    .unwrap_or(1)
                    .max(1);
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
        let focus = pinch_center.or(resp.hover_pos()).unwrap_or(rect.center());

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
        let (screen, primary_down, primary_pressed, primary_released, secondary) = if pen_ink {
            if let Some(screen) = pen.pos {
                extra = pen.samples;
                (screen, pen.down, pen.pressed, pen.released, pen.eraser)
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
                .map(|r| r.expand(6.0).contains(screen))
                .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("top-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
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

        if tool.is_ink()
            && (primary_down || primary_pressed)
            && !resp.dragged_by(PointerButton::Middle)
        {
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
                        live.push(InkPoint::new(
                            loc,
                            mixed_pressure(live.nib, pressure, 200.0),
                        ));
                    }
                }
            }
        }
        if self.live.is_some()
            && (primary_released || !primary_down)
            && !(tool.is_ink() && primary_down)
        {
            self.finish_live(shift, time);
        }

        if tool.is_eraser() && (primary_down || secondary) {
            if primary_pressed
                || ui.input(|i| i.pointer.secondary_pressed())
                || (pen_ink && pen.pressed && secondary)
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
                } else if self
                    .lasso
                    .last()
                    .map(|p| p.distance(paper) > 2.0)
                    .unwrap_or(true)
                {
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
                if self.editing_text != Some((pg, id)) {
                    self.finish_text_edit();
                    self.editing_text = Some((pg, id));
                    self.text_focus = true;
                }
            } else {
                self.finish_text_edit();
                self.push_snapshot();
                let mut tx = TextBox::new(local, self.ink_color());
                tx.size = [420.0, 36.0];
                let id = tx.id;
                if let Some(n) = &mut self.note {
                    if let Some(pg) = n.pages.get_mut(page) {
                        pg.texts.push(tx);
                    }
                }
                self.editing_text = Some((page, id));
                self.text_focus = true;
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
                    .map(|(a, b)| {
                        egui::Rect::from_min_max(a + o, b + o)
                            .expand(12.0)
                            .contains(paper)
                    })
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
        pg.texts
            .iter()
            .rev()
            .find(|t| t.contains(local))
            .map(|t| (page, t.id))
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
                                .write_to(
                                    &mut std::io::Cursor::new(&mut png),
                                    image::ImageFormat::Png,
                                )
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
            painter.rect_filled(paper.translate(vec2(3.0, 4.0)), sheet_r, self.look.shadow);
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
                    painter.image(
                        tex.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
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
                    if tx.text.is_empty() {
                        let pos = map(tx.min() + origin);
                        let h = tx.size_pt * cam.zoom;
                        painter.line_segment(
                            [pos + vec2(0.6, 1.0), pos + vec2(0.6, h)],
                            Stroke::new(1.7_f32, tx.color32()),
                        );
                    }
                    continue;
                }
                let pos = map(tx.min() + origin);
                let max_w = (tx.size[0] * cam.zoom).max(8.0);
                let font = FontId::new(tx.size_pt * cam.zoom, FontFamily::Name("serif".into()));
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
                        SelKind::Stroke => page
                            .strokes
                            .iter()
                            .find(|x| x.id == s.id)
                            .and_then(|st| st.bbox()),
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
                        [
                            pos2(paper.min.x + 56.0 * z, y),
                            pos2(paper.max.x - 24.0 * z, y),
                        ],
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
                        let red = self.look.inks.get(2).copied().unwrap_or(self.look.ink);
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
                        [
                            pos2(x, paper.min.y + 24.0 * z),
                            pos2(x, paper.max.y - 24.0 * z),
                        ],
                        Stroke::new(0.8, self.look.paper_rule),
                    );
                    x += 24.0 * z;
                }
                let mut y = paper.min.y + 24.0 * z;
                while y < paper.max.y {
                    painter.line_segment(
                        [
                            pos2(paper.min.x + 24.0 * z, y),
                            pos2(paper.max.x - 24.0 * z, y),
                        ],
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
                        painter.circle_filled(
                            pos2(x, y),
                            1.1 * z.max(0.6),
                            self.look.paper_rule_strong,
                        );
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
                        [
                            pos2(paper.min.x + 48.0 * z, y),
                            pos2(paper.max.x - 20.0 * z, y),
                        ],
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
                        [
                            pos2(x, paper.min.y + 40.0 * z),
                            pos2(x, paper.max.y - 20.0 * z),
                        ],
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
        if self.cursor_off {
            ui.ctx().set_cursor_icon(CursorIcon::None);
            return;
        }
        if !resp.hovered() {
            return;
        }
        let space = ui.input(|i| i.key_down(Key::Space) || i.pointer.middle_down());
        if space {
            ui.ctx()
                .set_cursor_icon(if ui.input(|i| i.pointer.any_down()) {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
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

    fn pen_over_chrome(ctx: &Context, pos: Pos2) -> bool {
        ctx.data(|d| {
            d.get_temp::<Rect>(Id::new("dock-rect"))
                .map(|r| r.expand(8.0).contains(pos))
                .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("top-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
        })
    }

    /// Stylet → pointeur egui : hover + clics sur la chrome (trousse, règle, étagère).
    fn inject_pen_pointer(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let pen = self.tablet.snapshot();
        if pen.in_proximity || pen.down {
            if let Some(pos) = pen.pos {
                raw.events.push(Event::PointerMoved(pos));
                // Sur l’étagère : tout clic. Sur le pupitre : chrome seulement
                // (la feuille reste gérée par le pont tablette).
                let ui_click = match self.scene {
                    Scene::Shelf { .. } => true,
                    Scene::Desk => Self::pen_over_chrome(ctx, pos),
                };
                if ui_click {
                    let mods = raw.modifiers;
                    if pen.pressed {
                        raw.events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: mods,
                        });
                    }
                    if pen.released {
                        raw.events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers: mods,
                        });
                    }
                }
            }
            self.pen_was_prox = true;
        } else if self.pen_was_prox {
            raw.events.push(Event::PointerGone);
            self.pen_was_prox = false;
        }
    }

    fn sync_pen_cursor(&mut self, ctx: &Context) {
        let pen = self.tablet.snapshot();
        let hide = pen.in_proximity || pen.down;
        if hide {
            ctx.set_cursor_icon(CursorIcon::None);
            if !self.cursor_off {
                ctx.send_viewport_cmd(ViewportCommand::CursorVisible(false));
                self.cursor_off = true;
            }
        } else if self.cursor_off {
            ctx.send_viewport_cmd(ViewportCommand::CursorVisible(true));
            self.cursor_off = false;
        }
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
            root: self
                .lib
                .media_path(note.id, "")
                .parent()
                .unwrap_or(&self.lib.root)
                .to_path_buf(),
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
    if s.is_empty() {
        "note".into()
    } else {
        s
    }
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

/// Lucide trash-2 (ISC) — `assets/trash.svg`, teinté.
fn raster_lucide_trash(color: Color32, px: u32) -> ColorImage {
    use tiny_skia::{LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke as SkStroke, Transform};
    let mut pm = Pixmap::new(px, px).expect("trash pixmap");
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r(), color.g(), color.b(), 255);
    paint.anti_alias = true;
    let stroke = SkStroke {
        width: 2.0,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..SkStroke::default()
    };
    let t = Transform::from_scale(px as f32 / 24.0, px as f32 / 24.0);
    let stroke_line = |pm: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32| {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, &paint, &stroke, t, None);
        }
    };
    stroke_line(&mut pm, 3.0, 6.0, 21.0, 6.0);
    let mut body = PathBuilder::new();
    body.move_to(19.0, 6.0);
    body.line_to(19.0, 20.0);
    body.cubic_to(19.0, 21.0, 18.0, 22.0, 17.0, 22.0);
    body.line_to(7.0, 22.0);
    body.cubic_to(6.0, 22.0, 5.0, 21.0, 5.0, 20.0);
    body.line_to(5.0, 6.0);
    if let Some(path) = body.finish() {
        pm.stroke_path(&path, &paint, &stroke, t, None);
    }
    let mut lid = PathBuilder::new();
    lid.move_to(8.0, 6.0);
    lid.line_to(8.0, 4.0);
    lid.cubic_to(8.0, 3.0, 9.0, 2.0, 10.0, 2.0);
    lid.line_to(14.0, 2.0);
    lid.cubic_to(15.0, 2.0, 16.0, 3.0, 16.0, 4.0);
    lid.line_to(16.0, 6.0);
    if let Some(path) = lid.finish() {
        pm.stroke_path(&path, &paint, &stroke, t, None);
    }
    stroke_line(&mut pm, 10.0, 11.0, 10.0, 17.0);
    stroke_line(&mut pm, 14.0, 11.0, 14.0, 17.0);
    ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data())
}

fn mix_col(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
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
        pts.push(pos2(c.x + flip * a.cos() * 8.1, c.y + a.sin() * 8.1 + 1.1));
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

fn paint_tool(
    p: &egui::Painter,
    tool: Tool,
    c: Pos2,
    fg: Color32,
    _cut: Color32,
    _ink: Color32,
    _wash: Color32,
) {
    let tilt = -0.82_f32;
    let wire = Stroke::new(1.55_f32, fg);
    let hair = Stroke::new(1.35_f32, fg);
    match tool {
        Tool::Fineliner => {
            stroke_round(p, c, tilt, -8.4, 3.6, 1.45, 1.45, wire);
            p.line_segment([at(c, tilt, 3.35, 1.2), at(c, tilt, 11.0, 0.0)], wire);
            p.line_segment([at(c, tilt, 3.35, -1.2), at(c, tilt, 11.0, 0.0)], wire);
            p.line_segment([at(c, tilt, -6.5, 2.35), at(c, tilt, -0.7, 2.35)], hair);
        }
        Tool::Brush => {
            stroke_round(p, c, tilt, -8.6, 1.4, 1.35, 1.35, wire);
            p.add(egui::Shape::closed_line(
                leaf_pts(c, tilt, 10.9, 1.15, 2.05),
                wire,
            ));
            p.circle_stroke(at(c, tilt, 4.15, 0.0), 1.05, hair);
            p.line_segment([at(c, tilt, 4.15, 0.0), at(c, tilt, 8.35, 0.0)], hair);
        }
        Tool::Pencil => {
            stroke_round(p, c, tilt, -9.5, -7.15, 1.4, 1.4, wire);
            stroke_round(p, c, tilt, -7.35, -4.15, 1.72, 0.45, wire);
            p.line_segment([at(c, tilt, -6.7, -1.15), at(c, tilt, -6.7, 1.15)], hair);
            p.line_segment([at(c, tilt, -5.55, -1.15), at(c, tilt, -5.55, 1.15)], hair);
            stroke_round(p, c, tilt, -4.35, 4.15, 1.55, 0.4, wire);
            p.line_segment([at(c, tilt, 4.15, 1.55), at(c, tilt, 10.85, 0.0)], wire);
            p.line_segment([at(c, tilt, 4.15, -1.55), at(c, tilt, 10.85, 0.0)], wire);
            p.line_segment([at(c, tilt, 7.7, -0.72), at(c, tilt, 7.7, 0.72)], hair);
        }
        Tool::Highlighter => {
            stroke_round(p, c, tilt, -8.5, -0.7, 2.75, 1.5, wire);
            p.add(egui::Shape::closed_line(
                vec![
                    at(c, tilt, -0.15, 2.45),
                    at(c, tilt, 8.55, 3.7),
                    at(c, tilt, 6.7, -3.7),
                    at(c, tilt, -0.15, -2.45),
                ],
                wire,
            ));
        }
        Tool::EraserStroke => {
            let ang = -0.32_f32;
            stroke_round(p, c, ang, -7.4, 7.4, 3.15, 1.7, wire);
            p.line_segment([at(c, ang, -0.85, -2.15), at(c, ang, -0.85, 2.15)], hair);
        }
        Tool::EraserArea => {
            let ang = -0.42_f32;
            stroke_round(p, c, ang, -4.15, 4.15, 2.35, 1.15, wire);
            p.line_segment([at(c, ang, -0.55, -1.35), at(c, ang, -0.55, 1.35)], hair);
            paint_brackets(p, c, 7.7, 2.45, fg);
        }
        Tool::Lasso => {
            let rot = -0.35_f32;
            let (rs, rc) = rot.sin_cos();
            let n = 32;
            let span = std::f32::consts::TAU * 0.84;
            let start = 0.7_f32;
            let mut pts = Vec::with_capacity(n + 3);
            for i in 0..=n {
                let t = start + span * (i as f32 / n as f32);
                let breathe = 6.2_f32;
                let x = t.cos() * breathe;
                let y = t.sin() * breathe * 0.7;
                pts.push(pos2(c.x + x * rc - y * rs, c.y + x * rs + y * rc));
            }
            if pts.len() >= 2 {
                let end = pts[pts.len() - 1];
                let prev = pts[pts.len() - 2];
                let dir = (end - prev).normalized();
                let nrm = vec2(-dir.y, dir.x);
                pts.push(end + dir * 2.2 + nrm * 1.0);
                pts.push(end + dir * 3.8 + nrm * 2.4);
            }
            p.add(egui::Shape::line(pts, wire));
        }
        Tool::Text => {
            let o = c + vec2(0.0, 0.4);
            let y = o.y - 6.3;
            p.line_segment([pos2(o.x - 7.1, y), pos2(o.x + 7.1, y)], wire);
            p.line_segment([pos2(o.x - 7.1, y), pos2(o.x - 7.1, y + 2.5)], wire);
            p.line_segment([pos2(o.x + 7.1, y), pos2(o.x + 7.1, y + 2.5)], wire);
            p.line_segment([pos2(o.x, y), pos2(o.x, o.y + 6.5)], wire);
            p.line_segment(
                [pos2(o.x - 3.3, o.y + 6.5), pos2(o.x + 3.3, o.y + 6.5)],
                wire,
            );
        }
        Tool::Image => {
            let fr = Rect::from_center_size(c + vec2(0.1, 0.15), vec2(15.0, 12.0));
            p.rect_stroke(fr, 2.4, wire, StrokeKind::Inside);
            let x1 = fr.right() - 1.9;
            let y0 = fr.top() + 1.7;
            p.line_segment([pos2(x1 - 3.0, y0), pos2(x1, y0 + 3.0)], hair);
            p.circle_stroke(pos2(fr.left() + 4.3, fr.top() + 4.05), 1.45, hair);
            let base = fr.bottom() - 2.7;
            p.line_segment(
                [pos2(fr.left() + 2.5, base), pos2(c.x - 1.5, c.y - 0.2)],
                hair,
            );
            p.line_segment([pos2(c.x - 1.5, c.y - 0.2), pos2(c.x + 0.4, base)], hair);
            p.line_segment([pos2(c.x - 0.5, base), pos2(c.x + 2.7, c.y + 1.45)], hair);
            p.line_segment(
                [pos2(c.x + 2.7, c.y + 1.45), pos2(fr.right() - 2.3, base)],
                hair,
            );
        }
    }
}

fn at(c: Pos2, ang: f32, along: f32, side: f32) -> Pos2 {
    let (s, co) = ang.sin_cos();
    pos2(c.x + co * along - s * side, c.y + s * along + co * side)
}

fn round_pts(c: Pos2, ang: f32, a0: f32, a1: f32, half: f32, rad: f32) -> Vec<Pos2> {
    let rad = rad.min((a1 - a0) * 0.48).min(half * 0.98).max(0.2);
    let mut pts = Vec::with_capacity(20);
    let n = 3;
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
        for i in 0..n {
            let t = i as f32 / n as f32;
            let a = a_from + (a_to - a_from) * t;
            pts.push(at(c, ang, cx + a.cos() * rad, cy + a.sin() * rad));
        }
    }
    pts
}

fn stroke_round(
    p: &egui::Painter,
    c: Pos2,
    ang: f32,
    a0: f32,
    a1: f32,
    half: f32,
    rad: f32,
    stroke: Stroke,
) {
    p.add(egui::Shape::closed_line(
        round_pts(c, ang, a0, a1, half, rad),
        stroke,
    ));
}

fn leaf_pts(c: Pos2, ang: f32, tip: f32, base: f32, half: f32) -> Vec<Pos2> {
    let n = 6;
    let mut pts = Vec::with_capacity(n * 2);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let w = half * (1.0 - (1.0 - t) * (1.0 - t));
        pts.push(at(c, ang, tip + (base - tip) * t, w));
    }
    for i in (1..=n).rev() {
        let t = i as f32 / n as f32;
        let w = half * (1.0 - (1.0 - t) * (1.0 - t));
        pts.push(at(c, ang, tip + (base - tip) * t, -w));
    }
    pts
}

fn paint_brackets(p: &egui::Painter, c: Pos2, reach: f32, arm: f32, fg: Color32) {
    let st = Stroke::new(1.55_f32, fg);
    for sx in [-1.0_f32, 1.0] {
        for sy in [-1.0_f32, 1.0] {
            let o = pos2(c.x + sx * reach, c.y + sy * reach);
            p.line_segment([o, pos2(o.x - sx * arm, o.y)], st);
            p.line_segment([o, pos2(o.x, o.y - sy * arm)], st);
        }
    }
}
