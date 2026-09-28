use super::*;

struct Fixture {
    app: CahierApp,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cahier-ui-{}", Uuid::new_v4()));
        let lib = Library {
            root: root.clone(),
            index: Default::default(),
        };
        Self {
            app: CahierApp::with_library(Look::load(), lib),
            root,
        }
    }

    fn notebook(&mut self, name: &str) -> Uuid {
        let note = Note::blank(name, 0);
        let id = note.id;
        self.app.lib.insert_new(&note);
        id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root).expect("remove isolated test library");
        }
    }
}

struct Frames {
    ctx: Context,
    time: f64,
    textures: HashMap<TextureId, ColorImage>,
    output: Option<FullOutput>,
    size: Vec2,
}

impl Frames {
    fn new(app: &CahierApp) -> Self {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        app.look.apply(&ctx);
        Self {
            ctx,
            time: 0.0,
            textures: HashMap::new(),
            output: None,
            size: vec2(1280.0, 860.0),
        }
    }

    fn step(&mut self, app: &mut CahierApp, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let output = self.ctx.run(input, |ctx| {
            app.shortcuts(ctx);
            match app.scene {
                Scene::Shelf { .. } => app.ui_shelf(ctx),
                Scene::Desk => app.ui_desk(ctx),
            }
        });
        for (id, delta) in &output.textures_delta.set {
            let ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let target = self.textures.get_mut(id).expect("texture allocated");
                for row in 0..image.height() {
                    let start = (y + row) * target.width() + x;
                    target.pixels[start..start + image.width()].copy_from_slice(
                        &image.pixels[row * image.width()..(row + 1) * image.width()],
                    );
                }
            } else {
                self.textures.insert(*id, (**image).clone());
            }
        }
        self.output = Some(output);
    }

    fn settle(&mut self, app: &mut CahierApp) {
        for _ in 0..20 {
            self.step(app, vec![]);
        }
    }

    fn drag(&mut self, app: &mut CahierApp, from: Pos2, to: Pos2) {
        self.step(app, vec![Event::PointerMoved(from)]);
        self.step(
            app,
            vec![Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
        );
        self.step(app, vec![Event::PointerMoved(to)]);
        self.settle(app);
        self.step(
            app,
            vec![Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }

    fn key(&mut self, app: &mut CahierApp, key: Key) {
        for pressed in [true, false] {
            self.step(
                app,
                vec![Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
    }

    fn touch(&mut self, app: &mut CahierApp, phase: TouchPhase, pos: Pos2) {
        let mut events = vec![
            Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(1),
                phase,
                pos,
                force: None,
            },
            Event::PointerMoved(pos),
        ];
        if phase != TouchPhase::Move {
            events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: phase == TouchPhase::Start,
                modifiers: Modifiers::NONE,
            });
        }
        if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
            events.push(Event::PointerGone);
        }
        self.step(app, events);
    }

    // Optional CPU snapshots of the actual egui meshes, without opening a desktop window.
    fn snapshot(&self, name: &str) {
        let Some(directory) = std::env::var_os("CAHIER_TEST_ARTIFACTS") else {
            return;
        };
        let output = self.output.as_ref().unwrap();
        let scale = output.pixels_per_point;
        let mut image =
            image::RgbaImage::new((self.size.x * scale) as u32, (self.size.y * scale) as u32);
        for primitive in self.ctx.tessellate(output.shapes.clone(), scale) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                continue;
            };
            let texture = &self.textures[&mesh.texture_id];
            let clip = primitive.clip_rect;
            for indices in mesh.indices.chunks_exact(3) {
                let v = [
                    mesh.vertices[indices[0] as usize],
                    mesh.vertices[indices[1] as usize],
                    mesh.vertices[indices[2] as usize],
                ];
                let points = v.map(|v| (v.pos.to_vec2() * scale).to_pos2());
                let cross = |a: Vec2, b: Vec2| a.x * b.y - a.y * b.x;
                let area = cross(points[1] - points[0], points[2] - points[0]);
                if area.abs() < 0.00001 {
                    continue;
                }
                let bounds = Rect::from_points(&points).intersect(Rect::from_min_max(
                    (clip.min.to_vec2() * scale).to_pos2(),
                    (clip.max.to_vec2() * scale).to_pos2(),
                ));
                let colors = v.map(|v| v.color.to_array());
                for y in bounds.min.y.max(0.0).floor() as u32
                    ..(bounds.max.y.ceil() as u32).min(image.height())
                {
                    for x in bounds.min.x.max(0.0).floor() as u32
                        ..(bounds.max.x.ceil() as u32).min(image.width())
                    {
                        let point = pos2(x as f32 + 0.5, y as f32 + 0.5);
                        let a = cross(points[1] - point, points[2] - point) / area;
                        let b = cross(points[2] - point, points[0] - point) / area;
                        let c = 1.0 - a - b;
                        if a < 0.0 || b < 0.0 || c < 0.0 {
                            continue;
                        }
                        let uv =
                            v[0].uv.to_vec2() * a + v[1].uv.to_vec2() * b + v[2].uv.to_vec2() * c;
                        let tx =
                            ((uv.x * texture.width() as f32) as usize).min(texture.width() - 1);
                        let ty =
                            ((uv.y * texture.height() as f32) as usize).min(texture.height() - 1);
                        let texel = texture.pixels[ty * texture.width() + tx].to_array();
                        let source: [f32; 4] = std::array::from_fn(|i| {
                            (colors[0][i] as f32 * a
                                + colors[1][i] as f32 * b
                                + colors[2][i] as f32 * c)
                                * texel[i] as f32
                                / 255.0
                        });
                        let target = image.get_pixel_mut(x, y);
                        for i in 0..4 {
                            target.0[i] = (source[i]
                                + target.0[i] as f32 * (1.0 - source[3] / 255.0))
                                .round()
                                .clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        image.save(directory.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn fitted_canvas_tracks_same_page_through_resizes() {
    let mut fixture = Fixture::new();
    let app = &mut fixture.app;
    let mut note = Note::blank("Fit", 0);
    note.pages.push(crate::document::Page::at(1, 0));
    app.note = Some(note);
    app.need_fit = false;
    app.canvas_rect = Rect::from_min_size(pos2(0.0, 64.0), vec2(1000.0, 800.0));
    app.camera
        .fit_page(app.canvas_rect, app.origin_of(1), app.pw(), app.ph());
    app.fit_to_screen();
    for size in [vec2(800.0, 480.0), vec2(1800.0, 900.0), vec2(820.0, 1200.0)] {
        let rect = Rect::from_min_size(pos2(18.0, 80.0), size);
        assert!(app.update_canvas_rect(rect));
        assert_eq!(app.fitted_cell, Some((1, 0)));
        assert_eq!(app.page_in_view(rect), 1);
        assert_eq!(app.zoom_percent(), 100);
        assert_eq!(app.note.as_ref().unwrap().page_size(), (PAGE_W, PAGE_H));
    }
}

#[test]
fn manual_canvas_keeps_focus_through_minimize_and_reopen() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Manual");
    let app = &mut fixture.app;
    app.open_note(id);
    app.update_canvas_rect(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    app.fit_to_screen();
    app.update_canvas_rect(app.canvas_rect);
    app.set_zoom_level(2.0);
    assert_eq!(app.fitted_cell, None);
    let zoom = app.camera.zoom;
    let focus = app
        .camera
        .to_paper(app.canvas_rect.center(), app.canvas_rect);
    for size in [
        vec2(800.0, 1000.0),
        Vec2::ZERO,
        vec2(1300.0, 2.0),
        vec2(1600.0, 700.0),
    ] {
        let previous = app.canvas_rect;
        let rect = Rect::from_min_size(pos2(20.0, 80.0), size);
        let valid = app.update_canvas_rect(rect);
        assert_eq!(valid, Camera::valid_viewport(rect));
        if !valid {
            assert_eq!(app.canvas_rect, previous);
        }
        assert_eq!(app.camera.zoom, zoom);
        assert!(
            app.camera
                .to_paper(app.canvas_rect.center(), app.canvas_rect)
                .distance(focus)
                < 0.001
        );
    }
    app.close_desk();
    app.open_note(id);
    assert_eq!(app.canvas_rect, Rect::ZERO);
    assert_eq!(app.fitted_cell, None);
    assert_eq!(app.land_page, Some(0));
}

#[test]
fn manual_scroll_leaves_fitted_mode() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Scroll");
    let app = &mut fixture.app;
    app.open_note(id);
    app.fit_to_screen();
    let mut frames = Frames::new(app);
    frames.settle(app);
    assert_eq!(app.fitted_cell, Some((0, 0)));
    frames.snapshot("page-fit-wide");
    frames.size = vec2(820.0, 620.0);
    frames.settle(app);
    assert_eq!(app.zoom_percent(), 100);
    frames.snapshot("page-fit-small");
    let center = app.canvas_rect.center();
    frames.step(app, vec![Event::PointerMoved(center)]);
    frames.step(
        app,
        vec![Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -30.0),
            modifiers: Modifiers::NONE,
        }],
    );
    assert_eq!(app.fitted_cell, None);
    let zoom = app.camera.zoom;
    let focus = app
        .camera
        .to_paper(app.canvas_rect.center(), app.canvas_rect);
    frames.size = vec2(1000.0, 1000.0);
    frames.settle(app);
    assert_eq!(app.camera.zoom, zoom);
    assert!(
        app.camera
            .to_paper(app.canvas_rect.center(), app.canvas_rect)
            .distance(focus)
            < 0.001
    );
    frames.snapshot("page-manual-tall");
}

#[test]
fn corner_drop_reopens_smoothly_and_targets_bottom_right() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Drop");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.settle(app);
    for _ in 0..2 {
        app.shelf_haul_ids = vec![id];
        app.shelf_haul_armed = true;
        frames.step(app, vec![]);
        assert!(app.trash_reveal > 0.0 && app.trash_reveal < 1.0);
        frames.settle(app);
        assert_eq!(app.trash_reveal, 1.0);
        frames.snapshot("corner-drop");
        let screen = Rect::from_min_size(Pos2::ZERO, frames.size);
        assert!(app.trash_rect.min.x >= screen.right() - BIN_DROP_RADIUS);
        assert!(app.over_trash_drop_zone(screen.right_bottom() - vec2(75.0, 75.0), screen, 0.0));
        assert!(!app.over_trash_drop_zone(screen.left_bottom() + vec2(75.0, -75.0), screen, 0.0));
        assert!(!app.over_trash_drop_zone(screen.right_bottom() - vec2(170.0, 170.0), screen, 0.0));
        app.shelf_haul_armed = false;
        frames.step(app, vec![]);
        assert!(app.trash_reveal > 0.0 && app.trash_reveal < 1.0);
        frames.settle(app);
        assert_eq!(app.trash_reveal, 0.0);
    }
}

#[test]
fn drawer_grip_click_closes_without_deleting_and_can_reopen() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Keep in trash");
    let app = &mut fixture.app;
    app.lib.trash_note(id);
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    frames.snapshot("bin-drawer-grip");
    let height = app.bin_panel_height;
    let grip = app.bin_handle_rect.center();
    assert!(app.bin_handle_rect.height() >= 44.0);
    frames.drag(app, grip, grip);
    assert!(!app.shelf_trash);
    assert!(!app.shelf_band_armed);
    assert!(app.lib.is_trashed(id));
    frames.step(app, vec![]);
    assert!(app.bin_panel_height > 0.0 && app.bin_panel_height < height);
    frames.snapshot("bin-drawer-closing");
    frames.settle(app);
    assert_eq!(app.bin_panel_height, 0.0);
    assert!(app.trash_grid.is_empty());
    let basket = app.trash_rect.center();
    frames.drag(app, basket, basket);
    assert!(app.shelf_trash);
    frames.settle(app);
    assert_eq!(app.bin_panel_height, height);
    assert!(app.lib.is_trashed(id));
}

#[test]
fn drawer_grip_touch_tap_and_downward_swipe_close() {
    for distance in [0.0, 80.0] {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.shelf_trash = true;
        let mut frames = Frames::new(app);
        frames.settle(app);
        let height = app.bin_panel_height;
        let origin = app.bin_handle_rect.center();
        frames.touch(app, TouchPhase::Start, origin);
        assert_eq!(app.bin_handle_drag.map(|(pos, _)| pos), Some(origin));
        assert!(app.shelf_band.is_none());
        let target = origin + vec2(0.0, distance);
        if distance > 0.0 {
            frames.touch(app, TouchPhase::Move, target);
            assert!((app.bin_panel_height - (height - distance)).abs() < 1.0);
            frames.snapshot("bin-drawer-touch-drag");
        }
        frames.touch(app, TouchPhase::End, target);
        assert!(!app.shelf_trash);
        frames.settle(app);
        assert_eq!(app.bin_panel_height, 0.0);
        assert!(app.bin_handle_drag.is_none());
        assert_eq!(app.trash_reveal, 0.0);
    }
}

#[test]
fn drawer_grip_short_sideways_upward_and_cancelled_swipes_stay_open() {
    for (delta, phase) in [
        (vec2(0.0, 24.0), TouchPhase::End),
        (vec2(110.0, 70.0), TouchPhase::End),
        (vec2(0.0, -80.0), TouchPhase::End),
        (vec2(0.0, 90.0), TouchPhase::Cancel),
    ] {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.shelf_trash = true;
        let mut frames = Frames::new(app);
        frames.settle(app);
        let height = app.bin_panel_height;
        let origin = app.bin_handle_rect.center();
        frames.touch(app, TouchPhase::Start, origin);
        frames.touch(app, TouchPhase::Move, origin + delta);
        frames.touch(app, phase, origin + delta);
        frames.settle(app);
        assert!(app.shelf_trash, "gesture {delta:?}, {phase:?}");
        assert_eq!(app.bin_panel_height, height);
        assert!(app.bin_handle_drag.is_none());
        assert!(app.shelf_band.is_none());
        assert!(!app.shelf_haul_armed);
    }
}

#[test]
fn drawer_keyboard_close_respects_modal_priority_and_selected_notebooks() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Selected");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    app.shelf_trash = true;
    frames.settle(app);
    app.emoji_pick = Some(id);
    frames.key(app, Key::Escape);
    assert!(app.emoji_pick.is_none());
    assert!(app.shelf_trash);
    for key in [Key::Escape, Key::Enter, Key::Space] {
        app.shelf_trash = true;
        app.shelf_sel = vec![id];
        frames.settle(app);
        if key == Key::Enter {
            for _ in 0..32 {
                frames.key(app, Key::Tab);
                if frames
                    .ctx
                    .memory(|m| m.has_focus(Id::new("bin-drawer-handle")))
                {
                    break;
                }
            }
            assert!(
                frames
                    .ctx
                    .memory(|m| m.has_focus(Id::new("bin-drawer-handle")))
            );
        } else if key == Key::Space {
            frames
                .ctx
                .memory_mut(|m| m.request_focus(Id::new("bin-drawer-handle")));
        }
        frames.key(app, key);
        assert!(!app.shelf_trash, "{key:?} should close the drawer");
        assert!(matches!(app.scene, Scene::Shelf { .. }));
        assert!(!app.lib.is_trashed(id));
        frames.settle(app);
        assert_eq!(app.bin_panel_height, 0.0);
    }
}

#[test]
fn drawer_grip_second_finger_cancels_without_turning_release_into_click() {
    let mut fixture = Fixture::new();
    let app = &mut fixture.app;
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let origin = app.bin_handle_rect.center();
    frames.touch(app, TouchPhase::Start, origin);
    for phase in [TouchPhase::Start, TouchPhase::End] {
        frames.step(
            app,
            vec![Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(2),
                phase,
                pos: origin + vec2(40.0, 0.0),
                force: None,
            }],
        );
        assert!(app.bin_handle_drag.is_none());
    }
    frames.touch(app, TouchPhase::End, origin);
    assert!(app.shelf_trash);
    frames.settle(app);
    assert_eq!(app.bin_panel_height, CahierApp::shelf_slot().y + 20.0);
}

