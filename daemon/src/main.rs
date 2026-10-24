//! dial-daemon: the Smart Dial on a CMF Buds Pro 2 case as events for the desktop.
//!
//! Capture, decode, state machine, mode actions (uinput keys and scroll, logind brightness) and the
//! overlay (wheel and HUD). Apps and Windows are not built yet.
//!
//!   --no-buds   don't touch BlueZ or evdev; drive it from stdin (r l p d, +3, -2)
//!   --no-ui     no overlay, log only

mod actions;
mod encoder;
mod event;
mod machine;
mod sources;
mod ui;
mod wm;

use std::time::Instant;

use tokio::sync::mpsc;
use zbus::Connection;
use zbus::OwnedObjectPath;

use actions::Actions;
use encoder::Encoder;
use event::{DialEvent, ModeId, log};
use machine::{Effect, Machine, Phase};
use sources::Raw;
use ui::{UiHandle, UiMsg};

struct Daemon {
    conn: Connection,
    machine: Machine,
    encoder: Encoder,
    actions: Actions,
    ui: Option<UiHandle>,
    transport: Option<OwnedObjectPath>,
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let dir = args.get(i + 1).map_or("snapshots", |s| s.as_str());
        return match ui::snapshot(std::path::Path::new(dir)) {
            Ok(()) => Ok(println!("wrote snapshots to {dir}")),
            Err(e) => Ok(eprintln!("snapshot failed: {e}")),
        };
    }
    if has("--wm-list") {
        let wm = wm::spawn();
        std::thread::sleep(std::time::Duration::from_millis(800));
        let snap = wm.snapshot();
        for w in &snap.windows {
            println!("window {} app={:?} title={:?} min={} active={} recency={}", w.id, w.app_id, w.title, w.minimized, w.activated, w.recency);
        }
        for (i, w) in snap.workspaces.iter().enumerate() {
            println!("workspace {i} name={:?} active={}", w.name, w.active);
        }
        return Ok(());
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Raw>();

    log("daemon", "dial-daemon starting. Lid must be CLOSED for the dial to send anything. Ctrl-C to stop.");
    let conn = Connection::system().await?;
    if !has("--no-buds") {
        tokio::spawn({
            let (conn, tx) = (conn.clone(), tx.clone());
            async move {
                if let Err(e) = sources::bluez(conn, tx).await {
                    log("bluez", format!("stopped: {e}"));
                }
            }
        });
        tokio::spawn(sources::avrcp_keys(tx.clone()));
    }
    tokio::spawn(sources::simulated(tx.clone()));

    let mut d = Daemon {
        machine: Machine::new(),
        encoder: Encoder::default(),
        actions: Actions::new(conn.clone(), wm::spawn()),
        ui: (!has("--no-ui")).then(ui::spawn),
        transport: None,
        conn,
    };

    loop {
        let sleep = async {
            match d.machine.deadline() {
                Some(t) => tokio::time::sleep_until(t.into()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sleep => {
                let before = d.machine.describe();
                let fx = d.machine.tick(Instant::now());
                if before != d.machine.describe() {
                    log("state", format!("{before} → {} (idle)", d.machine.describe()));
                }
                d.apply(fx).await;
            }
            Some(raw) = rx.recv() => d.on_raw(raw).await,
        }
    }
    log("daemon", "bye");
    Ok(())
}

impl Daemon {
    async fn on_raw(&mut self, raw: Raw) {
        let now = Instant::now();
        match raw {
            Raw::TransportUp(path, volume) => {
                self.encoder.baseline(volume);
                self.transport = Some(path);
                // A reconnect while parked must park again.
                if self.machine.should_park() {
                    self.encoder.set_parking(false, now);
                    if let Some(v) = self.encoder.set_parking(true, now) {
                        self.write(v).await;
                    }
                }
            }
            Raw::TransportDown(path) => {
                if self.transport.as_ref() == Some(&path) {
                    self.transport = None;
                    self.encoder.transport_lost();
                }
            }
            Raw::TransportVolume(v) => {
                let decoded = self.encoder.report(v, now);
                if let Some(w) = decoded.write {
                    self.write(w).await;
                }
                match decoded.steps {
                    Some(steps) => {
                        // At rest and in Volume mode the buds moved the volume themselves.
                        if !self.machine.should_park() {
                            self.hud(ModeId::Volume, format!("{}", (v as u32 * 100 + 63) / 127), Some(v as f32 / 127.0));
                        }
                        self.dispatch(DialEvent::Turn(steps), "bluez").await;
                    }
                    None => log("encoder", format!("volume {v}: echo or no change")),
                }
            }
            Raw::Dial(e, src) => self.dispatch(e, src).await,
        }
    }

    async fn dispatch(&mut self, event: DialEvent, source: &str) {
        log("event", format!("{event} from {source}  [state: {}]", self.machine.describe()));
        if event == DialEvent::Press && self.machine.phase() == Phase::Picking {
            self.ui_send(UiMsg::Press);
        }
        let before = self.machine.describe();
        let fx = self.machine.handle(event, Instant::now());
        if before != self.machine.describe() {
            log("state", format!("{before} → {}", self.machine.describe()));
        }
        self.apply(fx).await;
    }

    async fn apply(&mut self, fx: Vec<Effect>) {
        for effect in fx {
            match effect {
                Effect::SetParking(on) => {
                    log("park", if on { "parking at midpoint" } else { "parking off, volume free" });
                    if let Some(v) = self.encoder.set_parking(on, Instant::now()) {
                        self.write(v).await;
                    }
                }
                Effect::Wheel(Some(m)) => {
                    log("wheel", format!("open, selected {}", m.label()));
                    self.ui_send(UiMsg::Wheel(Some(m)));
                }
                Effect::Wheel(None) => {
                    log("wheel", "closed");
                    self.ui_send(UiMsg::Wheel(None));
                }
                Effect::Activate(m) => {
                    let text = self.actions.activate(m).await;
                    log("mode", format!("{}: activate → {text}", m.label()));
                    if m == ModeId::Volume {
                        if let Some(v) = self.encoder.last_volume() {
                            self.hud(m, format!("{}", (v as u32 * 100 + 63) / 127), Some(v as f32 / 127.0));
                        }
                    } else {
                        self.hud(m, text, None);
                    }
                }
                Effect::Deactivate(m) => {
                    self.actions.deactivate(m);
                    log("mode", format!("{}: deactivate", m.label()));
                }
                Effect::ModeTurn(m, n) => {
                    let text = self.actions.turn(m, n).await;
                    log("mode", format!("{}: turn {n:+} → {text}", m.label()));
                    if m != ModeId::Volume {
                        self.hud(m, text, None);
                    }
                }
                Effect::ModePress(m) => {
                    let text = self.actions.press(m).await;
                    log("mode", format!("{}: press → {text}", m.label()));
                    self.hud(m, text, None);
                }
            }
        }
    }

    /// Sends the HUD pill. Level modes give "95%" as text; that becomes a meter and a number.
    fn hud(&self, mode: ModeId, text: String, level: Option<f32>) {
        let (text, level) = match (level, text.strip_suffix('%').and_then(|n| n.parse::<f32>().ok())) {
            (None, Some(p)) => (format!("{p:.0}"), Some(p / 100.0)),
            _ => (text, level),
        };
        self.ui_send(UiMsg::Hud { mode, text, level });
    }

    fn ui_send(&self, msg: UiMsg) {
        if let Some(ui) = &self.ui {
            ui.send(msg);
        }
    }

    async fn write(&self, value: u16) {
        let Some(path) = self.transport.as_ref() else {
            log("park", "no transport to write to");
            return;
        };
        match sources::write_transport_volume(&self.conn, path, value).await {
            Ok(()) => log("park", format!("wrote MediaTransport1.Volume={value}")),
            Err(e) => log("park", format!("write failed: {e}")),
        }
    }
}
