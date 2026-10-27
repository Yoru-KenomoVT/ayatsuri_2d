// Linux on Wayland: take the pen through the tablet protocol, on the
// window's own connection, as Qt (and with it Krita) does. Without it a
// compositor falls back to passing the pen on as a pointer, and Hyprland's
// fallback (0.56) sends its motion without frames and none of its clicks,
// so an app that does not use the protocol cannot be clicked with a pen.
//
// Once bound, the compositor sends this connection the pen's own events:
// where it is, the tip going down and up, its pressure, its buttons. Each
// frame of them becomes pointer events the editor hands to egui (the tip
// as the primary button, the barrel buttons as secondary and middle), and
// the pressure goes to the same readings the tablet reader fills. The
// pen's cursor is set through the cursor-shape protocol, as nothing else
// sets one for a tablet tool.
//
// The connection is winit's: events arrive on it whoever reads the socket,
// sorted into this module's own queue, which a thread here dispatches. The
// one unsafe call attaches to that connection from its display pointer.
#![allow(unsafe_code)]

use std::os::raw::c_void;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_backend::client::Backend;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, event_created_child};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2::ZwpTabletManagerV2,
    zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
    zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2,
    zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2,
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
    zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
    zwp_tablet_v2::ZwpTabletV2,
};

use crate::{PenButton, PenPointer, push_pointer, store_down, store_eraser, store_pressure, wake};

/// Linux button codes for a pen's barrel buttons.
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;
const BTN_STYLUS3: u32 = 0x149;

/// How long the reader waits on the socket before it checks for stop.
const POLL: Duration = Duration::from_millis(50);

static STOP: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Attach to a Wayland display (the window's) and start taking the pen
/// through the tablet protocol. False when the compositor has no tablet
/// protocol or the reader could not start.
pub(crate) fn attach(display: NonNull<c_void>) -> bool {
    // SAFETY: the pointer is the live wl_display of the window's own
    // connection (winit's, through raw-window-handle), which stays open for
    // as long as the app runs; the reader is stopped before the window goes
    // (`stop`), and a backend made from a foreign display never closes it.
    let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
    let conn = Connection::from_backend(backend);
    let (globals, queue) = match registry_queue_init::<State>(&conn) {
        Ok(g) => g,
        Err(e) => {
            log::warn!("pen tablet (Wayland): could not list the compositor's globals: {e}");
            return false;
        }
    };
    let qh = queue.handle();
    let manager: ZwpTabletManagerV2 = match globals.bind(&qh, 1..=1, ()) {
        Ok(m) => m,
        Err(_) => {
            log::info!("pen tablet (Wayland): the compositor has no tablet protocol");
            return false;
        }
    };
    let seat: WlSeat = match globals.bind(&qh, 1..=7, ()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pen tablet (Wayland): no seat: {e}");
            return false;
        }
    };
    let shapes: Option<WpCursorShapeManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
    let tablet_seat = manager.get_tablet_seat(&seat, &qh, ());
    let state = State { shapes, tools: Vec::new(), _keep: (manager, seat, tablet_seat) };
    let spawned = std::thread::Builder::new()
        .name("pen-wayland".into())
        .spawn(move || run(conn, queue, state));
    match spawned {
        Ok(handle) => {
            if let Ok(mut t) = THREAD.lock() {
                *t = Some(handle);
            }
            log::info!("pen tablet (Wayland): taking the pen through the tablet protocol");
            true
        }
        Err(e) => {
            log::warn!("pen tablet (Wayland): could not start the reader: {e}");
            false
        }
    }
}

/// Stop the reader and wait for it, before the window's connection closes.
pub(crate) fn stop() {
    STOP.store(true, Ordering::Release);
    let handle = THREAD.lock().ok().and_then(|mut t| t.take());
    if let Some(h) = handle {
        let _ = h.join();
    }
}

fn run(conn: Connection, mut queue: EventQueue<State>, mut state: State) {
    let timeout = Timespec { tv_sec: 0, tv_nsec: POLL.as_nanos() as i64 };
    while !STOP.load(Ordering::Acquire) {
        if queue.dispatch_pending(&mut state).is_err() || conn.flush().is_err() {
            break;
        }
        // None: events are already waiting for this queue
        let Some(guard) = queue.prepare_read() else { continue };
        let ready = {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            poll(&mut fds, Some(&timeout)).unwrap_or(0) > 0
        };
        if ready {
            if guard.read().is_err() {
                break;
            }
        } else {
            drop(guard);
        }
    }
    log::info!("pen tablet (Wayland): stopped");
}

/// One pen (a tablet tool), between its frames.
struct Tool {
    tool: ZwpTabletToolV2,
    cursor: Option<WpCursorShapeDeviceV1>,
    /// Where it is (surface pixels) and what is down, as last sent.
    at: Option<(f32, f32)>,
    down: bool,
    buttons: Vec<u32>,
    /// What this frame changed.
    moved: Option<(f32, f32)>,
    tip: Option<bool>,
    pressed: Vec<(u32, bool)>,
    left: bool,
    eraser: bool,
}

