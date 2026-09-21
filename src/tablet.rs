//! Pont Wayland `zwp_tablet_manager_v2` → encre.
//!
//! winit 0.30 n’écoute pas le protocole tablette. Sous Hyprland le stylet
//! (Galaxy Book / Wacom AES) n’apparaît donc jamais comme souris. On se greffe
//! sur le `wl_display` d’eframe (guest) et on lit tip / motion / pression.

use std::collections::HashSet;

use egui::Pos2;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use wayland_backend::client::Backend;
use wayland_client::protocol::{
    wl_registry::{self, WlRegistry},
    wl_seat::{self, WlSeat},
};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2::{self, ZwpTabletManagerV2},
    zwp_tablet_pad_dial_v2::{self, ZwpTabletPadDialV2},
    zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
    zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
    zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
    zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
    zwp_tablet_v2::{self, ZwpTabletV2},
};

const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

#[derive(Clone, Debug, Default)]
pub struct PenSnapshot {
    pub pos: Option<Pos2>,
    pub samples: Vec<Pos2>,
    pub down: bool,
    pub pressed: bool,
    pub released: bool,
    pub eraser: bool,
    pub pressure: Option<f32>,
    pub in_proximity: bool,
}

pub struct TabletBridge {
    inner: Option<Inner>,
    dead: bool,
}

struct Inner {
    conn: Connection,
    queue: EventQueue<TabletState>,
    state: TabletState,
}

#[derive(Default)]
struct TabletState {
    #[allow(dead_code)]
    registry: Option<WlRegistry>,
    manager: Option<ZwpTabletManagerV2>,
    seats: Vec<(u32, WlSeat)>,
    #[allow(dead_code)]
    tablet_seats: Vec<ZwpTabletSeatV2>,
    bound_seats: HashSet<u32>,
    pos: Option<Pos2>,
    samples: Vec<Pos2>,
    down: bool,
    pressed: bool,
    released: bool,
    in_proximity: bool,
    eraser_tool: bool,
    stylus_btn: bool,
    pressure: Option<f32>,
}

impl TabletBridge {
    pub fn new() -> Self {
        Self {
            inner: None,
            dead: false,
        }
    }

    pub fn pump(&mut self, frame: &eframe::Frame) {
        if self.dead {
            return;
        }
        if self.inner.is_none() {
            match unsafe { attach(frame) } {
                Ok(inner) => self.inner = Some(inner),
                Err(()) => {
                    self.dead = true;
                    return;
                }
            }
        }
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        inner.state.pressed = false;
        inner.state.released = false;
        inner.state.samples.clear();
        let _ = inner.conn.flush();
        if inner.queue.dispatch_pending(&mut inner.state).is_err() {
            self.dead = true;
            self.inner = None;
            return;
        }
        let _ = inner.conn.flush();
    }

    pub fn snapshot(&self) -> PenSnapshot {
        let Some(inner) = self.inner.as_ref() else {
            return PenSnapshot::default();
        };
        let s = &inner.state;
        PenSnapshot {
            pos: s.pos,
            samples: s.samples.clone(),
            down: s.down,
            pressed: s.pressed,
            released: s.released,
            eraser: s.eraser_tool || s.stylus_btn,
            pressure: s.pressure,
            in_proximity: s.in_proximity,
        }
    }

    pub fn wants_repaint(&self) -> bool {
        if self.dead {
            return false;
        }
        match &self.inner {
            None => true,
            Some(i) => {
                i.state.in_proximity || i.state.down || i.state.manager.is_none()
            }
        }
    }
}

