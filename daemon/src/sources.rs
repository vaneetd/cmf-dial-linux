//! Input sources. Each one runs as a task and sends `Raw` messages to the main loop.
//!
//! - BlueZ: `MediaTransport1.Volume` over D-Bus (turn), plus transport appear/disappear.
//! - evdev: the buds' AVRCP key device (press, double press), re-found by name after a reconnect.
//! - Simulated: lines on stdin, so the state machine can be driven without the buds.

use std::collections::HashMap;
use std::time::Duration;

use evdev::{EventSummary, KeyCode};
use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::UnboundedSender;
use zbus::{OwnedObjectPath, OwnedVariant as OwnedValue};
use zbus::{Connection, MatchRule, MessageStream};

use crate::event::{DialEvent, log};

#[derive(Debug)]
pub enum Raw {
    /// A transport appeared (or was found at start) with this volume.
    TransportUp(OwnedObjectPath, u16),
    TransportDown(OwnedObjectPath),
    TransportVolume(u16),
    Dial(DialEvent, &'static str),
}

const TRANSPORT: &str = "org.bluez.MediaTransport1";

type Props = HashMap<String, OwnedValue>;

pub async fn bluez(conn: Connection, tx: UnboundedSender<Raw>) -> zbus::Result<()> {
    // Subscribe first so nothing is missed between the snapshot and the first signal.
    let props_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")
        .interface("org.freedesktop.DBus.Properties")
        .member("PropertiesChanged")
        .build()?;
    let added_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")
        .interface("org.freedesktop.DBus.ObjectManager")
        .member("InterfacesAdded")
        .build()?;
    let removed_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")
        .interface("org.freedesktop.DBus.ObjectManager")
        .member("InterfacesRemoved")
        .build()?;
    let mut props = MessageStream::for_match_rule(props_rule, &conn, None).await?;
    let mut added = MessageStream::for_match_rule(added_rule, &conn, None).await?;
    let mut removed = MessageStream::for_match_rule(removed_rule, &conn, None).await?;

    let manager = zbus::fdo::ObjectManagerProxy::builder(&conn)
        .destination("org.bluez")
        .path("/")
        .build()
        .await?;
    match manager.get_managed_objects().await {
        Ok(objects) => {
            for (path, ifaces) in objects {
                if let Some(t) = ifaces.get(TRANSPORT) {
                    announce_up(&tx, path, t);
                }
            }
        }
        Err(e) => log("bluez", format!("BlueZ not reachable yet: {e}")),
    }

    loop {
        tokio::select! {
            Some(Ok(msg)) = props.next() => {
                let Some(path) = msg.header().path().map(|p| p.to_owned()) else { continue };
                if let Ok((iface, changed, _inv)) = msg.body().deserialize::<(String, Props, Vec<String>)>() {
                    if iface == TRANSPORT {
                        if let Some(v) = changed.get("Volume").and_then(|v| u16::try_from(v).ok()) {
                            log("bluez", format!("transport volume {v}/127 {path}"));
                            let _ = tx.send(Raw::TransportVolume(v));
                        }
                    }
                }
            }
            Some(Ok(msg)) = added.next() => {
                if let Ok((path, ifaces)) = msg.body().deserialize::<(OwnedObjectPath, HashMap<String, Props>)>() {
                    if let Some(t) = ifaces.get(TRANSPORT) {
                        announce_up(&tx, path, t);
                    }
                }
            }
            Some(Ok(msg)) = removed.next() => {
                if let Ok((path, ifaces)) = msg.body().deserialize::<(OwnedObjectPath, Vec<String>)>() {
                    if ifaces.iter().any(|i| i == TRANSPORT) {
                        log("bluez", format!("transport gone {path}"));
                        let _ = tx.send(Raw::TransportDown(path));
                    }
                }
            }
            else => return Ok(()),
        }
    }
}

fn announce_up(tx: &UnboundedSender<Raw>, path: OwnedObjectPath, props: &Props) {
    let volume = props.get("Volume").and_then(|v| u16::try_from(v).ok());
    log("bluez", format!("transport up {path} volume={volume:?}"));
    if let Some(v) = volume {
        let _ = tx.send(Raw::TransportUp(path, v));
    }
}

/// Write `MediaTransport1.Volume` (the park).
pub async fn write_transport_volume(conn: &Connection, path: &OwnedObjectPath, value: u16) -> zbus::Result<()> {
    let proxy = zbus::Proxy::new(conn, "org.bluez", path.as_ref(), TRANSPORT).await?;
    proxy.set_property("Volume", value).await?;
    Ok(())
}

fn find_avrcp() -> Option<evdev::Device> {
    evdev::enumerate().map(|(_, d)| d).find(|d| {
        let name = d.name().unwrap_or("").to_lowercase();
        name.contains("avrcp") && (name.contains("cmf") || name.contains("buds"))
    })
}

pub async fn avrcp_keys(tx: UnboundedSender<Raw>) {
    let mut warned = false;
    loop {
        let Some(device) = find_avrcp() else {
            if !warned {
                log("evdev", "no AVRCP input device yet (buds not connected, or not readable); retrying");
                warned = true;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        warned = false;
        let name = device.name().unwrap_or("?").to_string();
        let mut stream = match device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                log("evdev", format!("cannot open {name}: {e}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        log("evdev", format!("listening on {name}"));
        loop {
            match stream.next_event().await {
                Ok(ev) => {
                    if let EventSummary::Key(_, code, 1) = ev.destructure() {
                        let event = match code {
                            KeyCode::KEY_PLAYCD | KeyCode::KEY_PAUSECD | KeyCode::KEY_PLAYPAUSE => {
                                Some(DialEvent::Press)
                            }
                            KeyCode::KEY_NEXTSONG => Some(DialEvent::DoublePress),
                            _ => None,
                        };
                        log("evdev", format!("{code:?} down{}", event.map_or(String::new(), |e| format!(" → {e}"))));
                        if let Some(e) = event {
                            let _ = tx.send(Raw::Dial(e, "evdev"));
                        }
                    }
                }
                Err(e) => {
                    log("evdev", format!("device lost ({e}); looking again"));
                    break;
                }
            }
        }
    }
}

/// `r`/`l` = one detent right/left, `+3`/`-2` = several, `p` = press, `d` = double press.
pub async fn simulated(tx: UnboundedSender<Raw>) {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let s = line.trim();
        let event = match s {
            "r" => Some(DialEvent::Turn(1)),
            "l" => Some(DialEvent::Turn(-1)),
            "p" => Some(DialEvent::Press),
            "d" => Some(DialEvent::DoublePress),
            _ => s.parse::<i32>().ok().filter(|n| *n != 0).map(DialEvent::Turn),
        };
        match event {
            Some(e) => {
                let _ = tx.send(Raw::Dial(e, "sim"));
            }
            None if s.is_empty() => {}
            None => log("sim", format!("unknown {s:?}: r l p d, or a number like +3 / -2")),
        }
    }
}
