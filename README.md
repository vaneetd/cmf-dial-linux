# Dial for Linux

Turn the Smart Dial on a **CMF Buds Pro 2** charging case into a control surface for your Linux desktop.

Double press the dial to open a radial wheel of nine modes. Turn it to pick one, press to choose it. From then on every turn drives that mode (volume, brightness, apps, windows, workspaces, tabs, scroll, undo/redo, slides) until you double press again.

> **Use at your own risk.** This software is provided as is, without warranty. See [Disclaimer](#disclaimer).

This project is inspired by [Dial for macOS](https://github.com/athulrajt/dial) by athulrajt. The interaction model, the mode set and the visual design come from that project. This repository is an independent Linux implementation, built from scratch for Wayland and the COSMIC desktop.

## How it works

The dial doesn't talk to the computer directly. It talks to the earbuds in the case, and the earbuds report it over Bluetooth the way headphones report their own controls. Dial for Linux listens to those reports:

| Gesture | What the computer sees |
|---|---|
| Turn | A change in the earbuds' absolute volume (`MediaTransport1.Volume` on BlueZ). The sign of the change is the direction. |
| Press | An AVRCP key press (`KEY_PLAYCD` / `KEY_PAUSECD`) on the earbuds' virtual input device |
| Double press | An AVRCP key press (`KEY_NEXTSONG`) on the same device |

Turning the dial moves the earbuds' volume, and a volume has ends. To keep turns from running into 0% or 100%, the daemon **parks** the volume at the midpoint after every detent, except in Volume mode. It also ignores its own writes when they echo back. The hardware measurements behind this are in [docs/phase0-findings.md](docs/phase0-findings.md).

```
BlueZ D-Bus ──┐                                     ┌── uinput  (keys, scroll)
              ├─▶ decoder ─▶ state machine ─▶ modes ┼── logind  (brightness)
evdev (AVRCP) ┘   (parking,    rest / picking /     ├── COSMIC protocols (windows, workspaces)
                   echo filter)   in-mode           └── Wayland overlay (wheel, HUD)
```

## Modes

| Mode | Turn | Press |
|---|---|---|
| Volume | The earbuds move the volume themselves | Play / pause |
| Brightness | Dimmer / brighter, 6% per detent | None |
| Apps | Step through running apps, most recently used first | Bring that app forward |
| Windows | Bring the next or previous window forward | Bring the current pick forward |
| Workspaces | Previous / next workspace | Workspace overview |
| Tabs | Next / previous tab (`Ctrl+Tab`) | New tab |
| Scroll | Scroll 3 lines per detent | Switch between vertical and sideways |
| Undo / Redo | Left undoes, right redoes | Save |
| Slides | Next / previous slide | Start / end the show |

The wheel and a small HUD pill are drawn as a non-focus-stealing overlay, so the app you are controlling keeps keyboard focus.

## Tech stack

- **Language:** Rust (edition 2024), async runtime [tokio](https://tokio.rs)
- **Bluetooth and system buses:** [zbus](https://github.com/dbus2/zbus) for BlueZ (turn events, parking) and systemd-logind (brightness)
- **Input:** [evdev](https://crates.io/crates/evdev) to read the earbuds' key device and to create virtual keyboard and scroll-wheel devices through `/dev/uinput`
- **Overlay:** [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) with the `wlr-layer-shell` protocol, drawn in software with [tiny-skia](https://github.com/linebender/tiny-skia) and [fontdue](https://github.com/mooman219/fontdue)
- **Windows and workspaces:** COSMIC's Wayland protocols via [cosmic-protocols](https://github.com/pop-os/cosmic-protocols) and `ext-workspace-v1` / `ext-foreign-toplevel-list-v1` from `wayland-protocols`
- **Hardware probe:** a Python logger in [tools/dial-probe](tools/dial-probe) that was used to discover how the dial behaves

## Requirements

- A **CMF Buds Pro 2** (by Nothing), paired and connected. Set the Nothing X gestures to: press = Play/Pause, double press = Next track, rotate = Volume. **The case lid must be closed**, or the dial sends nothing.
- A Wayland desktop with layer-shell and COSMIC's toplevel and workspace protocols. Developed and tested on **Pop!_OS 24.04 with COSMIC**.
- BlueZ, systemd-logind, and read access to the earbuds' input device and `/dev/uinput` (`dial-daemon install` sets this up with a udev rule).
- Rust (via [rustup](https://rustup.rs)) only if you build from source.

## Installation

### 1. Prepare the earbuds

1. Pair and connect the CMF Buds Pro 2 to your computer (COSMIC Settings → Bluetooth), and choose them as the sound output.
2. In the Nothing X app on your phone, set the gestures to: **press = Play/Pause, double press = Next track, rotate = Volume**.
3. Keep the earbuds in the case with **the lid closed** while you use the dial. With the lid open the dial sends nothing.

### 2. Get the program

Pick one. All three give you the same single `dial-daemon` binary.

**A. Download a release** (nothing else to install). Replace `OWNER/REPO` with this repository's path:

```bash
curl -L -o dial-daemon https://github.com/OWNER/REPO/releases/latest/download/dial-daemon-x86_64-linux
chmod +x dial-daemon
```

**B. Install with cargo** (needs [Rust](https://rustup.rs)), straight from Git, no clone:

```bash
cargo install --git https://github.com/OWNER/REPO dial-daemon
```

**C. Build from a clone** (needs [Rust](https://rustup.rs)):

```bash
git clone https://github.com/OWNER/REPO
cd REPO/daemon
cargo build --release
```

### 3. Set it up

Run the setup once. It asks for your `sudo` password for one step only, the udev rule.

```bash
./dial-daemon install                    # option A, from the folder you downloaded it to
dial-daemon install                      # option B
./target/release/dial-daemon install     # option C, from the daemon/ folder
```

It does three things:

| Step | Where | Why |
|---|---|---|
| Copies the program | `~/.local/bin/dial-daemon` | A fixed place to run it from |
| Adds an autostart entry | `~/.config/autostart/dial-daemon.desktop` | Starts Dial when you log in |
| Installs a udev rule (needs `sudo`) | `/etc/udev/rules.d/70-dial.rules` | Lets your user read the earbuds' key device and create virtual input devices, with no group changes |

### 4. Start it

Log out and back in, or start it now:

```bash
nohup ~/.local/bin/dial-daemon >/dev/null 2>&1 &
```

Close the case lid, then **double press the dial**. The wheel should appear at the top of the screen.

To see what it is doing, run it in a terminal instead (`dial-daemon`). Every event is logged with a timestamp.

### Uninstall

```bash
dial-daemon uninstall
pkill dial-daemon      # stops a copy that is still running
```

### Troubleshooting

| Problem | What to check |
|---|---|
| Nothing happens when you turn or press | The case lid must be closed, and the earbuds connected. Run `dial-daemon` in a terminal and watch for `[bluez]` and `[evdev]` lines. |
| `no AVRCP input device yet` in the log | The earbuds aren't connected, or the udev rule isn't active. Reconnect them, or run `sudo udevadm trigger --subsystem-match=input`. |
| `no virtual keyboard` in the log | `/dev/uinput` isn't accessible. Run `dial-daemon install` again, then log out and in. |
| The wheel never appears but the log shows `[wheel]` lines | Your compositor needs to support the layer-shell protocol. COSMIC does. The log shows `[ui]` errors if the overlay couldn't start. |
| Volume jumps or the dial feels slow | The earbuds report a turn at most about three times a second. See [docs/phase0-findings.md](docs/phase0-findings.md). |

## Development

```bash
cd daemon
cargo run --release                    # with the earbuds
cargo run --release -- --no-buds       # keyboard instead: r / l turn, p press, d double press, +3 / -2 several detents
cargo run --release -- --no-ui         # log only, no overlay
cargo run --release -- --snapshot DIR  # render the wheel and HUD to PNG files
cargo run --release -- --wm-list       # print the windows and workspaces the compositor reports
cargo test                             # decoder and state-machine tests
```

Logs are printed with millisecond timestamps, one line per event, state change and action. Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed.

## Repository layout

| Path | What it is |
|---|---|
| `daemon/` | The Rust daemon (`src/sources.rs` input, `encoder.rs` turn decoding, `machine.rs` state machine, `actions.rs` mode actions, `wm.rs` windows and workspaces, `install.rs` setup, `ui/` overlay) |
| `tools/dial-probe/` | The Python probe that logs every signal the dial produces |
| `docs/phase0-findings.md` | What the hardware does, measured |
| `daemon/assets/` | The udev rule and autostart entry that `dial-daemon install` writes |

## Status and limitations

This is an early project, built around one pair of earbuds and one desktop.

- The input path (turn, press, double press, parking, echo filtering) is tested against real hardware. The earbuds report at most about three times a second, so a very fast spin can merge or drop detents. That is a limit of the hardware, not of the software.
- The Apps mode shows its pick in the HUD; the card fan from the macOS design is not built yet.
- The overlay uses the dark theme only.
- Reconnecting the earbuds while the daemon runs is handled in code but has had little testing.
- Only COSMIC is supported for Windows, Apps and Workspaces. Other compositors would need their own window-management code.

## Disclaimer

**This software is provided "as is", without warranty of any kind. Everyone who installs or runs it does so entirely at their own risk.** The authors and contributors are not liable for any damage, data loss or other harm that results from using it.

Things worth knowing before you run it:

- It sends **keystrokes and scroll events** to whichever window has focus, for example undo, redo, save, close-tab-style shortcuts and slide keys. A mode picked by mistake can change or save a document.
- It **changes your screen brightness and the earbuds' volume**. The volume is moved to the midpoint while you use the wheel and modes, so it can jump suddenly. Take the earbuds out of your ears if that matters to you.
- It **brings windows forward and switches workspaces**.
- `dial-daemon install` writes a **udev rule** to `/etc/udev/rules.d` with `sudo`, and the program creates virtual input devices through `/dev/uinput`. Read `daemon/assets/70-dial.rules` and the source first if you want to be sure of what it does.
- It was built and tested on one setup (Pop!_OS 24.04 with COSMIC, one pair of CMF Buds Pro 2). Anything else is untested.
- This is an independent project. It is not affiliated with, endorsed by or supported by Nothing, System76, or the authors of the macOS Dial app.

## License

[MIT](LICENSE)

## Acknowledgements

Inspired by [athulrajt/dial](https://github.com/athulrajt/dial), the macOS app that came up with the idea of using the Smart Dial as a mode wheel. The visual language of the wheel follows that project's design spec.
