//! dial-daemon: the Smart Dial on a CMF Buds Pro 2 case as events for the desktop.
//!
//! Phase 1 scope: capture, decode, state machine. Mode actions only log what they would do.
//!
//!   --no-buds   don't touch BlueZ or evdev; drive it from stdin (r l p d, +3, -2)

mod encoder;
mod event;
mod machine;
mod sources;

use std::time::Instant;

use tokio::sync::mpsc;
use zbus::Connection;
use zbus::OwnedObjectPath;

use encoder::Encoder;
use event::{DialEvent, log};
use machine::{Effect, Machine};
use sources::Raw;

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let no_buds = std::env::args().any(|a| a == "--no-buds");
    let (tx, mut rx) = mpsc::unbounded_channel::<Raw>();

    log("daemon", "dial-daemon starting. Lid must be CLOSED for the dial to send anything. Ctrl-C to stop.");
    let conn = Connection::system().await?;
    if !no_buds {
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

    let mut machine = Machine::new();
    let mut encoder = Encoder::default();
    let mut transport: Option<OwnedObjectPath> = None;

    loop {
        let sleep = async {
            match machine.deadline() {
                Some(d) => tokio::time::sleep_until(d.into()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sleep => {
                let fx = machine.tick(Instant::now());
                apply(fx, &machine, &mut encoder, &conn, transport.as_ref()).await;
            }
            Some(raw) = rx.recv() => {
                let now = Instant::now();
                match raw {
                    Raw::TransportUp(path, volume) => {
                        encoder.baseline(volume);
                        transport = Some(path);
                        // A reconnect while parked must park again.
                        if machine.should_park() {
                            encoder.set_parking(false, now);
                            park(&mut encoder, &conn, transport.as_ref(), now).await;
                        }
                    }
                    Raw::TransportDown(path) => {
                        if transport.as_ref() == Some(&path) {
                            transport = None;
                            encoder.transport_lost();
                        }
                    }
                    Raw::TransportVolume(v) => {
                        let d = encoder.report(v, now);
                        if let Some(w) = d.write {
                            write(&conn, transport.as_ref(), w).await;
                        }
                        match d.steps {
                            Some(steps) => dispatch(DialEvent::Turn(steps), "bluez", &mut machine, &mut encoder, &conn, transport.as_ref()).await,
                            None => log("encoder", format!("volume {v}: echo or no change")),
                        }
                    }
                    Raw::Dial(e, src) => dispatch(e, src, &mut machine, &mut encoder, &conn, transport.as_ref()).await,
                }
            }
        }
    }
    log("daemon", "bye");
    Ok(())
}

async fn dispatch(
    event: DialEvent,
    source: &str,
    machine: &mut Machine,
    encoder: &mut Encoder,
    conn: &Connection,
    transport: Option<&OwnedObjectPath>,
) {
    log("event", format!("{event} from {source}  [state: {}]", machine.describe()));
    let before = machine.describe();
    let fx = machine.handle(event, Instant::now());
    if before != machine.describe() {
        log("state", format!("{before} → {}", machine.describe()));
    }
    apply(fx, machine, encoder, conn, transport).await;
}

async fn apply(fx: Vec<Effect>, machine: &Machine, encoder: &mut Encoder, conn: &Connection, transport: Option<&OwnedObjectPath>) {
    for effect in fx {
        match effect {
            Effect::SetParking(on) => {
                log("park", if on { "parking at midpoint" } else { "parking off, volume free" });
                if let Some(v) = encoder.set_parking(on, Instant::now()) {
                    write(conn, transport, v).await;
                }
            }
            Effect::Wheel(Some(m)) => log("wheel", format!("open, selected {}", m.label())),
            Effect::Wheel(None) => log("wheel", "closed"),
            Effect::Activate(m) => log("mode", format!("{}: activate (would act here)", m.label())),
            Effect::Deactivate(m) => log("mode", format!("{}: deactivate", m.label())),
            Effect::ModeTurn(m, n) => log("mode", format!("{}: turn {n:+} (would act here)", m.label())),
            Effect::ModePress(m) => log("mode", format!("{}: press (would act here)", m.label())),
        }
    }
    let _ = machine;
}

/// Park again after a reconnect.
async fn park(encoder: &mut Encoder, conn: &Connection, transport: Option<&OwnedObjectPath>, now: Instant) {
    if let Some(v) = encoder.set_parking(true, now) {
        write(conn, transport, v).await;
    }
}

async fn write(conn: &Connection, transport: Option<&OwnedObjectPath>, value: u16) {
    let Some(path) = transport else {
        log("park", "no transport to write to");
        return;
    };
    match sources::write_transport_volume(conn, path, value).await {
        Ok(()) => log("park", format!("wrote MediaTransport1.Volume={value}")),
        Err(e) => log("park", format!("write failed: {e}")),
    }
}
