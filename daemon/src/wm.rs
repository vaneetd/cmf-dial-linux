//! Windows and workspaces through COSMIC's own Wayland protocols.
//!
//! - `ext_workspace_v1`: the workspace list, which one is active, and activating one.
//! - `ext_foreign_toplevel_list_v1` + `zcosmic_toplevel_info_v1`: every window with its app id,
//!   title and state (activated, minimized).
//! - `zcosmic_toplevel_manager_v1`: bring a window forward.
//!
//! A thread owns the Wayland connection and keeps a `Snapshot` up to date behind a mutex. Commands
//! go in over a calloop channel. Window order is most recently active first, tracked here because
//! the compositor doesn't report it.

use std::sync::{Arc, Mutex};

use cosmic_protocols::toplevel_info::v1::client::{
    zcosmic_toplevel_handle_v1::{self, ZcosmicToplevelHandleV1},
    zcosmic_toplevel_info_v1::{self, ZcosmicToplevelInfoV1},
};
use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1::{self, ZcosmicToplevelManagerV1};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Sender};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::{GlobalListContents, registry_queue_init};
use smithay_client_toolkit::reexports::client::protocol::{wl_registry, wl_seat};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, Proxy, QueueHandle, event_created_child,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};

use crate::event::log;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// The Wayland object id of the foreign handle, stable while the window lives.
    pub id: u32,
    pub app_id: String,
    pub title: String,
    pub minimized: bool,
    pub activated: bool,
    /// Higher = more recently active.
    pub recency: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Most recently active first.
    pub windows: Vec<Window>,
    pub workspaces: Vec<WorkspaceInfo>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceInfo {
    pub name: String,
    pub active: bool,
}

impl Snapshot {
    pub fn active_workspace(&self) -> Option<usize> {
        self.workspaces.iter().position(|w| w.active)
    }
}

enum Cmd {
    Activate(u32),
    Workspace(usize),
}

#[derive(Clone)]
pub struct Wm {
    shared: Arc<Mutex<Snapshot>>,
    tx: Sender<Cmd>,
}

impl Wm {
    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Bring a window forward (and unminimize it).
    pub fn activate(&self, window: u32) {
        let _ = self.tx.send(Cmd::Activate(window));
    }

    /// Switch to the workspace at this index.
    pub fn go_to_workspace(&self, index: usize) {
        let _ = self.tx.send(Cmd::Workspace(index));
    }
}

pub fn spawn() -> Wm {
    let shared = Arc::new(Mutex::new(Snapshot::default()));
    let (tx, rx) = channel::channel();
    let thread_shared = shared.clone();
    std::thread::Builder::new()
        .name("dial-wm".into())
        .spawn(move || {
            if let Err(e) = run(thread_shared, rx) {
                log("wm", format!("stopped: {e}"));
            }
        })
        .expect("spawn wm thread");
    Wm { shared, tx }
}

struct Toplevel {
    foreign: ExtForeignToplevelHandleV1,
    cosmic: Option<ZcosmicToplevelHandleV1>,
    app_id: String,
    title: String,
    minimized: bool,
    activated: bool,
    recency: u64,
}

struct Workspace {
    handle: ExtWorkspaceHandleV1,
    name: String,
    active: bool,
    hidden: bool,
}

struct State {
    shared: Arc<Mutex<Snapshot>>,
    info: ZcosmicToplevelInfoV1,
    manager: ZcosmicToplevelManagerV1,
    seat: wl_seat::WlSeat,
    ws_manager: ExtWorkspaceManagerV1,
    toplevels: Vec<Toplevel>,
    workspaces: Vec<Workspace>,
    counter: u64,
}

fn run(shared: Arc<Mutex<Snapshot>>, rx: Channel<Cmd>) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
    let qh = event_queue.handle();

    let info = globals.bind::<ZcosmicToplevelInfoV1, _, _>(&qh, 2..=3, ()).map_err(|e| format!("zcosmic_toplevel_info_v1: {e}"))?;
    let manager = globals.bind::<ZcosmicToplevelManagerV1, _, _>(&qh, 1..=4, ()).map_err(|e| format!("zcosmic_toplevel_manager_v1: {e}"))?;
    let seat = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=7, ()).map_err(|e| format!("wl_seat: {e}"))?;
    let ws_manager = globals.bind::<ExtWorkspaceManagerV1, _, _>(&qh, 1..=1, ()).map_err(|e| format!("ext_workspace_manager_v1: {e}"))?;
    let _list = globals.bind::<ExtForeignToplevelListV1, _, _>(&qh, 1..=1, ()).map_err(|e| format!("ext_foreign_toplevel_list_v1: {e}"))?;

    let mut state = State { shared, info, manager, seat, ws_manager, toplevels: Vec::new(), workspaces: Vec::new(), counter: 0 };

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle())?;
    event_loop
        .handle()
        .insert_source(rx, |event, _, state| {
            if let channel::Event::Msg(cmd) = event {
                state.command(cmd);
            }
        })
        .map_err(|e| format!("channel: {e}"))?;
    log("wm", "connected (workspaces, toplevel info and management)");
    loop {
        event_loop.dispatch(None, &mut state)?;
        if let Some(err) = conn.protocol_error() {
            return Err(format!("wayland protocol error: {err}").into());
        }
    }
}

