#![allow(float_literal_f32_fallback)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::*;
use egui::containers::scroll_area::{ScrollBarVisibility, ScrollSource};
use egui::epaint::Vertex;
use egui::style::ScrollAnimation;
use uuid::Uuid;

use crate::camera::{page_at, page_origin, Camera, ZOOM_STOPS};
use crate::document::{ImageObj, Note, PaperKind, SheetJoin, TextBox, PAGE_H, PAGE_W};
use crate::emoji::Atlas;
use crate::export::{self, MediaLoader};
use crate::ink::{
    default_width, draw_ants, erase_area, map_mesh, maybe_snap_shape, mixed_pressure, InkPoint,
    InkStroke, Nib, Tool,
};
use crate::library::{
    ensure_png, image_size, DockEdge, Library, SHELF_COLS, SHELF_ROWS, SHELF_SLOTS, TRASH_SLOTS,
};
use crate::look::Look;
use crate::pressure::Pressure;
use crate::seed;
use crate::tablet::{PenSnapshot, TabletBridge};
use crate::undo::UndoStack;

const DOS_W: f32 = 128.0;
const DOS_H: f32 = 156.0;
const DOS_PAD: f32 = 10.0;
const PAPER_PEEK: f32 = 18.0;
const TITLE_ROW: f32 = 44.0;
const SHELF_GAP: f32 = 8.0;

#[derive(Clone)]
enum Scene {
    Shelf { query: String },
    Desk,
}

#[derive(Clone, Copy)]
enum DosAct {
    Open,
    Select,
    Toggle,
    Range,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShelfDrop {
    Shelf(usize),
    Bin(usize),
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SheetEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WheelPick {
    Pin,
    Mark,
    Color,
    Trash,
    Cloth(u8),
}

struct SpineWheel {
    id: Uuid,
    origin: Pos2,
    t0: f64,
    colors: bool,
    hover: Option<WheelPick>,
    secondary: bool,
    color_hold_t0: Option<f64>,
}

#[derive(Clone, Copy)]
enum MoreAct {
    Save,
    Duplicate,
    Pin,
    Linked,
    Separate,
    Png,
    Pdf,
    Trash,
}

pub struct CahierApp {
    look: Look,
    lib: Library,
    scene: Scene,
    note: Option<Note>,
    tool: Tool,
    color_i: usize,
    /// Current ink (theme swatch or a free tint).
    ink: Color32,
    color_custom: bool,
    /// Watercolor tin (large palette) is open.
    tin_open: bool,
    tin_hue: f32,
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
    /// Still nib: anchor, since when, and the shape already shown.
    shape_anchor: Option<Pos2>,
    shape_still: Option<Instant>,
    shape_preview: Option<InkStroke>,
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
    /// Page to frame in writing view (top of the sheet).
    land_page: Option<usize>,
    title_buf: String,
    dock_edge: DockEdge,
    dock_float: Option<Pos2>,
    dock_grab: Vec2,
    dock_moved: bool,
    canvas_rect: Rect,
    /// Click held on a sheet ear: do not lay ink.
    tab_held: bool,
    tablet: TabletBridge,
    live_from_pen: bool,
    /// Mouse cursor hidden (stylus in proximity).
    cursor_off: bool,
    /// Stylus was in proximity on the previous frame (for PointerGone).
    pen_was_prox: bool,
    /// Emoji picker open for this note.
    emoji_pick: Option<Uuid>,
    /// Filter inside the type case.
    emoji_query: String,
    /// Folder title currently being edited.
    rename_id: Option<Uuid>,
    rename_buf: String,
    /// Title position, so the editor can sit outside the scroll.
    rename_rect: Option<Rect>,
    /// The text field was just placed: give it the keyboard.
    text_focus: bool,
    /// Ink palette unfolded (panel beside the dock; dock size stays fixed).
    palette_open: bool,
    /// Stylus click that began on the chrome (not on the sheet).
    pen_ui_grab: bool,
    /// Open the image picker outside the click (system portal).
    pending_image: Option<(usize, Pos2)>,
    /// Explorer-style selection on the shelf.
    shelf_sel: Vec<Uuid>,
    shelf_anchor: Option<Uuid>,
    /// Red bin row open at the bottom of the shelf.
    shelf_trash: bool,
    /// Spine faces, for the selection rectangle.
    shelf_slots: Vec<(Uuid, Rect)>,
    /// Origin of the rectangle (None = no band).
    shelf_band: Option<Pos2>,
    shelf_band_now: Option<Pos2>,
    shelf_band_add: bool,
    shelf_band_armed: bool,
    shelf_band_base: Vec<Uuid>,
    /// Dragging spines toward the basket.
    shelf_haul: Option<Pos2>,
    shelf_haul_now: Option<Pos2>,
    shelf_haul_ids: Vec<Uuid>,
    shelf_haul_armed: bool,
    /// Grid cell while dragging.
    shelf_drop: Option<ShelfDrop>,
    /// Every shelf cell (occupied or empty).
    shelf_grid: Vec<Rect>,
    /// Bin row cells when the red zone is open.
    trash_grid: Vec<Rect>,
    shelf_toss: Vec<Toss>,
    trash_rect: Rect,
    trash_mouth: Pos2,
    shelf_fed: bool,
    /// Long-press radial menu on a spine.
    spine_wheel: Option<SpineWheel>,
    shelf_hold_t0: Option<f64>,
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
        let ink0 = look.inks.first().copied().unwrap_or(look.ink);
        let mut app = Self {
            look,
            lib,
            scene: Scene::Shelf {
                query: String::new(),
            },
            note: None,
            tool: Tool::Fineliner,
            color_i: 0,
            ink: ink0,
            color_custom: false,
            tin_open: false,
            tin_hue: 0.0,
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
            shape_anchor: None,
            shape_still: None,
            shape_preview: None,
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
            tab_held: false,
            tablet: TabletBridge::new(),
            live_from_pen: false,
            cursor_off: false,
            pen_was_prox: false,
            emoji_pick: None,
            emoji_query: String::new(),
            rename_id: None,
            rename_buf: String::new(),
            rename_rect: None,
            text_focus: false,
            palette_open: false,
            pen_ui_grab: false,
            pending_image: None,
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
            shelf_drop: None,
            shelf_grid: Vec::new(),
            trash_grid: Vec::new(),
            shelf_toss: Vec::new(),
            trash_rect: Rect::NOTHING,
            trash_mouth: Pos2::ZERO,
            shelf_fed: false,
            spine_wheel: None,
            shelf_hold_t0: None,
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
            Color32::from_rgba_unmultiplied(self.ink.r(), self.ink.g(), self.ink.b(), 90)
        } else {
            Color32::from_rgb(self.ink.r(), self.ink.g(), self.ink.b())
        }
    }

    fn pick_theme_ink(&mut self, i: usize) {
        self.color_custom = false;
        self.color_i = i;
        if self.tool == Tool::Highlighter {
            let n = self.look.highs.len().max(1);
            let c = self.look.highs[i % n];
            self.ink = Color32::from_rgb(c.r(), c.g(), c.b());
        } else {
            let n = self.look.inks.len().saturating_sub(1).max(1);
            self.ink = self.look.inks[i % n];
        }
        self.tin_hue = rgb_to_hsv(self.ink).0;
    }

    fn pick_free_ink(&mut self, c: Color32) {
        self.color_custom = true;
        self.ink = Color32::from_rgb(c.r(), c.g(), c.b());
        let (h, s, _) = rgb_to_hsv(self.ink);
        if s > 0.04 {
            self.tin_hue = h;
        }
    }

    fn ph(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.page_h.max(1.0))
            .unwrap_or(PAGE_H)
    }