#[test]
fn notebook_drag_over_grip_never_dismisses_drawer() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Move past grip");
    let app = &mut fixture.app;
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let grip = app.bin_handle_rect.center();
    frames.drag(app, source, grip);
    assert!(app.shelf_trash);
    assert!(!app.lib.is_trashed(id));
    assert!(app.bin_handle_drag.is_none());
    frames.drag(app, source, grip + vec2(0.0, 80.0));
    assert!(app.shelf_trash);
    assert!(app.lib.is_trashed(id));
}

#[test]
fn management_drag_moves_between_home_and_bin_slots() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Move");
    let app = &mut fixture.app;
    app.shelf_trash = true;
    let mut frames = Frames::new(app);
    frames.settle(app);
    frames.snapshot("bin-management-empty");
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let bin = app.trash_grid[1].center();
    frames.drag(app, source, bin);
    assert_eq!(
        app.lib
            .index
            .trash
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .slot,
        1
    );
    assert_eq!(app.trash_grid.len(), TRASH_SLOTS as usize);
    assert_eq!(app.trash_reveal, 0.0);
    frames.snapshot("bin-management-full");
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let bin = app.trash_grid[2].center();
    frames.drag(app, source, bin);
    assert_eq!(
        app.lib
            .index
            .trash
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .slot,
        2
    );
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let home = app.shelf_grid[1].center();
    frames.drag(app, source, home);
    assert!(!app.lib.is_trashed(id));
    assert_eq!(
        app.lib
            .index
            .notes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .slot,
        1
    );
}