impl State {
    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Activate(id) => {
                let Some(t) = self.toplevels.iter().find(|t| t.foreign.id().protocol_id() == id) else {
                    log("wm", format!("no window {id} to activate"));
                    return;
                };
                if let Some(cosmic) = &t.cosmic {
                    if t.minimized {
                        self.manager.unset_minimized(cosmic);
                    }
                    self.manager.activate(cosmic, &self.seat);
                }
            }
            Cmd::Workspace(index) => {
                let visible: Vec<&Workspace> = self.workspaces.iter().filter(|w| !w.hidden).collect();
                if let Some(w) = visible.get(index) {
                    w.handle.activate();
                    self.ws_manager.commit();
                } else {
                    log("wm", format!("no workspace at index {index}"));
                }
            }
        }
    }

    fn publish(&self) {
        let mut windows: Vec<Window> = self
            .toplevels
            .iter()
            .filter(|t| t.cosmic.is_some() && !t.app_id.is_empty())
            .map(|t| Window {
                id: t.foreign.id().protocol_id(),
                app_id: t.app_id.clone(),
                title: t.title.clone(),
                minimized: t.minimized,
                activated: t.activated,
                recency: t.recency,
            })
            .collect();
        windows.sort_by(|a, b| b.recency.cmp(&a.recency));
        let workspaces = self
            .workspaces
            .iter()
            .filter(|w| !w.hidden)
            .map(|w| WorkspaceInfo { name: w.name.clone(), active: w.active })
            .collect();
        if let Ok(mut s) = self.shared.lock() {
            *s = Snapshot { windows, workspaces };
        }
    }

    fn toplevel_mut(&mut self, foreign_id: u32) -> Option<&mut Toplevel> {
        self.toplevels.iter_mut().find(|t| t.foreign.id().protocol_id() == foreign_id)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZcosmicToplevelManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZcosmicToplevelManagerV1, _: zcosmic_toplevel_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZcosmicToplevelInfoV1, ()> for State {
    fn event(state: &mut Self, _: &ZcosmicToplevelInfoV1, event: zcosmic_toplevel_info_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zcosmic_toplevel_info_v1::Event::Done = event {
            state.publish();
        }
    }

    event_created_child!(State, ZcosmicToplevelInfoV1, [
        zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (ZcosmicToplevelHandleV1, u32::MAX),
    ]);
}

/// User data is the id of the matching foreign handle.
impl Dispatch<ZcosmicToplevelHandleV1, u32> for State {
    fn event(state: &mut Self, _: &ZcosmicToplevelHandleV1, event: zcosmic_toplevel_handle_v1::Event, foreign_id: &u32, _: &Connection, _: &QueueHandle<Self>) {
        use zcosmic_toplevel_handle_v1::Event;
        match event {
            Event::State { state: bytes } => {
                state.counter += 1;
                let counter = state.counter;
                let Some(t) = state.toplevel_mut(*foreign_id) else { return };
                let states: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_ne_bytes(c.try_into().unwrap())).collect();
                t.minimized = states.contains(&1);
                let activated = states.contains(&2);
                if activated && !t.activated {
                    t.recency = counter;
                }
                t.activated = activated;
                state.publish();
            }
            Event::Closed => {
                state.toplevels.retain(|t| t.foreign.id().protocol_id() != *foreign_id);
                state.publish();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(state: &mut Self, _: &ExtForeignToplevelListV1, event: ext_foreign_toplevel_list_v1::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            let id = toplevel.id().protocol_id();
            let cosmic = state.info.get_cosmic_toplevel(&toplevel, qh, id);
            state.toplevels.push(Toplevel {
                foreign: toplevel,
                cosmic: Some(cosmic),
                app_id: String::new(),
                title: String::new(),
                minimized: false,
                activated: false,
                recency: 0,
            });
        }
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(state: &mut Self, handle: &ExtForeignToplevelHandleV1, event: ext_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use ext_foreign_toplevel_handle_v1::Event;
        let id = handle.id().protocol_id();
        match event {
            Event::Title { title } => {
                if let Some(t) = state.toplevel_mut(id) {
                    t.title = title;
                }
            }
            Event::AppId { app_id } => {
                if let Some(t) = state.toplevel_mut(id) {
                    t.app_id = app_id;
                }
            }
            Event::Done => state.publish(),
            Event::Closed => {
                state.toplevels.retain(|t| t.foreign.id().protocol_id() != id);
                state.publish();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    fn event(state: &mut Self, _: &ExtWorkspaceManagerV1, event: ext_workspace_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                state.workspaces.push(Workspace { handle: workspace, name: String::new(), active: false, hidden: false });
            }
            ext_workspace_manager_v1::Event::Done => state.publish(),
            _ => {}
        }
    }

    event_created_child!(State, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for State {
    fn event(_: &mut Self, _: &ExtWorkspaceGroupHandleV1, _: ext_workspace_group_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for State {
    fn event(state: &mut Self, handle: &ExtWorkspaceHandleV1, event: ext_workspace_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use ext_workspace_handle_v1::Event;
        match event {
            Event::Name { name } => {
                if let Some(w) = state.workspaces.iter_mut().find(|w| &w.handle == handle) {
                    w.name = name;
                }
            }
            Event::State { state: bits } => {
                if let Some(w) = state.workspaces.iter_mut().find(|w| &w.handle == handle) {
                    let bits: u32 = bits.into();
                    w.active = bits & 1 != 0;
                    w.hidden = bits & 4 != 0;
                }
            }
            Event::Removed => {
                state.workspaces.retain(|w| &w.handle != handle);
            }
            _ => {}
        }
    }
}
