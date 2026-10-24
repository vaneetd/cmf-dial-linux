//! What each mode does. Keys and scroll go through `/dev/uinput`, volume and brightness through
//! `wpctl` and `brightnessctl`. Every action returns a short text for the HUD (shown in the log
//! until the HUD exists).
//!
//! Shortcuts are the ones the apps and COSMIC use by default (`/usr/share/cosmic/.../defaults`).

use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode};
use zbus::Connection;

use crate::event::{ModeId, log};

const LINES_PER_DETENT: i32 = 3;
const BRIGHTNESS_STEP_PERCENT: u32 = 6;

pub struct Actions {
    system: Connection,
    keys: Option<VirtualDevice>,
    wheel: Option<VirtualDevice>,
    scroll_sideways: bool,
    presenting: bool,
}

impl Actions {
    pub fn new(system: Connection) -> Self {
        let keys = keyboard().map_err(|e| log("uinput", format!("no virtual keyboard: {e}"))).ok();
        let wheel = mouse().map_err(|e| log("uinput", format!("no virtual wheel: {e}"))).ok();
        if keys.is_some() && wheel.is_some() {
            log("uinput", "virtual keyboard and wheel ready");
        }
        Actions { system, keys, wheel, scroll_sideways: false, presenting: false }
    }

    pub async fn activate(&mut self, mode: ModeId) -> String {
        match mode {
            ModeId::Brightness => match brightness_percent().await {
                Some(p) => format!("{p}%"),
                None => "brightness unavailable".into(),
            },
            _ => String::new(),
        }
    }

    pub fn deactivate(&mut self, mode: ModeId) {
        if mode == ModeId::Slides {
            self.presenting = false;
        }
    }

    pub async fn turn(&mut self, mode: ModeId, steps: i32) -> String {
        let forward = steps > 0;
        let n = steps.unsigned_abs() as usize;
        match mode {
            // The buds move the volume themselves in this mode.
            ModeId::Volume => "volume".into(),
            ModeId::Brightness => {
                self.nudge_brightness(steps).await;
                match brightness_percent().await {
                    Some(p) => format!("{p}%"),
                    None => "brightness".into(),
                }
            }
            ModeId::Tabs => {
                let combo: &[KeyCode] = if forward {
                    &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_TAB]
                } else {
                    &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_TAB]
                };
                self.tap_n(combo, n);
                (if forward { "Next tab" } else { "Previous tab" }).into()
            }
            ModeId::UndoRedo => {
                let combo: &[KeyCode] = if forward {
                    &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_Z]
                } else {
                    &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_Z]
                };
                self.tap_n(combo, n);
                (if forward { "Redo" } else { "Undo" }).into()
            }
            ModeId::Slides => {
                self.tap_n(&[if forward { KeyCode::KEY_RIGHT } else { KeyCode::KEY_LEFT }], n);
                (if forward { "Next slide" } else { "Previous slide" }).into()
            }
            ModeId::Desktops => {
                let arrow = if forward { KeyCode::KEY_RIGHT } else { KeyCode::KEY_LEFT };
                self.tap_n(&[KeyCode::KEY_LEFTMETA, KeyCode::KEY_LEFTCTRL, arrow], n);
                (if forward { "Next workspace" } else { "Previous workspace" }).into()
            }
            ModeId::Scroll => {
                self.scroll(steps * LINES_PER_DETENT);
                let arrow = match (self.scroll_sideways, forward) {
                    (false, true) => "↓",
                    (false, false) => "↑",
                    (true, true) => "→",
                    (true, false) => "←",
                };
                format!("{arrow} {} lines", n as i32 * LINES_PER_DETENT)
            }
            ModeId::Apps | ModeId::Windows => format!("{}: not built yet", mode.label()),
        }
    }

    pub async fn press(&mut self, mode: ModeId) -> String {
        match mode {
            // The press already reached the media player (nothing grabs the device).
            ModeId::Volume => "Play / Pause".into(),
            ModeId::Brightness => "Nothing (no night light control yet)".into(),
            ModeId::Tabs => {
                self.tap(&[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_T]);
                "New tab".into()
            }
            ModeId::UndoRedo => {
                self.tap(&[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_S]);
                "Save".into()
            }
            ModeId::Slides => {
                if self.presenting {
                    self.tap(&[KeyCode::KEY_ESC]);
                    self.presenting = false;
                    "End show".into()
                } else {
                    self.tap(&[KeyCode::KEY_F5]);
                    self.presenting = true;
                    "Start show".into()
                }
            }
            ModeId::Desktops => {
                self.tap(&[KeyCode::KEY_LEFTMETA, KeyCode::KEY_W]);
                "Workspace overview".into()
            }
            ModeId::Scroll => {
                self.scroll_sideways = !self.scroll_sideways;
                (if self.scroll_sideways { "Sideways" } else { "Vertical" }).into()
            }
            ModeId::Apps | ModeId::Windows => format!("{}: not built yet", mode.label()),
        }
    }

    fn tap_n(&mut self, combo: &[KeyCode], n: usize) {
        for _ in 0..n {
            self.tap(combo);
        }
    }

    /// Modifiers down in order, last key tapped, then everything released in reverse.
    fn tap(&mut self, combo: &[KeyCode]) {
        let Some(dev) = self.keys.as_mut() else {
            log("uinput", "no virtual keyboard, dropping keys");
            return;
        };
        let key = |code: KeyCode, value: i32| InputEvent::new(EventType::KEY.0, code.code(), value);
        let pause = || std::thread::sleep(Duration::from_millis(4));
        for &k in combo {
            let _ = dev.emit(&[key(k, 1)]);
            pause();
        }
        for &k in combo.iter().rev() {
            let _ = dev.emit(&[key(k, 0)]);
            pause();
        }
    }

    /// Positive lines = down (or right when sideways).
    fn scroll(&mut self, lines: i32) {
        let Some(dev) = self.wheel.as_mut() else {
            log("uinput", "no virtual wheel, dropping scroll");
            return;
        };
        let (lo, hi) = if self.scroll_sideways {
            (RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_HWHEEL_HI_RES)
        } else {
            (RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_WHEEL_HI_RES)
        };
        // evdev wheels count up/right as positive.
        let sign = if self.scroll_sideways { 1 } else { -1 };
        let step = lines.signum() * sign;
        for _ in 0..lines.abs() {
            let _ = dev.emit(&[
                InputEvent::new(EventType::RELATIVE.0, hi.0, step * 120),
                InputEvent::new(EventType::RELATIVE.0, lo.0, step),
            ]);
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}

fn keyboard() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in [
        KeyCode::KEY_LEFTCTRL,
        KeyCode::KEY_LEFTSHIFT,
        KeyCode::KEY_LEFTMETA,
        KeyCode::KEY_TAB,
        KeyCode::KEY_Z,
        KeyCode::KEY_S,
        KeyCode::KEY_T,
        KeyCode::KEY_W,
        KeyCode::KEY_F5,
        KeyCode::KEY_ESC,
        KeyCode::KEY_LEFT,
        KeyCode::KEY_RIGHT,
    ] {
        keys.insert(k);
    }
    VirtualDevice::builder()?.name("dial-linux keyboard").with_keys(&keys)?.build()
}