#[test]
fn home_drop_uses_corner_until_toss_completes() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Throw");
    let app = &mut fixture.app;
    let mut frames = Frames::new(app);
    frames.settle(app);
    let source = app
        .shelf_slots
        .iter()
        .find(|(note, _)| *note == id)
        .unwrap()
        .1
        .center();
    let target = frames.size.to_pos2() - vec2(75.0, 75.0);
    frames.drag(app, source, target);
    assert_eq!(app.shelf_toss.len(), 1);
    assert!(!app.lib.is_trashed(id));
    assert_eq!(app.trash_reveal, 1.0);
    for _ in 0..10 {
        frames.step(app, vec![]);
    }
    assert_eq!(app.shelf_toss.len(), 1);
    assert_eq!(app.trash_reveal, 1.0);
    frames.snapshot("corner-toss");
    for _ in 0..40 {
        frames.step(app, vec![]);
    }
    assert!(app.shelf_toss.is_empty());
    assert!(app.lib.is_trashed(id));
    assert_eq!(app.trash_reveal, 0.0);
}

#[test]
fn linked_canvas_keeps_selected_page_when_resized() {
    let mut fixture = Fixture::new();
    let id = fixture.notebook("Linked");
    let app = &mut fixture.app;
    app.open_note(id);
    let note = app.note.as_mut().unwrap();
    note.pages.push(crate::document::Page::at(1, 0));
    note.paper = PaperKind::Grid;
    let mut frames = Frames::new(app);
    frames.settle(app);
    app.camera
        .fit_page(app.canvas_rect, app.origin_of(1), app.pw(), app.ph());
    app.fit_to_screen();
    for (name, size) in [
        ("linked-wide", vec2(1400.0, 740.0)),
        ("linked-tall", vec2(820.0, 1050.0)),
    ] {
        frames.size = size;
        frames.settle(app);
        assert_eq!(app.fitted_cell, Some((1, 0)));
        assert_eq!(app.page_in_view(app.canvas_rect), 1);
        assert_eq!(app.zoom_percent(), 100);
        frames.snapshot(name);
    }
}