unsafe fn attach(frame: &eframe::Frame) -> Result<Inner, ()> {
    let handle = frame.display_handle().map_err(|_| ())?;
    let RawDisplayHandle::Wayland(w) = handle.as_raw() else {
        return Err(());
    };
    let backend = unsafe { Backend::from_foreign_display(w.display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let mut queue: EventQueue<TabletState> = conn.new_event_queue();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut state = TabletState {
        registry: Some(registry),
        ..Default::default()
    };
    let _ = conn.flush();
    let _ = queue.dispatch_pending(&mut state);
    Ok(Inner { conn, queue, state })
}

impl TabletState {
    fn bind_tablet_seats(&mut self, qh: &QueueHandle<Self>) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        for (name, seat) in &self.seats {
            if !self.bound_seats.insert(*name) {
                continue;
            }
            self.tablet_seats
                .push(manager.get_tablet_seat(seat, qh, ()));
        }
    }
}

fn tool_pos(x: f64, y: f64) -> Pos2 {
    Pos2::new(x as f32, y as f32)
}

impl Dispatch<WlRegistry, ()> for TabletState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    let seat: WlSeat = registry.bind(name, version.min(1), qh, ());
                    state.seats.push((name, seat));
                    state.bind_tablet_seats(qh);
                }
                "zwp_tablet_manager_v2" | "wp_tablet_manager_v2" => {
                    if state.manager.is_none() {
                        state.manager =
                            Some(registry.bind(name, version.min(2), qh, ()));
                        state.bind_tablet_seats(qh);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpTabletManagerV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpTabletManagerV2,
        event: zwp_tablet_manager_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

impl Dispatch<ZwpTabletSeatV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpTabletSeatV2,
        _: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(TabletState, ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<ZwpTabletV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        tablet: &ZwpTabletV2,
        event: zwp_tablet_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zwp_tablet_v2::Event::Removed) {
            tablet.destroy();
        }
    }
}

impl Dispatch<ZwpTabletToolV2, ()> for TabletState {
    fn event(
        state: &mut Self,
        tool: &ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_tool_v2::Event::Type { tool_type } => {
                state.eraser_tool = matches!(
                    tool_type,
                    WEnum::Value(zwp_tablet_tool_v2::Type::Eraser)
                );
            }
            zwp_tablet_tool_v2::Event::ProximityIn { .. } => {
                state.in_proximity = true;
            }
            zwp_tablet_tool_v2::Event::ProximityOut => {
                if state.down {
                    state.down = false;
                    state.released = true;
                }
                state.in_proximity = false;
                state.stylus_btn = false;
                state.pressure = None;
            }
            zwp_tablet_tool_v2::Event::Down { .. } => {
                state.down = true;
                state.pressed = true;
            }
            zwp_tablet_tool_v2::Event::Up => {
                state.down = false;
                state.released = true;
            }
            zwp_tablet_tool_v2::Event::Motion { x, y } => {
                let p = tool_pos(x, y);
                state.pos = Some(p);
                if state.down {
                    state.samples.push(p);
                }
            }
            zwp_tablet_tool_v2::Event::Pressure { pressure } => {
                let p = (pressure as f32 / 65535.0).clamp(0.0, 1.0);
                if p > 0.02 {
                    state.pressure = Some(p);
                }
            }
            zwp_tablet_tool_v2::Event::Button { button, state: st, .. } => {
                let pressed = matches!(st, WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed));
                if button == BTN_STYLUS || button == BTN_STYLUS2 {
                    state.stylus_btn = pressed;
                }
            }
            zwp_tablet_tool_v2::Event::Removed => {
                if state.down {
                    state.down = false;
                    state.released = true;
                }
                state.in_proximity = false;
                tool.destroy();
            }
            _ => {}
        }
    }
}

macro_rules! noop_dispatch {
    ($iface:ty, $ev:path) => {
        impl Dispatch<$iface, ()> for TabletState {
            fn event(
                _: &mut Self,
                _: &$iface,
                event: $ev,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                let _ = event;
            }
        }
    };
}

impl Dispatch<ZwpTabletPadV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        pad: &ZwpTabletPadV2,
        event: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, zwp_tablet_pad_v2::Event::Removed) {
            pad.destroy();
        }
    }

    event_created_child!(TabletState, ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for TabletState {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadGroupV2,
        event: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }

    event_created_child!(TabletState, ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
        zwp_tablet_pad_group_v2::EVT_DIAL_OPCODE => (ZwpTabletPadDialV2, ()),
    ]);
}

noop_dispatch!(ZwpTabletPadRingV2, zwp_tablet_pad_ring_v2::Event);
noop_dispatch!(ZwpTabletPadStripV2, zwp_tablet_pad_strip_v2::Event);
noop_dispatch!(ZwpTabletPadDialV2, zwp_tablet_pad_dial_v2::Event);
