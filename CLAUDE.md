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

- **Phase 0 (probe): done.** Findings in `docs/phase0-findings.md`.
- **Phase 1 (daemon): done.** `daemon/`: BlueZ transport volume (turn) and AVRCP evdev (press, double press), parking, echo filter, state machine.
- **Phase 2 (actions): built, partly tested.** Brightness via logind, keys and scroll via uinput. Apps and Windows not built.
- **Phase 3 (overlay): built.** Wheel and HUD pill as a layer-shell overlay (`daemon/src/ui/`). Needs Art's check with the buds.

System setup done with Art's approval: rustup, `brightnessctl`, `/etc/udev/rules.d/70-dial.rules` (uaccess on the AVRCP device and `/dev/uinput`).

**Next:** Art tests Phases 2 and 3 with the buds; then Apps and Windows (COSMIC toplevel protocols). **Stop at the end of every phase.**

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