/// A wheel needs to look like a mouse to libinput, so it also declares motion and a button.
fn mouse() -> std::io::Result<VirtualDevice> {
    let mut axes = AttributeSet::<RelativeAxisCode>::new();
    for a in [
        RelativeAxisCode::REL_X,
        RelativeAxisCode::REL_Y,
        RelativeAxisCode::REL_WHEEL,
        RelativeAxisCode::REL_HWHEEL,
        RelativeAxisCode::REL_WHEEL_HI_RES,
        RelativeAxisCode::REL_HWHEEL_HI_RES,
    ] {
        axes.insert(a);
    }
    let mut buttons = AttributeSet::<KeyCode>::new();
    buttons.insert(KeyCode::BTN_LEFT);
    VirtualDevice::builder()?
        .name("dial-linux wheel")
        .with_keys(&buttons)?
        .with_relative_axes(&axes)?
        .build()
}

struct Backlight {
    name: String,
    current: i64,
    max: i64,
}

fn backlight() -> Option<Backlight> {
    let dir = std::fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?;
    let read = |f: &str| std::fs::read_to_string(dir.path().join(f)).ok()?.trim().parse::<i64>().ok();
    Some(Backlight {
        name: dir.file_name().to_string_lossy().into_owned(),
        current: read("brightness")?,
        max: read("max_brightness")?,
    })
}

async fn brightness_percent() -> Option<u32> {
    let b = backlight()?;
    Some((b.current * 100 / b.max) as u32)
}

impl Actions {
    /// Brightness goes through logind's `SetBrightness`, which needs no root or group for the
    /// active session. Never goes below 1%, so the screen can't be turned off by turning the dial.
    async fn nudge_brightness(&self, steps: i32) {
        let Some(b) = backlight() else {
            log("action", "no backlight device in /sys/class/backlight");
            return;
        };
        let step = b.max * BRIGHTNESS_STEP_PERCENT as i64 / 100;
        let target = (b.current + steps as i64 * step).clamp((b.max / 100).max(1), b.max) as u32;
        let result = self
            .system
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1/session/auto",
                Some("org.freedesktop.login1.Session"),
                "SetBrightness",
                &("backlight", b.name.as_str(), target),
            )
            .await;
        if let Err(e) = result {
            log("action", format!("SetBrightness failed: {e}"));
        }
    }
}
