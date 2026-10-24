# Dial for Linux

A Linux port of the idea behind Dial (macOS, sibling repo `../dial`): turn the Smart Dial on a CMF Buds Pro 2 case into a control surface for the desktop.

- **Double press** opens a radial wheel of nine modes.
- **Turn** to pick a mode, **press** to choose it.
- Then every turn drives that mode (volume, brightness, apps, windows, workspaces, tabs, scroll, undo/redo, slides) until the next double press.

The owner is Art, who runs the app with the real buds and pastes logs back. You can't turn or press the dial, so write clear, timestamped logs and say exactly what he should do and what you expect to see.

## Target machine

Pop!_OS 24.04, **COSMIC desktop, Wayland** (`cosmic-comp`), PipeWire 1.6.8 + WirePlumber, BlueZ 5.72, Python 3.12. XWayland exists but can't see native Wayland windows. Rust/cargo is **not installed yet**; `brightnessctl` is missing. `/dev/uinput` is `root:root 0600` and Art isn't in the `input` group.

## Reference: what the macOS version learned (`../dial/docs/phase0-findings.md`)

These are hardware facts; expect them to hold on Linux, but verify:

- The dial talks to the buds; the buds send Bluetooth headphone commands. The case **lid must be closed** or nothing is sent.
- **Turn** arrives as a change in the buds' absolute volume, quantised to 1/16 steps. Sign of the change = direction. Past 0% or 100% the direction is lost, so **park** the volume at a midpoint (0.5) after each detent in every mode except Volume. Our own write echoes back (about 8 notifications, 10-35 ms later): ignore changes within 400 ms of a write that land within ±0.032 of the value written.
- The buds **rate-limit** reports: changes never came closer than about 271 ms. Fast clicks are dropped or merged into a two-step change. So no "fast spin under 70 ms"; when not parked (Volume mode) count steps as `round(|delta| / 0.0625)`. Expect 0.3-0.5 s latency; Mac code can't shorten it.
- **Press** = play/pause, **double press** = next track (with Nothing X gestures set: press = Play/Pause, double press = Next track, rotate = Volume). On macOS these only reached the app that owned Now Playing, which forced a helper process.

## Status

**Phase 0: Probe. Built, not yet run against the buds.** `tools/dial-probe/dial_probe.py` logs three sources with ms timestamps:

- `[bluez]` `org.bluez.MediaTransport1.Volume` (0-127) and device state over D-Bus
- `[sink]` PipeWire sink volume via `pactl subscribe` + `pactl --format=json`
- `[evdev]` key events from BlueZ's virtual AVRCP input device

Options: `--park 0.5` (park the sink volume), `--park-transport 64` (park MediaTransport1.Volume), `--evdev-match`, `--no-evdev`. See `tools/dial-probe/README.md` for the four runs Art should do.

Verified so far (2026-10-24, buds connected, nothing turned or pressed yet):

- Device `CMF Buds Pro 2`, MAC 3C:B0:ED:33:AA:74, connected and paired.
- `MediaTransport1` at `/org/bluez/hci0/dev_3C_B0_ED_33_AA_74/sep1/fd0`, state `idle`, Volume 64.
- Default sink became `bluez_output.3C_B0_ED_33_AA_74.1`, volume 0.5039 (= 64/127), so sink volume and AVRCP volume are linked.
- AVRCP input device is `/dev/input/event12` ("CMF Buds Pro 2 (AVRCP)"), `root:input 0660`. Not readable until Art runs `sudo setfacl -m u:$USER:r /dev/input/event12` (resets on reboot; the number may change after a reconnect).

**Next step:** Art runs the probe (runs A-D in the README) and pastes the logs. Then write `docs/phase0-findings.md` (turn source: transport vs sink; do press/double press arrive as evdev `KEY_PLAYPAUSE`/`KEY_NEXTSONG` or only via MPRIS; does parking work; does another media app steal the keys) and recommend the capture path. **Stop at the end of every phase** and summarise what changed and what Art should test.

## Open questions to settle from the logs

1. Turn signal: transport Volume, sink volume, or both; which is cleaner and faster?
2. Press and double press: evdev keys, MPRIS, or both? If a media player (browser, Spotify) also reacts, do we grab the evdev device (`EVIOCGRAB`) to stop it?
3. Does parking at the midpoint work, and does the echo filter hold?
4. Does the buds' default-sink handling (drop-outs, new device id on reconnect) need re-attach logic?

## Planned direction (not yet decided with Art)

- Daemon in **Rust** (`zbus`, `evdev`, PipeWire or `wpctl`) emitting `turn(i32)`, `press`, `doublePress`; same parking and echo-filter logic as macOS. Install Rust via rustup first (ask Art).
- UI in **libcosmic/iced** with the Wayland **layer-shell** protocol (non-activating overlay, like the macOS `NSPanel`). Window listing via COSMIC's toplevel protocols (verify which `cosmic-comp` exposes). Key and scroll injection via `/dev/uinput` (needs a udev rule or group) or the libei/RemoteDesktop portal.
- Port the macOS architecture: input source -> `DialEvent` -> state machine (rest / picking / inMode) -> modes -> actions. A simulated-input source (global hotkeys) must exist so UI work never waits on hardware.
- Reuse the visual spec in `../dial/design/` (`DESIGN.md`, `tokens.json`, reference screenshots) for the wheel and HUD.

## Working rules

- No new dependencies or system changes (rustup, udev rules, group changes) without asking Art.
- Commit at the end of each phase, with the phase in the message.
- Only Linux code lives here; don't edit `../dial`.