struct State {
    shapes: Option<WpCursorShapeManagerV1>,
    tools: Vec<Tool>,
    /// The bound objects, kept alive for as long as the reader runs.
    _keep: (ZwpTabletManagerV2, WlSeat, ZwpTabletSeatV2),
}

impl State {
    fn tool(&mut self, t: &ZwpTabletToolV2) -> Option<&mut Tool> {
        self.tools.iter_mut().find(|x| x.tool == *t)
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Objects whose events are not needed here.
macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(
                _: &mut Self,
                _: &$t,
                _: <$t as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore!(
    WlSeat,
    ZwpTabletManagerV2,
    ZwpTabletV2,
    ZwpTabletPadRingV2,
    ZwpTabletPadStripV2,
    WpCursorShapeManagerV1,
    WpCursorShapeDeviceV1
);

impl Dispatch<ZwpTabletSeatV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpTabletSeatV2,
        event: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_tablet_seat_v2::Event::ToolAdded { id } = event {
            state.tools.push(Tool {
                tool: id,
                cursor: None,
                at: None,
                down: false,
                buttons: Vec::new(),
                moved: None,
                tip: None,
                pressed: Vec::new(),
                left: false,
                eraser: false,
            });
        }
    }

    event_created_child!(State, ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadV2,
        _: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadGroupV2,
        _: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
    ]);
}

impl Dispatch<ZwpTabletToolV2, ()> for State {
    fn event(
        state: &mut Self,
        tool: &ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use zwp_tablet_tool_v2::Event as E;
        if let E::Removed = event {
            state.tools.retain(|x| x.tool != *tool);
            return;
        }
        let shapes = state.shapes.clone();
        let Some(t) = state.tool(tool) else { return };
        match event {
            E::Type { tool_type } => {
                t.eraser = matches!(
                    tool_type,
                    wayland_client::WEnum::Value(zwp_tablet_tool_v2::Type::Eraser)
                );
            }
            E::ProximityIn { serial, .. } => {
                // nothing else gives a tablet tool a cursor: the default one
                match (&shapes, &t.cursor) {
                    (Some(m), None) => {
                        let device = m.get_tablet_tool_v2(tool, qh, ());
                        device.set_shape(serial, Shape::Default);
                        t.cursor = Some(device);
                    }
                    (_, Some(device)) => device.set_shape(serial, Shape::Default),
                    (None, None) => {}
                }
                store_eraser(t.eraser);
            }
            E::ProximityOut => t.left = true,
            E::Motion { x, y } => t.moved = Some((x as f32, y as f32)),
            E::Down { .. } => t.tip = Some(true),
            E::Up => t.tip = Some(false),
            E::Pressure { pressure } => store_pressure(pressure as f32 / 65535.0),
            E::Button { button, state: s, .. } => {
                let pressed = matches!(
                    s,
                    wayland_client::WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed)
                );
                t.pressed.push((button, pressed));
            }
            E::Frame { .. } => frame(t),
            _ => {}
        }
    }
}

/// A frame of one pen's events, as pointer events in the order a mouse
/// would give them: where it is first, then its buttons, then leaving.
fn frame(t: &mut Tool) {
    let mut out = Vec::new();
    if let Some((x, y)) = t.moved.take() {
        t.at = Some((x, y));
        out.push(PenPointer::Move { x, y });
    }
    if let Some(down) = t.tip.take()
        && down != t.down
    {
        t.down = down;
        store_down(down);
        out.push(PenPointer::Button { button: PenButton::Primary, pressed: down });
    }
    for (code, pressed) in t.pressed.drain(..) {
        let button = match code {
            BTN_STYLUS => PenButton::Secondary,
            BTN_STYLUS2 | BTN_STYLUS3 => PenButton::Middle,
            _ => continue,
        };
        let was = t.buttons.contains(&code);
        if pressed && !was {
            t.buttons.push(code);
        } else if !pressed && was {
            t.buttons.retain(|&c| c != code);
        } else {
            continue;
        }
        out.push(PenPointer::Button { button, pressed });
    }
    if std::mem::take(&mut t.left) {
        // a pen leaving with something held lets go of it first
        if std::mem::take(&mut t.down) {
            store_down(false);
            out.push(PenPointer::Button { button: PenButton::Primary, pressed: false });
        }
        for code in t.buttons.drain(..) {
            let button = if code == BTN_STYLUS { PenButton::Secondary } else { PenButton::Middle };
            out.push(PenPointer::Button { button, pressed: false });
        }
        t.at = None;
        out.push(PenPointer::Leave);
    }
    if !out.is_empty() {
        push_pointer(out);
        wake();
    }
}