    fn pw(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.page_w.max(1.0))
            .unwrap_or(PAGE_W)
    }

    fn page_cells(&self) -> Vec<(i32, i32)> {
        self.note
            .as_ref()
            .map(|n| n.pages.iter().map(|p| (p.col, p.row)).collect())
            .unwrap_or_else(|| vec![(0, 0)])
    }

    fn gap(&self) -> f32 {
        self.note
            .as_ref()
            .map(|n| n.sheet_join.gap())
            .unwrap_or(0.0)
    }

    fn origin_of(&self, i: usize) -> Vec2 {
        let cells = self.page_cells();
        let (col, row) = cells.get(i).copied().unwrap_or((0, 0));
        page_origin(col, row, self.pw(), self.ph(), self.gap())
    }

    fn page_at_paper(&self, paper: Pos2) -> usize {
        page_at(paper, &self.page_cells(), self.pw(), self.ph(), self.gap())
    }

    fn page_in_view(&self, rect: Rect) -> usize {
        self.page_at_paper(self.camera.to_paper(rect.center(), rect))
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

    /// 100 = the sheet fills the screen.
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
            self.clear_shape_hold();
            self.lasso.clear();
            self.editing_text = None;
            self.need_fit = false;
            self.land_page = Some(0);
            self.textures.clear();
            self.tab_held = false;
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

    fn save_now(&mut self, ctx: &Context) {
        if let Some(n) = &self.note {
            self.lib.save_note(n);
            self.dirty = false;
            let t = ctx.input(|i| i.time);
            self.toast("Saved", t);
        }
    }

    fn duplicate_open_note(&mut self) {
        let Some(n) = self.note.clone() else {
            return;
        };
        self.lib.save_note(&n);
        self.dirty = false;
        if let Some(copy) = self.lib.duplicate_note(&n) {
            self.open_note(copy.id);
        }
    }

    fn toggle_pin_open(&mut self) {
        let Some(n) = self.note.as_mut() else {
            return;
        };
        n.pinned = !n.pinned;
        let pinned = n.pinned;
        let id = n.id;
        self.mark_dirty();
        if pinned {
            self.lib.bring_front(id);
        }
    }

    fn set_sheet_join(&mut self, join: SheetJoin) {
        if self
            .note
            .as_ref()
            .is_none_or(|n| n.sheet_join == join)
        {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.push_snapshot();
        let rect = self.canvas_rect;
        let focus = rect.center();
        let old_paper = if rect.width() > 10.0 {
            self.camera.to_paper(focus, rect)
        } else {
            pos2(0.0, 0.0)
        };
        let (cells, old_gap) = self
            .note
            .as_ref()
            .map(|n| (n.unit_cells(), n.sheet_join.gap()))
            .unwrap_or_else(|| (vec![(0, 0)], 0.0));
        let idx = page_at(old_paper, &cells, PAGE_W, PAGE_H, old_gap);
        let (col, row) = cells.get(idx).copied().unwrap_or((0, 0));
        let local = old_paper - page_origin(col, row, PAGE_W, PAGE_H, old_gap);
        if let Some(n) = &mut self.note {
            n.apply_sheet_join(join);
        }
        if rect.width() > 10.0 {
            let o = page_origin(col, row, PAGE_W, PAGE_H, join.gap());
            let new_world = pos2(o.x + local.x, o.y + local.y);
            let z = self.camera.zoom;
            self.camera.pan = (focus - rect.min) - vec2(new_world.x * z, new_world.y * z);
        }
        self.sel.clear();
        self.lasso.clear();
        self.mark_dirty();
    }

    fn trash_open_note(&mut self) {
        self.autosave();
        if let Some(n) = self.note.take() {
            self.lib.save_note(&n);
            self.lib.trash_note(n.id);
        }
        self.scene = Scene::Shelf {
            query: String::new(),
        };
        self.undo.clear();
        self.sel.clear();
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

    /// Picks an eraser; tapping the same one again returns to ink.
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

    fn is_tablette(&self) -> bool {
        self.palm_guard(&self.tablet.snapshot())
    }

    fn slot(&self) -> f32 {
        44.0
    }

    fn chrome_btn(&self) -> f32 {
        44.0
    }

    fn drop_spot(&self) -> (usize, Pos2) {
        let rect = self.canvas_rect;
        let paper = if rect.width() > 10.0 {
            self.camera.to_paper(rect.center(), rect)
        } else {
            pos2(80.0, 80.0)
        };
        let page = self.page_at_paper(paper);
        let local = paper - self.origin_of(page);
        (page, local)
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
                if !self.color_custom {
                    self.pick_theme_ink(self.color_i);
                }
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
        let mut width_delta: f32 = 0.0;
        let mut tool: Option<Tool> = None;
        let mut color: Option<usize> = None;
        let mut cycle_paper = false;
        let mut erase_pick: Option<Tool> = None;
        let mut toggle_fiche = false;

        let typing = ctx.wants_keyboard_input()
            || self.editing_text.is_some()
            || self.rename_id.is_some()
            || self.emoji_pick.is_some();
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
                    if !typing && i.key_pressed(Key::N) {
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
                self.save_now(ctx);
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
                if t == Tool::Image {
                    self.pending_image = Some(self.drop_spot());
                }
            }
            if let Some(i) = color {
                self.pick_theme_ink(i);
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
        // Shelf shortcuts (selection / trash)
        if self.emoji_pick.is_none() && self.rename_id.is_none() {
            self.shelf_keys(ctx);
            self.spine_wheel_tick(ctx);
            self.shelf_pointer_tick(ctx);
            self.shelf_toss_tick(ctx);
            if self.shelf_haul_armed
                || !self.shelf_toss.is_empty()
                || self.spine_wheel.is_some()
                || self.shelf_hold_t0.is_some()
            {
                ctx.request_repaint();
            }
        }

        let query = match &self.scene {
            Scene::Shelf { query } => query.to_lowercase(),
            _ => String::new(),
        };
        let querying = !query.is_empty();
        let mut open = None;
        let mut select = None;
        let mut toggle = None;
        let mut range = None;
        self.shelf_slots.clear();
        self.shelf_grid.clear();
        self.trash_grid.clear();

        if self.bin_open() {
            self.ui_bin_zone(ctx, &mut open, &mut select, &mut toggle, &mut range);
        }

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                let notes: Vec<_> = self
                    .lib
                    .index
                    .notes
                    .iter()
                    .filter(|m| query.is_empty() || m.title.to_lowercase().contains(&query))
                    .filter(|m| !self.shelf_toss.iter().any(|t| t.id == m.id))
                    .cloned()
                    .collect();

                ui.add_space(92.0);

                let shifting = self.shelf_haul_armed && !querying;
                let packed = querying;

                ScrollArea::vertical()
                    .id_salt("shelf-grid")
                    .scroll_source(ScrollSource {
                        drag: false,
                        scroll_bar: true,
                        mouse_wheel: true,
                    })
                    .show(ui, |ui| {
                        if let Some(mt) = ui.input(|i| i.multi_touch()) {
                            if mt.num_touches >= 2 && mt.translation_delta != Vec2::ZERO {
                                ui.scroll_with_delta_animation(
                                    mt.translation_delta,
                                    ScrollAnimation::none(),
                                );
                                ui.ctx().request_repaint();
                            }
                        }
                        ui.add_space(4.0);
                        let left = 28.0;
                        let slot = Self::shelf_slot();
                        let pitch = vec2(slot.x + SHELF_GAP, slot.y + SHELF_GAP);
                        let (cols, rows) = if packed {
                            let available = ui.available_width() - left;
                            let cols =
                                ((available + SHELF_GAP) / pitch.x).floor().max(1.0) as usize;
                            let rows = (notes.len().max(1) + cols - 1) / cols;
                            (cols, rows.max(1))
                        } else {
                            (SHELF_COLS as usize, SHELF_ROWS as usize)
                        };
                        let n_cells = if packed {
                            rows * cols
                        } else {
                            SHELF_SLOTS as usize
                        };
                        let grid = vec2(left + cols as f32 * pitch.x, rows as f32 * pitch.y);
                        let (full, _) = ui.allocate_exact_size(grid, Sense::hover());
                        let origin = pos2(full.min.x + left, full.min.y);
                        let occ: HashMap<u32, crate::library::NoteMeta> = if packed {
                            HashMap::new()
                        } else {
                            notes.iter().map(|m| (m.slot, m.clone())).collect()
                        };
                        for idx in 0..n_cells {
                            let col = idx % cols;
                            let row = idx / cols;
                            let cell = Rect::from_min_size(
                                origin + vec2(col as f32 * pitch.x, row as f32 * pitch.y),
                                slot,
                            );
                            self.shelf_grid.push(cell);
                            let meta = if packed {
                                notes.get(idx).cloned()
                            } else {
                                occ.get(&(idx as u32)).cloned()
                            };
                            let lifted = meta
                                .as_ref()
                                .map(|m| shifting && self.shelf_haul_ids.contains(&m.id))
                                .unwrap_or(false);
                            let hot = shifting && self.shelf_drop == Some(ShelfDrop::Shelf(idx));
                            if let Some(meta) = meta.filter(|_| !lifted) {
                                ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                                    let selected = self.shelf_sel.contains(&meta.id);
                                    match self.cahier_dos(ui, &meta, selected, false) {
                                        Some(DosAct::Open) => open = Some(meta.id),
                                        Some(DosAct::Select) => select = Some(meta.id),
                                        Some(DosAct::Toggle) => toggle = Some(meta.id),
                                        Some(DosAct::Range) => range = Some(meta.id),
                                        None => {}
                                    }
                                });
                            } else if shifting {
                                self.place_dos(ui, cell, hot);
                            }
                        }
                    });
            });

        let ids: Vec<Uuid> = {
            let mut v = self.lib.index.notes.clone();
            v.sort_by_key(|n| n.slot);
            let mut ids: Vec<_> = v.into_iter().map(|n| n.id).collect();
            if self.shelf_trash {
                let mut t = self.lib.index.trash.clone();
                t.sort_by_key(|n| n.slot);
                ids.extend(t.into_iter().map(|n| n.id));
            }
            ids
        };
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
        if let Some(id) = open {
            if !self.lib.is_trashed(id) && !self.shelf_band_armed {
                self.emoji_pick = None;
                self.rename_id = None;
                self.shelf_sel.clear();
                self.open_note(id);
            }
        }

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

        // Wastebasket — drop corner of the lectern
        Area::new(Id::new("fab-corbeille"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-18.0, -16.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    let n = self.lib.index.trash.len() + self.shelf_toss.len();
                    if self.shelf_trash && n > 0 {
                        if self
                            .empty_bin_pull(ui)
                            .on_hover_text("Empty the bin")
                            .clicked()
                        {
                            self.lib.empty_trash();
                            self.shelf_sel.clear();
                            self.shelf_fed = true;
                        }
                    }
                    let hungry = self.shelf_haul_armed
                        && self
                            .shelf_haul_now
                            .is_some_and(|p| self.trash_rect.expand(18.0).contains(p));
                    let resp = self
                        .wastebasket(ui, self.bin_open(), n, hungry)
                        .on_hover_text("Trash");
                    if resp.clicked() && !self.shelf_fed && !self.shelf_haul_armed {
                        self.shelf_trash = !self.shelf_trash;
                        self.shelf_sel.clear();
                        self.shelf_anchor = None;
                        if let Scene::Shelf { query } = &mut self.scene {
                            query.clear();
                        }
                    }
                });
            });

        Area::new(Id::new("shelf-search"))
            .anchor(Align2::CENTER_TOP, vec2(0.0, 16.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                if let Scene::Shelf { query } = &mut self.scene {
                    let rail = 220.0;
                    let search_w = (ctx.screen_rect().width() - rail * 2.0).clamp(200.0, 560.0);
                    Frame::NONE
                        .fill(self.look.desk_deep)
                        .corner_radius(22)
                        .inner_margin(Margin::symmetric(18, 11))
                        .show(ui, |ui| {
                            ui.set_width(search_w);
                            let te = TextEdit::singleline(query)
                                .hint_text("Search")
                                .font(self.look.mono(15.0))
                                .frame(false)
                                .id(Id::new("shelf-search-field"));
                            ui.add(te);
                        });
                }
            });

        let fiche_ouverte = !self.lib.index.fiche_pliee;
        Area::new(Id::new("shelf-top-right"))
            .anchor(Align2::RIGHT_TOP, vec2(-16.0, 10.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    if self
                        .help_signet(ui, fiche_ouverte)
                        .on_hover_text(if fiche_ouverte { "Fold" } else { "Help" })
                        .clicked()
                    {
                        self.lib.index.fiche_pliee = !self.lib.index.fiche_pliee;
                        self.lib.save_index();
                    }
                    if self.inkwell(ui).on_hover_text("New  ·  N").clicked() {
                        self.new_note();
                    }
                    if self
                        .quit_signet(ui)
                        .on_hover_text("Quit")
                        .clicked()
                    {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });

        if fiche_ouverte {
            Area::new(Id::new("shelf-tuto-fiche"))
                .anchor(Align2::RIGHT_TOP, vec2(-16.0, 74.0))
                .order(Order::Foreground)
                .show(ctx, |ui| {
                    self.fiche_pupitre(ui);
                });
        }
        self.paint_shelf_haul(ctx);
        self.paint_spine_wheel(ctx);
        self.shelf_rename_field(ctx);
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
        let typing =
            ctx.wants_keyboard_input() || self.rename_id.is_some() || self.emoji_pick.is_some();
        let mut clear = false;
        let mut trash_sel = false;
        let mut select_all = false;
        let mut open_one = false;
        let mut pin = false;
        let mut mark = false;
        let mut restore = false;
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
            if !typing && !i.modifiers.command && i.key_pressed(Key::P) {
                pin = true;
            }
            if !typing && !i.modifiers.command && i.key_pressed(Key::E) {
                mark = true;
            }
            if !typing && !i.modifiers.command && i.key_pressed(Key::R) {
                restore = true;
            }
        });
        if clear {
            if self.spine_wheel.is_some() {
                self.spine_wheel = None;
            } else if !self.shelf_sel.is_empty() {
                self.shelf_sel.clear();
                self.shelf_anchor = None;
            } else if self.shelf_trash {
                self.shelf_trash = false;
            }
        }
        if select_all {
            self.shelf_sel = self.lib.index.notes.iter().map(|m| m.id).collect();
        }
        let ids = self.shelf_action_ids(ctx);
        if trash_sel && !ids.is_empty() {
            for id in &ids {
                if self.lib.is_trashed(*id) {
                    self.lib.purge_trashed(*id);
                } else {
                    self.lib.trash_note(*id);
                }
            }
            self.shelf_sel.retain(|id| !ids.contains(id));
        }
        if pin && !ids.is_empty() {
            let pin_on = ids.iter().any(|id| {
                self.lib
                    .index
                    .notes
                    .iter()
                    .find(|m| m.id == *id)
                    .is_some_and(|m| !m.pinned)
            });
            for id in &ids {
                if self.lib.is_trashed(*id) {
                    continue;
                }
                if let Some(mut note) = self.lib.load_note(*id) {
                    note.pinned = pin_on;
                    self.lib.save_note(&note);
                    if pin_on {
                        self.lib.bring_front(*id);
                    }
                }
            }
        }
        if mark {
            if let Some(id) = ids.first().copied() {
                if !self.lib.is_trashed(id) {
                    self.emoji_pick = Some(id);
                    self.rename_id = None;
                }
            }
        }
        if restore {
            for id in &ids {
                if self.lib.is_trashed(*id) {
                    self.lib.restore_note(*id);
                }
            }
            self.shelf_sel.retain(|id| !ids.contains(id));
        }
        if open_one {
            if let Some(id) = self.shelf_sel.first().copied() {
                if !self.lib.is_trashed(id) {
                    self.shelf_sel.clear();
                    self.open_note(id);
                }
            }
        }
    }

    fn shelf_action_ids(&self, ctx: &Context) -> Vec<Uuid> {
        if !self.shelf_sel.is_empty() {
            return self.shelf_sel.clone();
        }
        let Some(pos) = ctx.pointer_hover_pos() else {
            return Vec::new();
        };
        self.shelf_slots
            .iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(id, _)| vec![*id])
            .unwrap_or_default()
    }

    fn shelf_pointer_tick(&mut self, ctx: &Context) {
        let typing = ctx.wants_keyboard_input();
        let (pressed, down, released, pos, add, sec_pressed, touching, now) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.command || i.modifiers.shift,
                i.pointer.button_pressed(PointerButton::Secondary),
                i.any_touches(),
                i.time,
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
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            }
            return;
        };
        let screen = ctx.screen_rect();
        let chrome = pos.y < screen.min.y + 76.0 || pos.x < screen.min.x + 12.0;
        let on_trash = self.trash_rect.expand(12.0).contains(pos);
        let querying = matches!(&self.scene, Scene::Shelf { query } if !query.trim().is_empty());
        let hit = self
            .shelf_slots
            .iter()
            .find(|(_, r)| r.contains(pos))
            .map(|(id, _)| *id);

        if ctx.input(|i| i.multi_touch().is_some_and(|mt| mt.num_touches >= 2)) {
            self.spine_wheel = None;
            self.shelf_haul = None;
            self.shelf_haul_now = None;
            self.shelf_haul_armed = false;
            self.shelf_haul_ids.clear();
            self.shelf_drop = None;
            self.shelf_band = None;
            self.shelf_band_now = None;
            self.shelf_band_armed = false;
            self.shelf_hold_t0 = None;
            return;
        }

        if self.spine_wheel.is_some() {
            if released {
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            }
            return;
        }

        if sec_pressed && !typing && !on_trash && !self.shelf_fed {
            if let Some(id) = hit {
                if !self.lib.is_trashed(id) {
                    self.open_spine_wheel(id, pos, ctx, true);
                    return;
                }
            }
        }

        if pressed && !typing && !on_trash && !self.shelf_fed {
            if let Some(id) = hit {
                if !add {
                    self.shelf_haul = Some(pos);
                    self.shelf_haul_now = Some(pos);
                    self.shelf_haul_armed = false;
                    self.shelf_drop = None;
                    // Hold is for a finger; the mouse opens the wheel with right-click.
                    if touching && !self.lib.is_trashed(id) {
                        self.shelf_hold_t0 = Some(now);
                    }
                    let from_bin = self.lib.is_trashed(id);
                    self.shelf_haul_ids = if self.shelf_sel.contains(&id) {
                        self.shelf_sel
                            .iter()
                            .copied()
                            .filter(|x| self.lib.is_trashed(*x) == from_bin)
                            .collect()
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
                    self.shelf_hold_t0 = None;
                    if let Some(id) = self.shelf_haul_ids.first().copied() {
                        if !self.shelf_sel.contains(&id) {
                            self.shelf_sel = self.shelf_haul_ids.clone();
                            self.shelf_anchor = Some(id);
                        }
                    }
                    if !querying {
                        self.shelf_drop = self.haul_drop(pos);
                    }
                    ctx.set_cursor_icon(if on_trash {
                        CursorIcon::Move
                    } else {
                        CursorIcon::Grabbing
                    });
                    ctx.request_repaint();
                } else if touching {
                    if let Some(t0) = self.shelf_hold_t0 {
                        if now - t0 >= WHEEL_HOLD {
                            if let Some(id) = self.shelf_haul_ids.first().copied() {
                                self.open_spine_wheel(id, origin, ctx, false);
                            }
                        }
                    }
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
            self.shelf_hold_t0 = None;
            if self.shelf_haul_armed {
                let over = self
                    .shelf_haul_now
                    .is_some_and(|p| self.trash_rect.expand(22.0).contains(p));
                let from_bin = self.haul_from_bin();
                if over && !from_bin {
                    self.begin_toss(ctx.input(|i| i.time));
                    self.shelf_fed = true;
                } else if !querying {
                    if let Some(drop) = self.shelf_drop {
                        let mut ids = self.shelf_haul_ids.clone();
                        ids.sort_by_key(|id| {
                            self.lib
                                .index
                                .notes
                                .iter()
                                .chain(self.lib.index.trash.iter())
                                .find(|n| n.id == *id)
                                .map(|n| n.slot)
                                .unwrap_or(u32::MAX)
                        });
                        match drop {
                            ShelfDrop::Shelf(cell) => {
                                if from_bin {
                                    self.lib.restore_at(&ids, cell as u32);
                                } else {
                                    self.lib.place_at(&ids, cell as u32);
                                }
                            }
                            ShelfDrop::Bin(cell) => {
                                if from_bin {
                                    self.lib.place_trash_at(&ids, cell as u32);
                                } else if self.shelf_trash {
                                    // Into bin cells only while the bin row is open.
                                    self.lib.trash_at(&ids, cell as u32);
                                }
                            }
                        }
                        self.shelf_fed = true;
                    }
                }
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_armed = false;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
            } else if self.shelf_haul.is_some() {
                self.shelf_haul = None;
                self.shelf_haul_now = None;
                self.shelf_haul_ids.clear();
                self.shelf_drop = None;
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

    fn haul_drop(&self, pos: Pos2) -> Option<ShelfDrop> {
        // Bin cells are drop targets only while the bin row is deliberately open.
        // With the bin closed, shelf → trash goes through the wastebasket logo.
        let bin_ok = self.shelf_trash;
        if bin_ok {
            if let Some(i) = self.trash_grid.iter().position(|r| r.contains(pos)) {
                return Some(ShelfDrop::Bin(i));
            }
        }
        if let Some(i) = self.shelf_grid.iter().position(|r| r.contains(pos)) {
            return Some(ShelfDrop::Shelf(i));
        }
        let bin_near = if bin_ok {
            self.trash_grid.iter().enumerate().min_by(|(_, a), (_, b)| {
                a.center()
                    .distance_sq(pos)
                    .partial_cmp(&b.center().distance_sq(pos))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        } else {
            None
        };
        let shelf_near = self.shelf_grid.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.center()
                .distance_sq(pos)
                .partial_cmp(&b.center().distance_sq(pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        match (bin_near, shelf_near) {
            (Some((i, br)), Some((_, sr))) => {
                if br.center().distance_sq(pos) <= sr.center().distance_sq(pos) {
                    Some(ShelfDrop::Bin(i))
                } else {
                    shelf_near.map(|(i, _)| ShelfDrop::Shelf(i))
                }
            }
            (Some((i, _)), None) => Some(ShelfDrop::Bin(i)),
            (None, Some((i, _))) => Some(ShelfDrop::Shelf(i)),
            (None, None) => None,
        }
    }

    fn begin_toss(&mut self, now: f64) {
        let from = self.shelf_haul_now.unwrap_or(self.trash_mouth);
        let ids = self.shelf_haul_ids.clone();
        for (i, id) in ids.iter().enumerate() {
            let meta = self.lib.index.notes.iter().find(|m| m.id == *id).cloned();
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
        let hit = 62.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(hit, hit), Sense::click());
        self.trash_rect = rect;
        let id = Id::new("waste-basket");
        let hover_t = ui.ctx().animate_bool_with_time(
            id.with("h"),
            resp.hovered() || hungry || open || count > 0,
            0.16,
        );
        let e = hover_t * hover_t * (3.0 - 2.0 * hover_t);
        let side = 52.0 + e * 3.0;
        let icon = Rect::from_center_size(rect.center() + vec2(0.0, 2.0 - e), vec2(side, side));
        self.trash_mouth = pos2(icon.center().x, icon.min.y + side * 0.20);
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        self.paint_emoji(&ctx, &p, icon, "🗑️");
        resp.on_hover_cursor(if hungry {
            CursorIcon::Move
        } else {
            CursorIcon::PointingHand
        })
    }

    fn paper_tex(&mut self, ctx: &Context) -> TextureHandle {
        let key = "canson-paper";
        if let Some(tex) = self.textures.get(key) {
            return tex.clone();
        }
        let bytes = include_bytes!("../assets/canson.jpg");
        let dynimg = image::load_from_memory(bytes).expect("canson");
        let rgba = dynimg.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let img = ColorImage::from_rgba_unmultiplied(size, &rgba);
        let tex = ctx.load_texture(
            key,
            img,
            TextureOptions {
                magnification: TextureFilter::Linear,
                minification: TextureFilter::Linear,
                wrap_mode: TextureWrapMode::Repeat,
                mipmap_mode: Some(TextureFilter::Linear),
            },
        );
        self.textures.insert(key.into(), tex.clone());
        tex
    }

    fn paint_paper_rect(&self, p: &Painter, rect: Rect, tex: &TextureHandle) {
        let uv = Rect::from_min_max(
            Pos2::ZERO,
            pos2(rect.width() / PAPER_TILE, rect.height() / PAPER_TILE),
        );
        p.image(tex.id(), rect, uv, Color32::WHITE);
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

    fn shelf_rename_field(&mut self, ctx: &Context) {
        let Some(id) = self.rename_id else {
            self.rename_rect = None;
            return;
        };
        let Some(rect) = self.rename_rect else {
            return;
        };
        let mut commit = false;
        let mut cancel = false;
        Area::new(Id::new("shelf-rename").with(id))
            .fixed_pos(rect.min)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
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
                    .font(self.look.serif(21.0))
                    .text_color(self.look.fg)
                    .desired_width(rect.width())
                    .frame(false)
                    .margin(Margin::ZERO)
                    .horizontal_align(Align::Center)
                    .vertical_align(Align::Center)
                    .id(Id::new("shelf-rename-edit").with(id));
                let r = ui.add_sized(rect.size(), te);
                let armed = Id::new("shelf-rename-armed").with(id);
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
                if let Some(mut n) = self.lib.load_note(id) {
                    n.title = title;
                    n.touch();
                    self.lib.save_note(&n);
                }
            }
            self.rename_id = None;
            self.rename_rect = None;
        } else if cancel {
            self.rename_id = None;
            self.rename_rect = None;
        }
    }

    fn paint_shelf_haul(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        let mut ghosts: Vec<(Pos2, f32, f32, u8, String)> = Vec::new();
        if self.shelf_haul_armed {
            if let Some(pos) = self.shelf_haul_now {
                for (i, id) in self.shelf_haul_ids.iter().enumerate() {
                    let meta = self
                        .lib
                        .index
                        .notes
                        .iter()
                        .chain(self.lib.index.trash.iter())
                        .find(|n| n.id == *id);
                    if let Some(m) = meta {
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

    /// Sits on the wastebasket while the bin row is open.
    fn empty_bin_pull(&self, ui: &mut Ui) -> Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 40.0), Sense::click());
        let fill = if resp.hovered() {
            shade_rgb(self.look.rust(), 1.12)
        } else {
            self.look.rust()
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(18), fill);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Empty",
            self.look.mono(15.0),
            well_glyph(fill, self.look.paper, self.look.ink),
        );
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    /// Same well as New, with a question mark in the center.
    fn help_signet(&self, ui: &mut Ui, open: bool) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let paper = mix_col(self.look.paper, self.look.accent, 0.04);
        let well = if resp.hovered() || open {
            mix_col(paper, self.look.accent, 0.08)
        } else {
            paper
        };
        p.circle_filled(c, 25.0, well);
        p.circle_stroke(
            c,
            25.0,
            Stroke::new(1.2_f32, self.look.ink.gamma_multiply(0.22)),
        );
        p.circle_filled(c, 16.5, mix_col(well, self.look.paper, 0.35));
        p.circle_stroke(
            c,
            16.5,
            Stroke::new(1.0_f32, self.look.ink.gamma_multiply(0.16)),
        );
        p.text(
            c + vec2(0.0, 1.0),
            Align2::CENTER_CENTER,
            "?",
            self.look.serif(22.0),
            self.look.ink,
        );
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn quit_signet(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let well = if resp.hovered() {
            self.look.desk_edge
        } else {
            self.look.desk_deep
        };
        self.paint_inkwell_body(p, c, well);
        let fg = well_glyph(well, self.look.paper, self.look.ink);
        let s = 6.5;
        p.line_segment(
            [c + vec2(-s, -s), c + vec2(s, s)],
            Stroke::new(1.8_f32, fg),
        );
        p.line_segment(
            [c + vec2(s, -s), c + vec2(-s, s)],
            Stroke::new(1.8_f32, fg),
        );
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn paint_inkwell_body(&self, p: &Painter, c: Pos2, well: Color32) {
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
    }

    /// Notebook sheet: the same object for the help card and the marks.
    fn paint_cahier_page(&self, p: &Painter, rect: Rect, paper: &TextureHandle) -> Rect {
        self.paint_cahier_leaf(p, rect, 20.0, 18.0, paper)
    }

    fn paint_cahier_leaf(
        &self,
        p: &Painter,
        rect: Rect,
        top: f32,
        bot: f32,
        paper: &TextureHandle,
    ) -> Rect {
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
        self.paint_paper_rect(p, rect, paper);
        p.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0_f32, ink.gamma_multiply(0.12)),
            StrokeKind::Inside,
        );
        Rect::from_min_max(
            pos2(rect.min.x + 22.0, rect.min.y + top),
            pos2(rect.max.x - 22.0, rect.max.y - bot),
        )
    }

    fn fiche_pupitre(&mut self, ui: &mut Ui) {
        let w = 560.0_f32
            .min(ui.ctx().screen_rect().width() - 40.0)
            .max(360.0);
        let h = 420.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let paper = self.paper_tex(ui.ctx());
        let p = ui.painter_at(rect);
        let inner = self.paint_cahier_page(&p, rect, &paper);
        let ink = self.look.ink;
        let mute = ink.gamma_multiply(0.46);
        p.text(
            inner.min,
            Align2::LEFT_TOP,
            "Help",
            self.look.serif(26.0),
            ink,
        );
        let y0 = inner.min.y + 40.0;
        let gap = 22.0;
        let mid = inner.center().x;
        let left = Rect::from_min_max(pos2(inner.min.x, y0), pos2(mid - gap * 0.5, inner.max.y));
        let right = Rect::from_min_max(pos2(mid + gap * 0.5, y0), inner.max);
        let keys: &[(&str, &str)] = &[
            ("New", "N"),
            ("Open", "double-click"),
            ("Select", "click"),
            ("Place", "drag"),
            ("Wheel", "right-click"),
            ("Pin", "P"),
            ("Mark", "E"),
            ("Trash", "Del"),
            ("Restore", "R"),
            ("Pan", "space"),
            ("Tear page", "corner"),
            ("Undo", "Ctrl+Z"),
            ("Save", "Ctrl+S"),
            ("Zoom", "+ −"),
        ];
        let hands: &[(&str, &str)] = &[
            ("Draw", "stylus"),
            ("Pan", "finger"),
            ("Scroll", "two fingers"),
            ("Wheel", "hold"),
            ("Undo", "two-finger tap"),
            ("Zoom", "pinch"),
            ("Eraser", "stylus button"),
            ("Lasso", "2nd button"),
            ("Shape", "hold still"),
            ("Tear page", "fold"),
        ];
        self.paint_help_col(&p, left, "keyboard · mouse", keys, ink, mute);
        self.paint_help_col(&p, right, "hands", hands, ink, mute);

        if resp.clicked() {
            self.lib.index.fiche_pliee = true;
            self.lib.save_index();
        }
        resp.on_hover_cursor(CursorIcon::PointingHand);
    }

    fn paint_help_col(
        &self,
        p: &Painter,
        col: Rect,
        head: &str,
        rows: &[(&str, &str)],
        ink: Color32,
        mute: Color32,
    ) {
        p.text(
            col.min,
            Align2::LEFT_TOP,
            head,
            self.look.mono(10.0),
            mute,
        );
        let mut y = col.min.y + 22.0;
        for (k, v) in rows {
            p.text(
                pos2(col.min.x, y),
                Align2::LEFT_TOP,
                *k,
                self.look.serif(15.0),
                ink,
            );
            p.text(
                pos2(col.max.x, y + 2.0),
                Align2::RIGHT_TOP,
                *v,
                self.look.mono(11.0),
                mute,
            );
            y += 24.0;
        }
    }

    /// Paints a color icon into `bounds`. False if the bitmap is missing.
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

    /// Paper sheet of spine marks: emojis only, scroll to unroll.
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
                let d = self.look.desk;
                ui.painter().rect_filled(
                    r,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(d.r(), d.g(), d.b(), 150),
                );
                if resp.clicked() {
                    dismiss = true;
                }
            });

        Area::new(Id::new("emoji-pick").with(note_id))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(Order::Foreground)
            .show(ctx, |ui| {
                let paper_col = mix_col(self.look.paper, self.look.accent, 0.028);
                let ink = self.look.ink;
                let (outer, _sheet_resp) =
                    ui.allocate_exact_size(vec2(sheet_w, sheet_h), Sense::hover());
                let paper_tex = self.paper_tex(ui.ctx());
                let p = ui.painter_at(outer);
                p.rect_filled(
                    outer.translate(vec2(4.0, 7.0)),
                    CornerRadius::same(3),
                    self.look.shadow.gamma_multiply(0.42),
                );
                self.paint_paper_rect(&p, outer, &paper_tex);
                p.rect_stroke(
                    outer,
                    CornerRadius::same(3),
                    Stroke::new(1.0_f32, ink.gamma_multiply(0.12)),
                    StrokeKind::Inside,
                );
                let pad = 12.0;
                let inner = outer.shrink(pad);
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    let catalog: Vec<char> = self.emoji.catalog().to_vec();
                    let gap = 4.0;
                    let avail = inner.width().max(40.0);
                    let cols = (avail / (40.0 + gap)).floor().max(1.0) as usize;
                    let cell =
                        (avail - gap * cols.saturating_sub(1) as f32) / cols as f32;
                    let grid_h = inner.height().max(160.0);
                    ScrollArea::vertical()
                        .max_height(grid_h)
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                        .scroll_source(ScrollSource {
                            drag: true,
                            scroll_bar: false,
                            mouse_wheel: true,
                        })
                        .show(ui, |ui| {
                            ui.set_width(avail);
                            ui.spacing_mut().item_spacing = vec2(gap, gap);
                            let mut i = 0;
                            while i < catalog.len() {
                                ui.horizontal(|ui| {
                                    ui.set_width(avail);
                                    ui.spacing_mut().item_spacing = vec2(gap, 0.0);
                                    for _ in 0..cols {
                                        if i >= catalog.len() {
                                            break;
                                        }
                                        let em = catalog[i].to_string();
                                        i += 1;
                                        self.stamp_cell(
                                            ui,
                                            ctx,
                                            paper_col,
                                            ink,
                                            cell,
                                            Some(&em),
                                            &mut chosen,
                                        );
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
        side: f32,
        em: Option<&str>,
        chosen: &mut Option<String>,
    ) -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(3), mix_col(paper, ink, 0.06));
        }
        if let Some(em) = em {
            self.paint_emoji(ctx, &p, rect.shrink(5.0), em);
            if resp.clicked() {
                *chosen = Some(em.to_string());
                return true;
            }
        }
        false
    }

    fn haul_from_bin(&self) -> bool {
        self.shelf_haul_ids
            .first()
            .copied()
            .is_some_and(|id| self.lib.is_trashed(id))
    }

    fn bin_open(&self) -> bool {
        self.shelf_trash
    }

    fn shelf_slot() -> Vec2 {
        vec2(
            DOS_W + DOS_PAD * 2.0,
            DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW,
        )
    }

    fn ui_bin_zone(
        &mut self,
        ctx: &Context,
        open: &mut Option<Uuid>,
        select: &mut Option<Uuid>,
        toggle: &mut Option<Uuid>,
        range: &mut Option<Uuid>,
    ) {
        let slot = Self::shelf_slot();
        let h = slot.y + 20.0;
        let rust = self.look.rust();
        let fill = mix_col(
            self.look.desk,
            rust,
            if self.look.dark { 0.34 } else { 0.22 },
        );
        TopBottomPanel::bottom("bin-zone")
            .exact_height(h)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(fill))
            .show(ctx, |ui| {
                let shifting = self.shelf_haul_armed;
                let left = 28.0;
                let pitch = vec2(slot.x + SHELF_GAP, slot.y + SHELF_GAP);
                let cols = TRASH_SLOTS as usize;
                let grid = vec2(left + cols as f32 * pitch.x, slot.y);
                ui.add_space(8.0);
                let (full, _) = ui.allocate_exact_size(grid, Sense::hover());
                let origin = pos2(full.min.x + left, full.min.y);
                let occ: HashMap<u32, crate::library::NoteMeta> = self
                    .lib
                    .index
                    .trash
                    .iter()
                    .filter(|m| !self.shelf_toss.iter().any(|t| t.id == m.id))
                    .map(|m| (m.slot, m.clone()))
                    .collect();
                for idx in 0..cols {
                    let cell = Rect::from_min_size(
                        origin + vec2(idx as f32 * pitch.x, 0.0),
                        slot,
                    );
                    self.trash_grid.push(cell);
                    let meta = occ.get(&(idx as u32)).cloned();
                    let lifted = meta
                        .as_ref()
                        .map(|m| shifting && self.shelf_haul_ids.contains(&m.id))
                        .unwrap_or(false);
                    let hot = shifting && self.shelf_drop == Some(ShelfDrop::Bin(idx));
                    if let Some(meta) = meta.filter(|_| !lifted) {
                        ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                            let selected = self.shelf_sel.contains(&meta.id);
                            match self.cahier_dos(ui, &meta, selected, true) {
                                Some(DosAct::Open) => *open = Some(meta.id),
                                Some(DosAct::Select) => *select = Some(meta.id),
                                Some(DosAct::Toggle) => *toggle = Some(meta.id),
                                Some(DosAct::Range) => *range = Some(meta.id),
                                None => {}
                            }
                        });
                    } else if shifting {
                        self.place_dos(ui, cell, hot);
                    }
                }
            });
    }

    fn paint_sel_wash(&self, p: &Painter, rect: Rect, strong: bool) {
        let (r, g, b) = if self.look.dark {
            (255_u8, 255, 255)
        } else {
            (self.look.ink.r(), self.look.ink.g(), self.look.ink.b())
        };
        let fill = if strong {
            if self.look.dark { 28 } else { 22 }
        } else if self.look.dark {
            18
        } else {
            16
        };
        let line = if strong {
            if self.look.dark { 78 } else { 56 }
        } else if self.look.dark {
            52
        } else {
            36
        };
        p.rect_filled(
            rect,
            CornerRadius::same(11),
            Color32::from_rgba_unmultiplied(r, g, b, fill),
        );
        p.rect_stroke(
            rect,
            CornerRadius::same(11),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(r, g, b, line)),
            StrokeKind::Inside,
        );
    }

    fn place_dos(&self, ui: &mut Ui, cell: Rect, hot: bool) {
        let face = Rect::from_min_size(
            pos2(
                cell.center().x - DOS_W * 0.5,
                cell.min.y + DOS_PAD + PAPER_PEEK,
            ),
            vec2(DOS_W, DOS_H),
        );
        self.paint_sel_wash(&ui.painter_at(cell), face.expand(5.0), hot);
    }

    fn cahier_dos(
        &mut self,
        ui: &mut Ui,
        meta: &crate::library::NoteMeta,
        selected: bool,
        in_bin: bool,
    ) -> Option<DosAct> {
        let slot = vec2(
            DOS_W + DOS_PAD * 2.0,
            DOS_PAD + PAPER_PEEK + DOS_H + TITLE_ROW,
        );
        let (slot_rect, resp) = ui.allocate_exact_size(slot, Sense::click());
        let id = Id::new("cahier-dos").with(meta.id);
        let renaming = self.rename_id == Some(meta.id) && !in_bin;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = !renaming && pointer.is_some_and(|p| slot_rect.contains(p));
        if over {
            ui.ctx().request_repaint();
        }
        let lift_t = ui.ctx().animate_bool_with_time(id.with("peek"), over, 0.16);
        let lift = lift_t * lift_t * (3.0 - 2.0 * lift_t);
        let e = if in_bin { lift * 0.2 } else { lift };
        let cloth = if in_bin {
            mix_col(self.look.cloth_at(meta.cover), self.look.rust(), 0.16)
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
        let hit = Rect::from_min_max(face.min, pos2(face.max.x, face.max.y + TITLE_ROW));
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
            self.paint_sel_wash(&painter, face.expand(5.0), false);
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

        // Center of the board (the title sits under the spine).
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
            pos2(face.min.x, face.max.y + 10.0),
            vec2(face.width(), TITLE_ROW - 10.0),
        );
        if renaming {
            self.rename_rect = Some(title_rect);
        } else {
            let title: String = {
                let t = meta.title.as_str();
                if t.chars().count() > 14 {
                    format!("{}…", t.chars().take(12).collect::<String>())
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
                    vec2((title_rect.width() - 4.0).max(56.0), 30.0),
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
                self.look.serif(21.0),
                self.look.fg,
            );
            if !in_bin {
                let title_resp = ui.interact(title_rect, id.with("title"), Sense::click());
                if title_resp.clicked() {
                    self.rename_buf = meta.title.clone();
                    self.rename_id = Some(meta.id);
                    self.rename_rect = Some(title_rect);
                }
                title_resp.on_hover_cursor(CursorIcon::Text);
            }
        }

        let mut act = None;
        if act.is_none()
            && !renaming
            && !resp.secondary_clicked()
            && self.spine_wheel.is_none()
        {
            let on_title = pointer.is_some_and(|p| title_rect.contains(p));
            let mods = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            if resp.double_clicked() && !on_title && !in_bin {
                act = Some(DosAct::Open);
            } else if resp.clicked()
                && !on_title
                && !self.shelf_band_armed
                && !self.shelf_haul_armed
                && !self.shelf_fed
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

    fn open_spine_wheel(&mut self, id: Uuid, pos: Pos2, ctx: &Context, secondary: bool) {
        let now = ctx.input(|i| i.time);
        self.spine_wheel = Some(SpineWheel {
            id,
            origin: pos,
            t0: now,
            colors: false,
            hover: None,
            secondary,
            color_hold_t0: None,
        });
        self.shelf_haul = None;
        self.shelf_haul_now = None;
        self.shelf_haul_armed = false;
        self.shelf_haul_ids.clear();
        self.shelf_drop = None;
        self.shelf_hold_t0 = None;
        self.shelf_fed = true;
    }

    fn spine_wheel_tick(&mut self, ctx: &Context) {
        let Some(wheel) = self.spine_wheel.as_mut() else {
            return;
        };
        let n_cloth = self.look.cloth.len().max(1);
        let secondary = wheel.secondary;
        let (pos, down, released, primary_pressed, now) = ctx.input(|i| {
            (
                i.pointer.interact_pos().or(i.pointer.hover_pos()),
                if secondary {
                    i.pointer.button_down(PointerButton::Secondary)
                } else {
                    i.pointer.primary_down()
                },
                if secondary {
                    i.pointer.button_released(PointerButton::Secondary)
                } else {
                    i.pointer.primary_released()
                },
                i.pointer.primary_pressed(),
                i.time,
            )
        });
        if let Some(pos) = pos {
            wheel.hover = wheel_hit(wheel.origin, pos, wheel.colors, n_cloth);
            // Finger: linger on Color to open the cloth ring.
            // Mouse: left-click Color instead (handled below).
            if !secondary && !wheel.colors {
                if wheel.hover == Some(WheelPick::Color) {
                    let t0 = *wheel.color_hold_t0.get_or_insert(now);
                    if now - t0 >= COLOR_HOLD {
                        wheel.colors = true;
                        wheel.color_hold_t0 = None;
                        wheel.hover = wheel_hit(wheel.origin, pos, true, n_cloth);
                    }
                } else {
                    wheel.color_hold_t0 = None;
                }
            }
        }

        if secondary {
            // Mouse: right-click opened the wheel; left-click activates.
            if primary_pressed {
                let id = wheel.id;
                let pick = wheel.hover;
                let origin = wheel.origin;
                let colors = wheel.colors;
                match pick {
                    Some(WheelPick::Color) if !colors => {
                        wheel.colors = true;
                        wheel.color_hold_t0 = None;
                        if let Some(pos) = pos {
                            wheel.hover = wheel_hit(wheel.origin, pos, true, n_cloth);
                        }
                        self.shelf_fed = true;
                    }
                    Some(_) => {
                        self.spine_wheel = None;
                        self.shelf_fed = true;
                        self.apply_spine_wheel(id, pick, origin, now);
                    }
                    None => {
                        // On the cloth ring, center click steps back to actions.
                        let at_center =
                            pos.is_some_and(|p| (p - origin).length() < WHEEL_IN);
                        if colors && at_center {
                            wheel.colors = false;
                            wheel.color_hold_t0 = None;
                            if let Some(pos) = pos {
                                wheel.hover = wheel_hit(origin, pos, false, n_cloth);
                            }
                            self.shelf_fed = true;
                        } else {
                            self.spine_wheel = None;
                            self.shelf_fed = true;
                        }
                    }
                }
            }
            ctx.request_repaint();
            return;
        }

        // Finger: commit on release.
        if released || !down {
            let id = wheel.id;
            let pick = wheel.hover;
            let origin = wheel.origin;
            self.spine_wheel = None;
            self.shelf_fed = true;
            self.apply_spine_wheel(id, pick, origin, now);
        }
        ctx.request_repaint();
    }

    fn apply_spine_wheel(&mut self, id: Uuid, pick: Option<WheelPick>, origin: Pos2, t: f64) {
        match pick {
            Some(WheelPick::Pin) => {
                if let Some(mut note) = self.lib.load_note(id) {
                    note.pinned = !note.pinned;
                    let pinned = note.pinned;
                    self.lib.save_note(&note);
                    if pinned {
                        self.lib.bring_front(id);
                    }
                }
            }
            Some(WheelPick::Mark) => {
                self.emoji_pick = Some(id);
                self.rename_id = None;
            }
            Some(WheelPick::Trash) => {
                self.shelf_haul_ids = vec![id];
                self.shelf_haul_now = Some(origin);
                self.begin_toss(t);
                self.shelf_fed = true;
            }
            Some(WheelPick::Cloth(i)) => {
                if let Some(mut note) = self.lib.load_note(id) {
                    note.cover = i;
                    note.touch();
                    self.lib.save_note(&note);
                }
            }
            Some(WheelPick::Color) | None => {}
        }
    }

    fn paint_spine_wheel(&mut self, ctx: &Context) {
        let Some(wheel) = self.spine_wheel.as_ref() else {
            return;
        };
        let origin = wheel.origin;
        let colors = wheel.colors;
        let hover = wheel.hover;
        let t0 = wheel.t0;
        let now = ctx.input(|i| i.time);
        let appear = ((now - t0) / 0.05).clamp(0.0, 1.0) as f32;
        let s = 1.0 - (1.0 - appear) * (1.0 - appear);
        let r0 = WHEEL_IN * s;
        let r1 = WHEEL_OUT * s;
        let n_cloth = self.look.cloth.len().max(1);
        let paper = self.paper_tex(ctx);

        Area::new(Id::new("spine-wheel"))
            .fixed_pos(ctx.screen_rect().min)
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let p = ui.painter().clone();
                p.circle_filled(
                    origin,
                    r1 + 14.0,
                    Color32::from_black_alpha(if self.look.dark { 40 } else { 22 }),
                );
                if colors {
                    for i in 0..n_cloth {
                        let cloth = self.look.cloth_at(i as u8);
                        let hot = hover == Some(WheelPick::Cloth(i as u8));
                        let fill = if hot { shade_rgb(cloth, 1.22) } else { cloth };
                        p.add(wedge_mesh(origin, r0, r1, n_cloth, i, fill, None));
                        if hot {
                            p.add(wedge_stroke(origin, r0, r1, n_cloth, i, self.look.ink));
                        }
                    }
                } else {
                    let picks = [
                        WheelPick::Pin,
                        WheelPick::Color,
                        WheelPick::Trash,
                        WheelPick::Mark,
                    ];
                    for (i, pick) in picks.into_iter().enumerate() {
                        let hot = hover == Some(pick);
                        let tint = if hot {
                            Color32::from_rgb(0xe8, 0xe0, 0xd0)
                        } else {
                            Color32::WHITE
                        };
                        p.add(wedge_mesh(
                            origin,
                            r0,
                            r1,
                            4,
                            i,
                            tint,
                            Some((paper.id(), PAPER_TILE)),
                        ));
                        if hot {
                            p.add(wedge_stroke(origin, r0, r1, 4, i, self.look.ink));
                        }
                    }
                }
                p.circle_filled(origin, r0 - 1.0, self.look.desk);
                p.circle_stroke(
                    origin,
                    r0,
                    Stroke::new(1.4_f32, self.look.ink.gamma_multiply(0.28)),
                );
                p.circle_stroke(
                    origin,
                    r1,
                    Stroke::new(1.4_f32, self.look.ink.gamma_multiply(0.22)),
                );

                if colors {
                    for i in 0..n_cloth {
                        let c = slice_mid(origin, i, n_cloth, (r0 + r1) * 0.52);
                        p.circle_filled(c, 14.0, shade_rgb(self.look.cloth_at(i as u8), 1.08));
                        p.circle_stroke(
                            c,
                            14.0,
                            Stroke::new(1.3_f32, self.look.ink.gamma_multiply(0.45)),
                        );
                    }
                } else {
                    let icons = ["📌", "🎨", "🗑️", "😊"];
                    for (i, em) in icons.iter().enumerate() {
                        let c = slice_mid(origin, i, 4, r0 + (r1 - r0) * 0.55);
                        let bounds = Rect::from_center_size(c, vec2(40.0, 40.0));
                        self.paint_emoji(ctx, &p, bounds, em);
                    }
                }
            });
    }

    fn menu_row(&self, ui: &mut Ui, label: &str, hint: &str, checked: bool) -> bool {
        let w = 228.0;
        let h = 32.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let fill = if resp.hovered() {
            self.look.desk_edge
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), fill);
        let fg = self.look.fg;
        let mute = self.look.muted;
        if checked {
            ui.painter().text(
                pos2(rect.min.x + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                "✓",
                self.look.mono(13.0),
                fg,
            );
        }
        ui.painter().text(
            pos2(rect.min.x + 30.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            self.look.mono(13.0),
            fg,
        );
        if !hint.is_empty() {
            ui.painter().text(
                pos2(rect.max.x - 12.0, rect.center().y),
                Align2::RIGHT_CENTER,
                hint,
                self.look.mono(11.0),
                mute,
            );
        }
        resp.clicked()
    }

    fn menu_sep(&self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(228.0, 8.0), Sense::hover());
        ui.painter().hline(
            rect.x_range().shrink(10.0),
            rect.center().y,
            Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.35)),
        );
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

    /// Leave the lectern: a paper plate showing three spines.
    fn shelf_exit_btn(&self, ui: &mut Ui) -> Response {
        let h = self.chrome_btn();
        let w = h + 10.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let p = ui.painter();
        let fill = if resp.hovered() {
            mix_col(self.look.desk_deep, self.look.paper, 0.22)
        } else {
            self.look.desk_deep
        };
        p.rect_filled(rect, CornerRadius::same(9), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.45)),
            StrokeKind::Inside,
        );
        // Three notebook spines side by side — the shelf.
        let cloth = [
            self.look.accent,
            mix_col(self.look.ink, self.look.accent, 0.35),
            self.look.fg.gamma_multiply(0.72),
        ];
        let spine_w = 5.2;
        let spine_h = h * 0.52;
        let gap = 3.4;
        let total = spine_w * 3.0 + gap * 2.0;
        let left = rect.center().x - total * 0.5;
        let top = rect.center().y - spine_h * 0.5;
        for (i, col) in cloth.iter().enumerate() {
            let x = left + i as f32 * (spine_w + gap);
            let r = Rect::from_min_size(pos2(x, top), vec2(spine_w, spine_h));
            p.rect_filled(r, CornerRadius::same(1), *col);
            p.line_segment(
                [pos2(r.center().x, r.min.y + 2.2), pos2(r.center().x, r.max.y - 2.2)],
                Stroke::new(1.0_f32, self.look.desk_deep.gamma_multiply(0.35)),
            );
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn inkwell(&self, ui: &mut Ui) -> Response {
        let size = 56.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let g = self.look.green();
        let well = if resp.hovered() {
            shade_rgb(g, 1.14)
        } else {
            g
        };
        self.paint_inkwell_body(p, c, well);
        paint_plus(p, c, well_glyph(well, self.look.paper, self.look.ink));
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn ui_desk(&mut self, ctx: &Context) {
        self.handle_drops_and_paste(ctx);

        TopBottomPanel::top("rule")
            .exact_height(52.0)
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
                        .shelf_exit_btn(ui)
                        .on_hover_text("Shelf")
                        .clicked()
                    {
                        self.close_desk();
                        return;
                    }
                    ui.add_space(8.0);
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
                    let chrome = self.chrome_btn();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        let more = self
                            .round_well(ui, chrome, false, paint_more)
                            .on_hover_text("More");
                        let join = self
                            .note
                            .as_ref()
                            .map(|n| n.sheet_join)
                            .unwrap_or(SheetJoin::Linked);
                        let pinned = self.note.as_ref().is_some_and(|n| n.pinned);
                        let mut act = None;
                        Popup::menu(&more).show(|ui| {
                            ui.set_min_width(228.0);
                            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                            if self.menu_row(ui, "Save", "Ctrl+S", false) {
                                act = Some(MoreAct::Save);
                            }
                            if self.menu_row(ui, "Duplicate", "", false) {
                                act = Some(MoreAct::Duplicate);
                            }
                            if self.menu_row(ui, "Pin", "", pinned) {
                                act = Some(MoreAct::Pin);
                            }
                            self.menu_sep(ui);
                            if self.menu_row(
                                ui,
                                "Linked pages",
                                "",
                                join == SheetJoin::Linked,
                            ) {
                                act = Some(MoreAct::Linked);
                            }
                            if self.menu_row(
                                ui,
                                "Separate pages",
                                "",
                                join == SheetJoin::Separate,
                            ) {
                                act = Some(MoreAct::Separate);
                            }
                            self.menu_sep(ui);
                            if self.menu_row(ui, "PNG", "Ctrl+E", false) {
                                act = Some(MoreAct::Png);
                            }
                            if self.menu_row(ui, "PDF", "Ctrl+Shift+E", false) {
                                act = Some(MoreAct::Pdf);
                            }
                            self.menu_sep(ui);
                            if self.menu_row(ui, "Move to trash", "", false) {
                                act = Some(MoreAct::Trash);
                            }
                        });
                        match act {
                            Some(MoreAct::Save) => self.save_now(ctx),
                            Some(MoreAct::Duplicate) => self.duplicate_open_note(),
                            Some(MoreAct::Pin) => self.toggle_pin_open(),
                            Some(MoreAct::Linked) => self.set_sheet_join(SheetJoin::Linked),
                            Some(MoreAct::Separate) => {
                                self.set_sheet_join(SheetJoin::Separate)
                            }
                            Some(MoreAct::Png) => self.export_png(ctx),
                            Some(MoreAct::Pdf) => self.export_pdf(ctx),
                            Some(MoreAct::Trash) => self.trash_open_note(),
                            None => {}
                        }
                        if self
                            .round_well(ui, chrome, false, paint_fit)
                            .on_hover_text("Fit")
                            .clicked()
                        {
                            self.fit_to_screen();
                        }
                        if self
                            .round_well(ui, chrome, false, paint_zoom_in)
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
                            let (r, lab) =
                                ui.allocate_exact_size(vec2(56.0, chrome), Sense::click());
                            ui.painter()
                                .rect_filled(r, CornerRadius::same(10), self.look.desk_deep);
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                pct,
                                self.look.mono(13.0),
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
                            .round_well(ui, chrome, false, paint_zoom_out)
                            .on_hover_text("Zoom out")
                            .clicked()
                        {
                            self.zoom_out();
                        }
                        if self
                            .round_well(ui, chrome, false, paint_paper_icon)
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
                    });
                });
            });

        CentralPanel::default()
            .frame(Frame::NONE.fill(self.look.desk))
            .show(ctx, |ui| {
                self.ui_canvas(ui);
            });

        self.ui_floating_dock(ctx);
        self.ui_ink_strip(ctx);
        self.ui_ink_tin(ctx);
        if let Some((page, local)) = self.pending_image.take() {
            self.pick_image(page, local);
        }
    }

    fn ui_floating_dock(&mut self, ctx: &Context) {
        let screen = ctx.screen_rect();
        let vertical = self.dock_edge.vertical();
        let edge_key = match self.dock_edge {
            DockEdge::Top => 0_u8,
            DockEdge::Bottom => 1,
            DockEdge::Left => 2,
            DockEdge::Right => 3,
        };
        // Distinct id per edge, so egui does not reuse the previous layout size
        // (a vertical bar would become a large square once horizontal).
        let mut area = Area::new(Id::new(("cahier-dock", edge_key)))
            .order(Order::Foreground)
            .interactable(true)
            .constrain(true);
        if let Some(pos) = self.dock_float {
            let pos = pos.clamp(screen.min, pos2(screen.max.x - 48.0, screen.max.y - 48.0));
            area = area.current_pos(pos);
        } else {
            let (align, off) = match self.dock_edge {
                DockEdge::Bottom => (Align2::CENTER_BOTTOM, vec2(0.0, -12.0)),
                DockEdge::Top => (Align2::CENTER_TOP, vec2(0.0, 60.0)),
                DockEdge::Left => (Align2::LEFT_CENTER, vec2(10.0, 0.0)),
                DockEdge::Right => (Align2::RIGHT_CENTER, vec2(-10.0, 0.0)),
            };
            area = area.anchor(align, off);
        }
        let inner = area.show(ctx, |ui| {
            // Shrink-wrap, so the area stays inside a tight max_rect.
            ui.set_max_size(vec2(
                (screen.width() - 24.0).max(80.0),
                (screen.height() - 80.0).max(80.0),
            ));
            Frame::NONE
                .fill(self.look.desk_deep)
                .stroke(Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.38)))
                .corner_radius(30)
                .inner_margin(if vertical {
                    Margin::symmetric(6, 8)
                } else {
                    Margin::symmetric(8, 6)
                })
                .show(ui, |ui| {
                    if vertical {
                        ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                        ui.vertical(|ui| {
                            self.dock_inner(ui, true);
                        });
                    } else {
                        ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                        ui.horizontal(|ui| {
                            self.dock_inner(ui, false);
                        });
                    }
                });
        });
        ctx.data_mut(|d| d.insert_temp(Id::new("dock-rect"), inner.response.rect));
    }

    fn ui_ink_strip(&mut self, ctx: &Context) {
        if !self.palette_open {
            ctx.data_mut(|d| d.remove::<Rect>(Id::new("strip-rect")));
            return;
        }
        let screen = ctx.screen_rect();
        let dock = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap_or(Rect::from_center_size(screen.center(), vec2(48.0, 48.0)));
        let vertical = self.dock_edge.vertical();
        let gap = 8.0;
        let high = self.tool == Tool::Highlighter;
        let n_colors = if high {
            self.look.highs.len()
        } else {
            self.look.inks.len().saturating_sub(1).max(1)
        };
        let show_w = self.tool.is_ink() || self.tool.is_eraser();
        let cell = 26.0;
        let strip_pad = 10.0;
        let items = n_colors + 1 + if show_w { 3 } else { 0 };
        let long = strip_pad * 2.0 + items as f32 * cell + (items.saturating_sub(1) as f32) * 2.0;
        let short = 40.0;
        let (sw, sh) = if vertical {
            (short, long.min(screen.height() - 100.0))
        } else {
            (long.min(screen.width() - 80.0), short)
        };
        let pos = match self.dock_edge {
            DockEdge::Left => pos2(
                dock.max.x + gap,
                (dock.center().y - sh * 0.5).clamp(screen.min.y + 48.0, screen.max.y - sh - 12.0),
            ),
            DockEdge::Right => pos2(
                dock.min.x - gap - sw,
                (dock.center().y - sh * 0.5).clamp(screen.min.y + 48.0, screen.max.y - sh - 12.0),
            ),
            DockEdge::Top => pos2(
                (dock.center().x - sw * 0.5).clamp(screen.min.x + 12.0, screen.max.x - sw - 12.0),
                dock.max.y + gap,
            ),
            DockEdge::Bottom => pos2(
                (dock.center().x - sw * 0.5).clamp(screen.min.x + 12.0, screen.max.x - sw - 12.0),
                dock.min.y - gap - sh,
            ),
        };
        let current = Color32::from_rgb(self.ink.r(), self.ink.g(), self.ink.b());
        let inner = Area::new(Id::new("cahier-ink-strip"))
            .order(Order::Foreground)
            .fixed_pos(pos)
            .constrain(true)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(self.look.desk_deep)
                    .stroke(Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.38)))
                    .corner_radius(22)
                    .inner_margin(Margin::symmetric(6, 6))
                    .show(ui, |ui| {
                        if vertical {
                            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                                self.ink_strip_inner(ui, true, high, current, show_w);
                            });
                        } else {
                            ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                self.ink_strip_inner(ui, false, high, current, show_w);
                            });
                        }
                    });
            });
        ctx.data_mut(|d| d.insert_temp(Id::new("strip-rect"), inner.response.rect));
        if ctx.input(|i| i.pointer.primary_pressed()) {
            if let Some(p) = ctx.pointer_latest_pos() {
                let on_strip = inner.response.rect.expand(4.0).contains(p);
                let on_dock = dock.expand(6.0).contains(p);
                let on_tin = ctx
                    .data(|d| d.get_temp::<Rect>(Id::new("tin-rect")))
                    .map(|r| r.expand(4.0).contains(p))
                    .unwrap_or(false);
                if !on_strip && !on_dock && !on_tin {
                    self.palette_open = false;
                    self.tin_open = false;
                }
            }
        }
    }

    fn ink_strip_inner(
        &mut self,
        ui: &mut Ui,
        vertical: bool,
        high: bool,
        current: Color32,
        show_w: bool,
    ) {
        if high {
            let n = self.look.highs.len();
            for i in 0..n {
                let c = self.look.highs[i];
                let swatch = Color32::from_rgb(c.r(), c.g(), c.b());
                if self.color_dot(ui, swatch, same_rgb(swatch, current), vertical) {
                    self.pick_theme_ink(i);
                }
            }
        } else {
            let n = self.look.inks.len().saturating_sub(1).max(1);
            for i in 0..n {
                let c = self.look.inks[i];
                if self.color_dot(ui, c, same_rgb(c, current) && !self.color_custom, vertical) {
                    self.pick_theme_ink(i);
                }
            }
        }
        if self
            .rainbow_well(ui, self.tin_open)
            .on_hover_text("Color tin")
            .clicked()
        {
            self.tin_open = !self.tin_open;
        }
        if show_w {
            ui.add_space(4.0);
            for &(w, r) in &[(2.2_f32, 3.0), (5.0, 4.4), (11.0, 6.0)] {
                let on = (self.width - w).abs() < 1.6;
                if self.thick_dot(ui, r, on, vertical) {
                    self.width = w;
                    if self.tool.is_ink() {
                        self.last_ink_width = self.width;
                    }
                }
            }
        }
    }

    fn ui_ink_tin(&mut self, ctx: &Context) {
        if !self.tin_open {
            ctx.data_mut(|d| d.remove::<Rect>(Id::new("tin-rect")));
            return;
        }
        let screen = ctx.screen_rect();
        let dock = ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock-rect")))
            .unwrap_or(Rect::from_center_size(screen.center(), vec2(48.0, 48.0)));
        let tin_w = 228.0;
        let tin_h = 268.0;
        let gap = 10.0;
        let pos = match self.dock_edge {
            DockEdge::Left => pos2(dock.max.x + gap, (dock.center().y - tin_h * 0.5).max(screen.min.y + 48.0)),
            DockEdge::Right => pos2(
                dock.min.x - gap - tin_w,
                (dock.center().y - tin_h * 0.5).max(screen.min.y + 48.0),
            ),
            DockEdge::Top => pos2(dock.center().x - tin_w * 0.5, dock.max.y + gap),
            DockEdge::Bottom => pos2(dock.center().x - tin_w * 0.5, dock.min.y - gap - tin_h),
        };
        let inner = Area::new(Id::new("cahier-tin"))
            .order(Order::Foreground)
            .fixed_pos(pos)
            .constrain(true)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(self.look.desk_deep)
                    .stroke(Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.38)))
                    .corner_radius(18)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(tin_w - 24.0);
                        ui.spacing_mut().item_spacing = vec2(4.0, 8.0);
                        let sq = 204.0;
                        let (sv, sv_resp) =
                            ui.allocate_exact_size(vec2(sq, 132.0), Sense::click_and_drag());
                        paint_sv_field(ui.painter(), sv, self.tin_hue);
                        let (_, s0, v0) = rgb_to_hsv(self.ink);
                        let cur = pos2(sv.min.x + s0 * sv.width(), sv.min.y + (1.0 - v0) * sv.height());
                        ui.painter().circle_stroke(
                            cur,
                            6.0,
                            Stroke::new(2.0_f32, Color32::WHITE),
                        );
                        ui.painter().circle_stroke(
                            cur,
                            6.0,
                            Stroke::new(1.0_f32, self.look.fg.gamma_multiply(0.7)),
                        );
                        if sv_resp.dragged() || sv_resp.clicked() {
                            if let Some(p) = sv_resp.interact_pointer_pos() {
                                let s = ((p.x - sv.min.x) / sv.width()).clamp(0.0, 1.0);
                                let v = (1.0 - (p.y - sv.min.y) / sv.height()).clamp(0.0, 1.0);
                                self.pick_free_ink(hsv_to_rgb(self.tin_hue, s, v));
                            }
                        }

                        let (hue_r, hue_resp) =
                            ui.allocate_exact_size(vec2(sq, 16.0), Sense::click_and_drag());
                        paint_hue_bar(ui.painter(), hue_r);
                        let hx = hue_r.min.x + self.tin_hue * hue_r.width();
                        ui.painter().vline(
                            hx,
                            hue_r.y_range(),
                            Stroke::new(2.0_f32, Color32::WHITE),
                        );
                        if hue_resp.dragged() || hue_resp.clicked() {
                            if let Some(p) = hue_resp.interact_pointer_pos() {
                                self.tin_hue = ((p.x - hue_r.min.x) / hue_r.width()).clamp(0.0, 1.0);
                                let (_, s, v) = rgb_to_hsv(self.ink);
                                self.pick_free_ink(hsv_to_rgb(self.tin_hue, s.max(0.12), v.max(0.18)));
                            }
                        }

                        let cols = 12;
                        let rows = 6;
                        let pan = 14.5;
                        let gap_p = 2.6;
                        let grid_w = cols as f32 * pan + (cols - 1) as f32 * gap_p;
                        let grid_h = (rows + 1) as f32 * pan + rows as f32 * gap_p;
                        let (grid, _) =
                            ui.allocate_exact_size(vec2(grid_w.max(sq), grid_h), Sense::hover());
                        let origin = pos2(
                            grid.min.x + (grid.width() - grid_w) * 0.5,
                            grid.min.y,
                        );
                        let painter = ui.painter();
                        for row in 0..rows {
                            for col in 0..cols {
                                let h = col as f32 / cols as f32;
                                let v = 0.92 - row as f32 * 0.13;
                                let s = 0.88;
                                let c = hsv_to_rgb(h, s, v);
                                let r = Rect::from_min_size(
                                    origin
                                        + vec2(
                                            col as f32 * (pan + gap_p),
                                            row as f32 * (pan + gap_p),
                                        ),
                                    vec2(pan, pan),
                                );
                                painter.circle_filled(r.center(), pan * 0.42, c);
                                if same_rgb(c, self.ink) {
                                    painter.circle_stroke(
                                        r.center(),
                                        pan * 0.42 + 2.0,
                                        Stroke::new(1.5_f32, self.look.fg),
                                    );
                                }
                                let hit = ui.interact(r, Id::new(("tin-pan", row, col)), Sense::click());
                                if hit.clicked() {
                                    self.pick_free_ink(c);
                                }
                            }
                        }
                        let gray_y = origin.y + rows as f32 * (pan + gap_p);
                        for col in 0..cols {
                            let g = col as f32 / (cols - 1) as f32;
                            let c = hsv_to_rgb(0.0, 0.0, g);
                            let r = Rect::from_min_size(
                                pos2(origin.x + col as f32 * (pan + gap_p), gray_y),
                                vec2(pan, pan),
                            );
                            painter.circle_filled(r.center(), pan * 0.42, c);
                            painter.circle_stroke(
                                r.center(),
                                pan * 0.42,
                                Stroke::new(0.6_f32, self.look.fg.gamma_multiply(0.35)),
                            );
                            if same_rgb(c, self.ink) {
                                painter.circle_stroke(
                                    r.center(),
                                    pan * 0.42 + 2.0,
                                    Stroke::new(1.5_f32, self.look.fg),
                                );
                            }
                            let hit = ui.interact(r, Id::new(("tin-gray", col)), Sense::click());
                            if hit.clicked() {
                                self.pick_free_ink(c);
                            }
                        }
                    });
            });
        ctx.data_mut(|d| d.insert_temp(Id::new("tin-rect"), inner.response.rect));
        if ctx.input(|i| i.pointer.primary_pressed()) {
            if let Some(p) = ctx.pointer_latest_pos() {
                let on_tin = inner.response.rect.expand(4.0).contains(p);
                let on_dock = dock.expand(6.0).contains(p);
                let on_strip = ctx
                    .data(|d| d.get_temp::<Rect>(Id::new("strip-rect")))
                    .map(|r| r.expand(4.0).contains(p))
                    .unwrap_or(false);
                if !on_tin && !on_dock && !on_strip {
                    self.tin_open = false;
                }
            }
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
        let current = Color32::from_rgb(self.ink.r(), self.ink.g(), self.ink.b());
        if self
            .palette_chip(ui, current, self.palette_open || self.tin_open, vertical)
            .on_hover_text(if self.palette_open {
                "Fold ink"
            } else {
                "Ink"
            })
            .clicked()
        {
            self.palette_open = !self.palette_open;
            if !self.palette_open {
                self.tin_open = false;
            }
        }
        self.dock_gap(ui, vertical);
        if self.tool_glyph(ui, Tool::Text) {
            self.tool = Tool::Text;
        }
        if self.tool_glyph(ui, Tool::Image) {
            self.finish_text_edit();
            self.tool = Tool::Image;
            self.pending_image = Some(self.drop_spot());
        }
    }

    fn dock_grip(&self, ui: &mut Ui, vertical: bool) -> Response {
        let s = self.slot();
        let size = if vertical {
            vec2(s, 14.0)
        } else {
            vec2(14.0, s)
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
        let s = self.slot();
        if vertical {
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(vec2(s, 2.0), Sense::hover());
            ui.painter().line_segment(
                [
                    pos2(rect.min.x + 1.0, rect.center().y),
                    pos2(rect.max.x - 1.0, rect.center().y),
                ],
                Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.55)),
            );
            ui.add_space(2.0);
        } else {
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(vec2(2.0, s), Sense::hover());
            ui.painter().line_segment(
                [
                    pos2(rect.center().x, rect.min.y + 1.0),
                    pos2(rect.center().x, rect.max.y - 1.0),
                ],
                Stroke::new(1.0_f32, self.look.muted.gamma_multiply(0.55)),
            );
            ui.add_space(2.0);
        }
    }

    fn palette_chip(&self, ui: &mut Ui, color: Color32, open: bool, vertical: bool) -> Response {
        let s = self.slot();
        let size = if vertical {
            vec2(s, 30.0)
        } else {
            vec2(32.0, s)
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let c = rect.center();
        let p = ui.painter();
        let r = 7.4;
        if !open {
            p.circle_filled(c + vec2(2.4, 2.1), r - 0.6, color.gamma_multiply(0.55));
        }
        p.circle_filled(c, r + 1.3, self.look.fg.gamma_multiply(0.55));
        p.circle_filled(c, r, color);
        p.circle_stroke(
            c,
            r,
            Stroke::new(1.05_f32, self.look.fg.gamma_multiply(0.55)),
        );
        if open {
            p.circle_stroke(c, r + 3.4, Stroke::new(1.5_f32, self.look.fg));
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn rainbow_well(&self, ui: &mut Ui, on: bool) -> Response {
        let s = self.slot();
        let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click());
        let p = ui.painter();
        let c = rect.center();
        let r = s * 0.34;
        for i in 0..12 {
            let t0 = i as f32 / 12.0;
            let t1 = (i + 1) as f32 / 12.0;
            let a0 = t0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let a1 = t1 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let col = hsv_to_rgb(t0 + 0.04, 0.9, 0.92);
            p.add(egui::Shape::convex_polygon(
                vec![
                    c,
                    pos2(c.x + a0.cos() * r, c.y + a0.sin() * r),
                    pos2(c.x + a1.cos() * r, c.y + a1.sin() * r),
                ],
                col,
                Stroke::NONE,
            ));
        }
        p.circle_filled(c, r * 0.38, self.look.desk_deep);
        if on {
            p.circle_stroke(c, r + 2.6, Stroke::new(1.6_f32, self.look.fg));
        }
        resp.on_hover_cursor(CursorIcon::PointingHand)
    }

    fn color_dot(&self, ui: &mut Ui, color: Color32, on: bool, vertical: bool) -> bool {
        let s = self.slot();
        let size = if vertical {
            vec2(s, 28.0)
        } else {
            vec2(30.0, s)
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
        let s = self.slot();
        let size = if vertical {
            vec2(s, 28.0)
        } else {
            vec2(28.0, s)
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

    fn tool_glyph(&mut self, ui: &mut Ui, tool: Tool) -> bool {
        self.paint_tool_well(ui, tool, self.tool == tool)
            .on_hover_text(self.tool_hover(tool))
            .clicked()
    }

    fn paint_tool_well(&mut self, ui: &mut Ui, tool: Tool, active: bool) -> Response {
        if let Some(em) = tool.emoji() {
            return self.emoji_well(ui, em, active);
        }
        self.round_well(ui, self.slot(), active, |p, c, fg| match tool {
            Tool::Text => paint_text_icon(p, c, fg),
            Tool::Image => paint_image_icon(p, c, fg),
            _ => {}
        })
    }

    fn emoji_well(&mut self, ui: &mut Ui, emoji: &str, active: bool) -> Response {
        let s = self.slot();
        let (rect, resp) = ui.allocate_exact_size(vec2(s, s), Sense::click());
        let bg = if active {
            self.look.accent
        } else if resp.hovered() {
            self.look.desk_edge
        } else {
            Color32::TRANSPARENT
        };
        let c = rect.center();
        {
            let p = ui.painter();
            if bg.a() > 0 {
                p.circle_filled(c, s * 0.43, bg);
            }
        }
        let bounds = Rect::from_center_size(c, vec2(s * 0.64, s * 0.64));
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        self.paint_emoji(&ctx, &p, bounds, emoji);
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
                let origin = self.origin_of(page.min(n - 1));
                self.camera.show_writing(rect, origin, pw, ph);
            } else if self.need_fit {
                let page = self.page_in_view(rect);
                self.camera.fit_page(rect, self.origin_of(page), pw, ph);
                self.need_fit = false;
            }
        }

        self.handle_camera(ui, &resp, rect);
        self.handle_tool(ui, &resp, rect);
        self.paint_world(&painter, rect);
        self.paint_sheet_tabs(ui, &painter, rect);
        self.paint_page_folds(ui, &painter, rect);
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
            // Trackpad: two fingers. Vertical = zoom, horizontal = pan.
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
        let on_tab = self.tab_held
            || ui.input(|i| i.pointer.press_origin()).is_some_and(|p| {
                self.sheet_tab_at(p, rect).is_some() || self.fold_at(p, rect).is_some()
            });
        if finger_pan
            && !on_tab
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
        if self.tab_held {
            let down = ui.input(|i| i.pointer.primary_down()) || self.tablet.snapshot().down;
            self.tab_held = false;
            if down {
                self.tab_held = true;
            }
            return;
        }
        let space = ui.input(|i| i.key_down(Key::Space));
        if space
            || ui.input(|i| i.pointer.middle_down())
            || ui.input(|i| i.multi_touch().is_some())
            || self.tablet.snapshot().pinching
        {
            return;
        }
        let pen = self.tablet.snapshot();
        let pen_ink =
            pen.down || pen.pressed || pen.released || (self.live.is_some() && self.live_from_pen);
        let screen_touch = ui.input(|i| i.any_touches());
        let on_tab = ui
            .input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()))
            .is_some_and(|p| {
                self.sheet_tab_at(p, rect).is_some() || self.fold_at(p, rect).is_some()
            });
        if !pen_ink && self.live.is_none() && !on_tab {
            if self.is_tablette() {
                if self.palm_guard(&pen) || self.finger_alive(ui) {
                    return;
                }
            } else if screen_touch {
                return;
            }
        }

        let extra: Vec<Pos2>;
        let (screen, primary_down, primary_pressed, primary_released, secondary) = if pen_ink {
            if let Some(screen) = pen.pos {
                extra = pen.samples;
                (screen, pen.down, pen.pressed, pen.released, pen.eraser)
            } else {
                if self.live.is_some() && self.live_from_pen && (pen.released || !pen.down) {
                    self.finish_live();
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
                || d.get_temp::<Rect>(Id::new("tin-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("strip-rect"))
                    .map(|r| r.expand(2.0).contains(screen))
                    .unwrap_or(false)
        }) {
            if self.live.is_some() && (primary_released || !primary_down) {
                self.finish_live();
            }
            return;
        }
        if primary_pressed {
            if let Some((col, row)) = self.fold_at(screen, rect) {
                self.tear_sheet_at(col, row);
                self.tab_held = true;
                return;
            }
            if let Some((col, row)) = self.sheet_tab_at(screen, rect) {
                self.add_sheet_from_tab(col, row);
                self.tab_held = true;
                return;
            }
        }
        let paper = self.camera.to_paper(screen, rect);
        let page = self.page_at_paper(paper);
        let local = paper - self.origin_of(page);

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
                self.shape_anchor = Some(local);
                self.shape_still = Some(Instant::now());
                self.shape_preview = None;
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
            let cells = self.page_cells();
            let pw = self.pw();
            let ph = self.ph();
            let gap = self.gap();
            let pressure = self.pressure.latest();
            if let Some((pgi, live)) = &mut self.live {
                for sp in &extra {
                    let pp = cam.to_paper(*sp, rect);
                    if page_at(pp, &cells, pw, ph, gap) == *pgi {
                        let (col, row) = cells.get(*pgi).copied().unwrap_or((0, 0));
                        let loc = pp - page_origin(col, row, pw, ph, gap);
                        live.push(InkPoint::new(
                            loc,
                            mixed_pressure(live.nib, pressure, 200.0),
                        ));
                    }
                }
            }
            self.consider_shape(local);
        }
        if self.live.is_some()
            && (primary_released || !primary_down)
            && !(tool.is_ink() && primary_down)
        {
            self.finish_live();
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

        let tap = primary_pressed || resp.clicked();
        if tool == Tool::Text && tap {
            if let Some((pg, id)) = self.hit_text(page, local) {
                if self.editing_text != Some((pg, id)) {
                    self.finish_text_edit();
                    self.editing_text = Some((pg, id));
                    self.text_focus = true;
                }
            } else {
                let on_live = self.editing_text.is_some_and(|(pg, id)| {
                    pg == page
                        && self.note.as_ref().is_some_and(|n| {
                            n.pages.get(pg).is_some_and(|p| {
                                p.texts
                                    .iter()
                                    .find(|t| t.id == id)
                                    .is_some_and(|t| t.rect().expand(12.0).contains(local))
                            })
                        })
                });
                if !on_live {
                    self.finish_text_edit();
                    self.push_snapshot();
                    let mut tx = TextBox::new(local, self.ink_color());
                    tx.size = [420.0, 48.0];
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
        }

        if tool == Tool::Image && tap {
            self.pending_image = Some((page, local));
        }
    }

    fn clear_shape_hold(&mut self) {
        self.shape_anchor = None;
        self.shape_still = None;
        self.shape_preview = None;
    }

    /// After one second still, the free stroke snaps to a line, square, or circle.
    fn consider_shape(&mut self, tip: Pos2) {
        if self.live.is_none() {
            return;
        }
        let still_r = 14.0 / self.camera.zoom.max(0.15);
        let anchor = self.shape_anchor.unwrap_or(tip);
        if anchor.distance(tip) > still_r {
            self.shape_anchor = Some(tip);
            self.shape_still = Some(Instant::now());
            self.shape_preview = None;
            return;
        }
        if self.shape_preview.is_some() {
            return;
        }
        let since = *self.shape_still.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_millis(1000) {
            return;
        }
        let Some((_, stroke)) = &self.live else {
            return;
        };
        self.shape_preview = maybe_snap_shape(stroke);
    }

    fn finish_live(&mut self) {
        self.live_from_pen = false;
        let preview = self.shape_preview.take();
        self.shape_anchor = None;
        self.shape_still = None;
        if let Some((p, mut s)) = self.live.take() {
            if let Some(snapped) = preview {
                s = snapped;
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
            let o = self.origin_of(s.page);
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
            let o = page_origin(
                page.col,
                page.row,
                n.page_w.max(1.0),
                n.page_h.max(1.0),
                n.sheet_join.gap(),
            );
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
            .set_title("Place an image")
            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
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
            let origin = page_origin(
                page.col,
                page.row,
                note.page_w,
                note.page_h,
                note.sheet_join.gap(),
            );
            let min = map(Pos2::new(origin.x, origin.y));
            let max = map(Pos2::new(origin.x + note.page_w, origin.y + note.page_h));
            let paper = Rect::from_min_max(min, max);
            let fused = note.sheet_join == SheetJoin::Linked && note.pages.len() > 1;
            let n_left = fused && note.unit_occupied(page.col - 1, page.row);
            let n_right = fused && note.unit_occupied(page.col + 1, page.row);
            let n_top = fused && note.unit_occupied(page.col, page.row - 1);
            let n_bot = fused && note.unit_occupied(page.col, page.row + 1);
            let rad = 5;
            let sheet_r = if fused {
                CornerRadius {
                    nw: if n_left || n_top { 0 } else { rad },
                    ne: if n_right || n_top { 0 } else { rad },
                    sw: if n_left || n_bot { 0 } else { rad },
                    se: if n_right || n_bot { 0 } else { rad },
                }
            } else {
                CornerRadius::same(rad)
            };
            let shadow = if fused {
                let mut sh = paper;
                if !n_right {
                    sh.max.x += 3.0;
                }
                if !n_bot {
                    sh.max.y += 4.0;
                }
                sh
            } else {
                paper.translate(vec2(3.0, 4.0))
            };
            painter.rect_filled(shadow, sheet_r, self.look.shadow);
            let fill = if note.paper == PaperKind::Slate {
                self.look.desk_deep
            } else {
                self.look.paper
            };
            painter.rect_filled(paper, sheet_r, fill);
            self.paint_template(painter, paper, note.paper, cam.zoom);
            if note.paper == PaperKind::Lined && !n_left {
                self.paint_punches(painter, paper, cam.zoom);
            }
            if note.paper != PaperKind::Slate
                && !n_right
                && !n_top
                && !note.can_tear_unit()
            {
                paint_page_fold(painter, paper, cam.zoom, self.look.paper_rule_strong);
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

        if let Some(pi) = self.live.as_ref().map(|(pi, _)| *pi) {
            let origin = self.origin_of(pi);
            let mesh = if self.shape_preview.is_some() {
                self.shape_preview.as_mut().unwrap().tessellate().clone()
            } else {
                self.live.as_mut().unwrap().1.tessellate().clone()
            };
            painter.add(Shape::mesh(map_mesh(&mesh, |p| map(p + origin))));
        }
        if self.lasso.len() >= 2 {
            let pts: Vec<_> = self.lasso.iter().copied().map(map).collect();
            painter.add(Shape::closed_line(pts, Stroke::new(1.2, self.look.accent)));
        }
        for s in &self.sel {
            if let Some(n) = &self.note {
                if let Some(page) = n.pages.get(s.page) {
                    let o = self.origin_of(s.page);
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

    fn unit_screen_rect(&self, canvas: Rect, col: i32, row: i32) -> Rect {
        let o = page_origin(col, row, PAGE_W, PAGE_H, self.gap());
        let min = self.camera.to_screen(Pos2::new(o.x, o.y), canvas);
        let max = self
            .camera
            .to_screen(Pos2::new(o.x + PAGE_W, o.y + PAGE_H), canvas);
        Rect::from_min_max(min, max)
    }

    fn sheet_tabs(&self, canvas: Rect) -> Vec<(i32, i32, Rect)> {
        let Some(n) = &self.note else {
            return Vec::new();
        };
        if canvas.width() < 10.0 {
            return Vec::new();
        }
        n.unit_tabs()
            .into_iter()
            .filter_map(|t| {
                let hit = if t.center {
                    let hole = self.unit_screen_rect(canvas, t.dest_col, t.dest_row);
                    if hole.width() < 8.0 || hole.height() < 8.0 {
                        return None;
                    }
                    center_band(hole)
                } else {
                    let edge = match (t.dcol, t.drow) {
                        (-1, 0) => SheetEdge::Left,
                        (1, 0) => SheetEdge::Right,
                        (0, -1) => SheetEdge::Top,
                        (0, 1) => SheetEdge::Bottom,
                        _ => return None,
                    };
                    let paper = self.unit_screen_rect(canvas, t.src_col, t.src_row);
                    if paper.width() < 8.0 || paper.height() < 8.0 {
                        return None;
                    }
                    outward_band(paper, edge)
                };
                Some((t.dest_col, t.dest_row, hit))
            })
            .collect()
    }

    fn sheet_tab_at(&self, screen: Pos2, canvas: Rect) -> Option<(i32, i32)> {
        if self.note.is_none() || canvas.width() < 10.0 {
            return None;
        }
        self.sheet_tabs(canvas)
            .into_iter()
            .filter(|(_, _, r)| r.contains(screen))
            .min_by(|a, b| {
                let da = a.2.center().distance(screen);
                let db = b.2.center().distance(screen);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(c, r, _)| (c, r))
    }

    fn add_sheet_from_tab(&mut self, col: i32, row: i32) {
        if self
            .note
            .as_ref()
            .is_none_or(|n| n.unit_occupied(col, row))
        {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.push_snapshot();
        if let Some(n) = &mut self.note {
            n.add_unit_at(col, row);
        }
        self.sel.clear();
        self.lasso.clear();
        self.mark_dirty();
    }

    fn paint_sheet_tabs(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        if self.note.is_none() || canvas.width() < 10.0 {
            return;
        }
        let hover = ui.input(|i| i.pointer.hover_pos());
        let (wash, plus, wash_hot, plus_hot) = sheet_tab_colors(self.look.desk);
        for (_, _, hit) in self.sheet_tabs(canvas) {
            if !hit.is_positive() {
                continue;
            }
            let hot = hover.is_some_and(|p| hit.contains(p));
            paint_sheet_tab(
                painter,
                hit,
                if hot { wash_hot } else { wash },
                if hot { plus_hot } else { plus },
            );
        }
    }

    fn page_folds(&self, canvas: Rect) -> Vec<(i32, i32, Rect)> {
        let Some(n) = &self.note else {
            return Vec::new();
        };
        if !n.can_tear_unit() || canvas.width() < 10.0 {
            return Vec::new();
        }
        let z = self.camera.zoom;
        n.unit_cells()
            .into_iter()
            .filter_map(|(col, row)| {
                let paper = self.unit_screen_rect(canvas, col, row);
                if paper.width() < 8.0 || paper.height() < 8.0 {
                    return None;
                }
                Some((col, row, fold_hit_rect(paper, z)))
            })
            .collect()
    }

    fn fold_at(&self, screen: Pos2, canvas: Rect) -> Option<(i32, i32)> {
        self.page_folds(canvas)
            .into_iter()
            .filter(|(_, _, r)| r.contains(screen))
            .min_by(|a, b| {
                let da = a.2.center().distance(screen);
                let db = b.2.center().distance(screen);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(c, r, _)| (c, r))
    }

    fn tear_sheet_at(&mut self, col: i32, row: i32) {
        if self
            .note
            .as_ref()
            .is_none_or(|n| !n.can_tear_unit() || !n.unit_occupied(col, row))
        {
            return;
        }
        self.finish_live();
        self.finish_text_edit();
        self.clear_shape_hold();
        self.push_snapshot();
        if let Some(n) = &mut self.note {
            n.remove_unit(col, row);
        }
        self.sel.clear();
        self.lasso.clear();
        self.editing_text = None;
        self.mark_dirty();
    }

    fn paint_page_folds(&self, ui: &Ui, painter: &Painter, canvas: Rect) {
        let Some(n) = &self.note else {
            return;
        };
        if !n.can_tear_unit() {
            return;
        }
        let hover = ui.input(|i| i.pointer.hover_pos());
        let z = self.camera.zoom;
        let rest = self.look.paper_rule_strong;
        let hot_c = self.look.rust();
        for (col, row, hit) in self.page_folds(canvas) {
            let paper = self.unit_screen_rect(canvas, col, row);
            let hot = hover.is_some_and(|p| hit.contains(p));
            paint_page_fold(painter, paper, z, if hot { hot_c } else { rest });
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

    fn paint_punches(&self, painter: &Painter, paper: Rect, zoom: f32) {
        let z = zoom.clamp(0.35, 3.0);
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
        let want_focus = self.text_focus;
        let mut took_focus = false;
        if let Some((pg, id)) = self.editing_text {
            let cam = self.camera;
            if let Some(note) = &mut self.note {
                let (pw, ph) = note.page_size();
                let gap = note.sheet_join.gap();
                if let Some(page) = note.pages.get_mut(pg) {
                    let origin = page_origin(page.col, page.row, pw, ph, gap);
                    if let Some(tx) = page.texts.iter_mut().find(|t| t.id == id) {
                        let min = cam.to_screen(tx.min() + origin, rect);
                        let size = vec2(tx.size[0] * cam.zoom, tx.size[1] * cam.zoom);
                        let size_pt = tx.size_pt;
                        let col = tx.color32();
                        Area::new(Id::new(("tx", id)))
                            .fixed_pos(min)
                            .order(Order::Foreground)
                            .constrain(false)
                            .show(ui.ctx(), |ui| {
                                ui.set_min_size(size);
                                let te = TextEdit::multiline(&mut tx.text)
                                    .id(Id::new(("tx-edit", id)))
                                    .font(FontId::new(
                                        size_pt * cam.zoom,
                                        FontFamily::Name("serif".into()),
                                    ))
                                    .text_color(col)
                                    .desired_width(size.x)
                                    .desired_rows(2)
                                    .lock_focus(true)
                                    .frame(false);
                                let r = ui.add(te);
                                if r.changed() {
                                    edited = true;
                                }
                                if want_focus {
                                    r.request_focus();
                                    took_focus = true;
                                }
                            });
                    }
                }
            }
        }
        if took_focus {
            self.text_focus = false;
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
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
            if self.sheet_tab_at(p, self.canvas_rect).is_some()
                || self.fold_at(p, self.canvas_rect).is_some()
            {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                return;
            }
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
                || d.get_temp::<Rect>(Id::new("tin-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
                || d.get_temp::<Rect>(Id::new("strip-rect"))
                    .map(|r| r.expand(4.0).contains(pos))
                    .unwrap_or(false)
        })
    }

    /// Stylus to egui pointer: hover and clicks on the chrome (pencil case, ruler, shelf).
    fn inject_pen_pointer(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        let pen = self.tablet.snapshot();
        if pen.in_proximity || pen.down {
            if let Some(pos) = pen.pos {
                raw.events.push(Event::PointerMoved(pos));
                // On the shelf, every click. On the lectern, chrome only,
                // and only when the press began on the chrome, so a stroke
                // drifting onto the pencil case stays ink.
                if pen.pressed {
                    self.pen_ui_grab = match self.scene {
                        Scene::Shelf { .. } => true,
                        Scene::Desk => Self::pen_over_chrome(ctx, pos),
                    };
                }
                if !pen.down && !pen.pressed {
                    self.pen_ui_grab = false;
                }
                let ui_click = match self.scene {
                    Scene::Shelf { .. } => true,
                    Scene::Desk => self.pen_ui_grab,
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
                        self.pen_ui_grab = false;
                    }
                } else if pen.released {
                    self.pen_ui_grab = false;
                }
            }
            self.pen_was_prox = true;
        } else if self.pen_was_prox {
            raw.events.push(Event::PointerGone);
            self.pen_was_prox = false;
            self.pen_ui_grab = false;
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

fn same_rgb(a: Color32, b: Color32) -> bool {
    a.r().abs_diff(b.r()) < 8 && a.g().abs_diff(b.g()) < 8 && a.b().abs_diff(b.b()) < 8
}

fn rgb_to_hsv(c: Color32) -> (f32, f32, f32) {
    let r = c.r() as f32 / 255.0;
    let g = c.g() as f32 / 255.0;
    let b = c.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-5 {
        0.0
    } else if (max - r).abs() < 1e-5 {
        (60.0 * ((g - b) / d) + 360.0) % 360.0
    } else if (max - g).abs() < 1e-5 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max < 1e-5 { 0.0 } else { d / max };
    (h / 360.0, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color32 {
    let h = ((h % 1.0) + 1.0) % 1.0 * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

fn paint_sv_field(p: &Painter, rect: Rect, hue: f32) {
    let nx = 18;
    let ny = 12;
    let cw = rect.width() / nx as f32;
    let ch = rect.height() / ny as f32;
    for y in 0..ny {
        for x in 0..nx {
            let s = (x as f32 + 0.5) / nx as f32;
            let v = 1.0 - (y as f32 + 0.5) / ny as f32;
            let cell = Rect::from_min_size(
                pos2(rect.min.x + x as f32 * cw, rect.min.y + y as f32 * ch),
                vec2(cw + 0.4, ch + 0.4),
            );
            p.rect_filled(cell, 0, hsv_to_rgb(hue, s, v));
        }
    }
    p.rect_stroke(
        rect,
        4,
        Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
        StrokeKind::Inside,
    );
}

fn paint_hue_bar(p: &Painter, rect: Rect) {
    let n = 48;
    let w = rect.width() / n as f32;
    for i in 0..n {
        let x = rect.min.x + i as f32 * w;
        p.rect_filled(
            Rect::from_min_size(pos2(x, rect.min.y), vec2(w + 0.5, rect.height())),
            0,
            hsv_to_rgb(i as f32 / n as f32, 0.95, 0.95),
        );
    }
    p.rect_stroke(
        rect,
        3,
        Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
        StrokeKind::Inside,
    );
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

fn well_glyph(well: Color32, paper: Color32, ink: Color32) -> Color32 {
    let lum = 0.299 * well.r() as f32 + 0.587 * well.g() as f32 + 0.114 * well.b() as f32;
    if lum > 155.0 {
        ink
    } else {
        paper
    }
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

const WHEEL_IN: f32 = 56.0;
const WHEEL_OUT: f32 = 152.0;
const WHEEL_HOLD: f64 = 0.35;
const COLOR_HOLD: f64 = 0.35;
const PAPER_TILE: f32 = 168.0;

fn wheel_hit(origin: Pos2, pos: Pos2, colors: bool, n_cloth: usize) -> Option<WheelPick> {
    let v = pos - origin;
    let r = v.length();
    if r < WHEEL_IN || r > WHEEL_OUT + 48.0 {
        return None;
    }
    let a = (v.angle() + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
    if colors {
        let n = n_cloth.max(1);
        let i = ((a / std::f32::consts::TAU) * n as f32).floor() as usize % n;
        return Some(WheelPick::Cloth(i as u8));
    }
    let i = ((a / std::f32::consts::TAU) * 4.0).floor() as usize % 4;
    Some(
        [
            WheelPick::Pin,
            WheelPick::Color,
            WheelPick::Trash,
            WheelPick::Mark,
        ][i],
    )
}

fn slice_mid(origin: Pos2, i: usize, n: usize, r: f32) -> Pos2 {
    let a = -std::f32::consts::FRAC_PI_2
        + (i as f32 + 0.5) * std::f32::consts::TAU / n.max(1) as f32;
    origin + Vec2::angled(a) * r
}

fn wedge_span(n: usize, i: usize) -> (f32, f32) {
    let n = n.max(1) as f32;
    let span = std::f32::consts::TAU / n;
    let pad = (3.2 / WHEEL_OUT).min(span * 0.12);
    let a0 = -std::f32::consts::FRAC_PI_2 + i as f32 * span + pad;
    let a1 = -std::f32::consts::FRAC_PI_2 + (i as f32 + 1.0) * span - pad;
    (a0, a1)
}

fn wedge_mesh(
    origin: Pos2,
    r0: f32,
    r1: f32,
    n: usize,
    i: usize,
    color: Color32,
    paper: Option<(TextureId, f32)>,
) -> Shape {
    let (a0, a1) = wedge_span(n, i);
    let steps = 12;
    let mut mesh = Mesh::default();
    if let Some((id, _)) = paper {
        mesh.texture_id = id;
    }
    let vert = |p: Pos2| {
        let uv = if let Some((_, tile)) = paper {
            pos2(p.x / tile, p.y / tile)
        } else {
            Pos2::ZERO
        };
        Vertex { pos: p, uv, color }
    };
    for s in 0..steps {
        let t0 = s as f32 / steps as f32;
        let t1 = (s + 1) as f32 / steps as f32;
        let u0 = a0 + (a1 - a0) * t0;
        let u1 = a0 + (a1 - a0) * t1;
        let i0 = mesh.vertices.len() as u32;
        mesh.vertices.extend([
            vert(origin + Vec2::angled(u0) * r0),
            vert(origin + Vec2::angled(u0) * r1),
            vert(origin + Vec2::angled(u1) * r1),
            vert(origin + Vec2::angled(u1) * r0),
        ]);
        mesh.indices
            .extend([i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
    }
    Shape::mesh(mesh)
}

fn wedge_stroke(origin: Pos2, r0: f32, r1: f32, n: usize, i: usize, color: Color32) -> Shape {
    let (a0, a1) = wedge_span(n, i);
    let steps = 12;
    let mut pts = Vec::with_capacity(steps * 2 + 2);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let a = a0 + (a1 - a0) * t;
        pts.push(origin + Vec2::angled(a) * r1);
    }
    for s in (0..=steps).rev() {
        let t = s as f32 / steps as f32;
        let a = a0 + (a1 - a0) * t;
        pts.push(origin + Vec2::angled(a) * r0);
    }
    Shape::closed_line(pts, Stroke::new(2.0_f32, color))
}

/// 50% of the edge, centered. A bit shorter in thickness, set off from the sheet.
fn outward_band(paper: Rect, edge: SheetEdge) -> Rect {
    let gap = paper.height() * 0.10;
    let thick = paper.height() * 0.22;
    let along_h = paper.height() * 0.50;
    let along_w = paper.width() * 0.50;
    let cy = paper.center().y;
    let cx = paper.center().x;
    match edge {
        SheetEdge::Left => Rect::from_min_max(
            pos2(paper.min.x - gap - thick, cy - along_h * 0.5),
            pos2(paper.min.x - gap, cy + along_h * 0.5),
        ),
        SheetEdge::Right => Rect::from_min_max(
            pos2(paper.max.x + gap, cy - along_h * 0.5),
            pos2(paper.max.x + gap + thick, cy + along_h * 0.5),
        ),
        SheetEdge::Top => Rect::from_min_max(
            pos2(cx - along_w * 0.5, paper.min.y - gap - thick),
            pos2(cx + along_w * 0.5, paper.min.y - gap),
        ),
        SheetEdge::Bottom => Rect::from_min_max(
            pos2(cx - along_w * 0.5, paper.max.y + gap),
            pos2(cx + along_w * 0.5, paper.max.y + gap + thick),
        ),
    }
}

fn center_band(paper: Rect) -> Rect {
    let w = paper.width() * 0.42;
    let h = paper.height() * 0.28;
    Rect::from_center_size(paper.center(), vec2(w, h))
}

fn sheet_tab_colors(desk: Color32) -> (Color32, Color32, Color32, Color32) {
    let lum = desk.r() as u16 + desk.g() as u16 + desk.b() as u16;
    if lum < 420 {
        (
            Color32::from_white_alpha(26),
            Color32::from_white_alpha(210),
            Color32::from_white_alpha(42),
            Color32::from_white_alpha(240),
        )
    } else {
        (
            Color32::from_black_alpha(12),
            Color32::from_black_alpha(70),
            Color32::from_black_alpha(20),
            Color32::from_black_alpha(110),
        )
    }
}

fn fold_hit_rect(paper: Rect, zoom: f32) -> Rect {
    let s = (24.0 * zoom).clamp(20.0, 52.0);
    Rect::from_min_max(
        pos2(paper.max.x - s, paper.min.y),
        pos2(paper.max.x, paper.min.y + s),
    )
}

fn paint_page_fold(p: &Painter, paper: Rect, zoom: f32, color: Color32) {
    let s = 16.0 * zoom;
    let fold = [
        pos2(paper.max.x, paper.min.y),
        pos2(paper.max.x - s, paper.min.y),
        pos2(paper.max.x, paper.min.y + s),
    ];
    p.add(Shape::convex_polygon(fold.to_vec(), color, Stroke::NONE));
}

fn paint_sheet_tab(p: &Painter, rect: Rect, wash: Color32, plus: Color32) {
    let short = rect.width().min(rect.height());
    let radius = short * 0.10;
    p.rect_filled(rect, radius, wash);
    let c = rect.center();
    let arm = short * 0.18;
    let thick = (short * 0.018).max(1.0);
    p.rect_filled(
        Rect::from_center_size(c, vec2(arm * 2.0, thick)),
        thick * 0.5,
        plus,
    );
    p.rect_filled(
        Rect::from_center_size(c, vec2(thick, arm * 2.0)),
        thick * 0.5,
        plus,
    );
}

fn paint_plus(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_filled(Rect::from_center_size(c, vec2(16.0, 3.0)), 2.0, fg);
    p.rect_filled(Rect::from_center_size(c, vec2(3.0, 16.0)), 2.0, fg);
}

fn paint_more(p: &egui::Painter, c: Pos2, fg: Color32) {
    for dy in [-7.4, 0.0, 7.4] {
        p.circle_filled(pos2(c.x, c.y + dy), 2.1, fg);
    }
}

fn paint_fit(p: &egui::Painter, c: Pos2, fg: Color32) {
    let st = Stroke::new(2.0_f32, fg);
    let s = 7.6;
    let m = 2.6;
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
    p.circle_stroke(c, 9.8, Stroke::new(1.95_f32, fg));
    p.rect_filled(Rect::from_center_size(c, vec2(10.6, 2.4)), 1.2, fg);
    p.rect_filled(Rect::from_center_size(c, vec2(2.4, 10.6)), 1.2, fg);
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

fn paint_text_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    let wire = Stroke::new(1.55_f32, fg);
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

fn paint_image_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    let wire = Stroke::new(1.55_f32, fg);
    let hair = Stroke::new(1.35_f32, fg);
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

fn paint_zoom_out(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.circle_stroke(c, 9.8, Stroke::new(1.95_f32, fg));
    p.rect_filled(Rect::from_center_size(c, vec2(10.6, 2.4)), 1.2, fg);
}

fn paint_paper_icon(p: &egui::Painter, c: Pos2, fg: Color32) {
    p.rect_stroke(
        Rect::from_center_size(c, vec2(14.5, 18.0)),
        4.0,
        Stroke::new(1.8_f32, fg),
        StrokeKind::Inside,
    );
    for dy in [-3.8, 0.0, 3.8] {
        p.line_segment(
            [pos2(c.x - 4.2, c.y + dy), pos2(c.x + 4.2, c.y + dy)],
            Stroke::new(1.55_f32, fg),
        );
    }
}


