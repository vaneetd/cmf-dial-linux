# Dial for Linux

A Linux implementation of Dial (macOS, https://github.com/athulrajt/dial, sibling checkout `../dial`): the Smart Dial on a CMF Buds Pro 2 case becomes a control surface for the desktop. User-facing description is in `README.md`.

- **Double press** opens a radial wheel of nine modes. **Turn** to pick, **press** to choose.
- Then every turn drives that mode (volume, brightness, apps, windows, workspaces, tabs, scroll, undo/redo, slides) until the next double press.

The owner is Art, who runs the app with the real buds and pastes logs back. You can't turn or press the dial, so write clear, timestamped logs and say exactly what he should do and what you expect to see.

## Target machine

Pop!_OS 24.04, **COSMIC desktop, Wayland** (`cosmic-comp`), BlueZ 5.72. Rust via rustup. `/dev/uinput` and the buds' AVRCP device are readable through a udev `uaccess` rule (`daemon/assets/70-dial.rules`, installed by `dial-daemon install`). XWayland exists but can't see native Wayland windows.

## Layout

- `daemon/`: the Rust daemon. `sources.rs` (BlueZ D-Bus, evdev, stdin), `encoder.rs` (turn decoding, parking, echo filter), `machine.rs` (rest / picking / inMode), `actions.rs` (mode actions), `wm.rs` (COSMIC windows and workspaces), `ui/` (layer-shell overlay drawn with tiny-skia), `install.rs` (`install` / `uninstall`).
- `tools/dial-probe/`: the Python probe from Phase 0.
- `docs/phase0-findings.md`: what the hardware does. Read it before touching `encoder.rs`.
- `../dial/design/`: the visual spec (`DESIGN.md`, `tokens.json`, screenshots) the overlay follows.

## Hardware facts that drive the design (details in `docs/phase0-findings.md`)

- The case **lid must be closed** or nothing is sent.
- **Turn** = change in `MediaTransport1.Volume` (0-127), one detent about 8 units. **Press** = `KEY_PLAYCD`/`KEY_PAUSECD`, **double press** = `KEY_NEXTSONG`, on the AVRCP evdev device. Triple press (`KEY_PREVIOUSSONG`) is seen but unused.
- The buds report at most about every 300 ms, so fast spins can merge or drop detents.
- Outside Volume mode the volume is **parked** at 64 after each detent; our own write echoes back once, 8-35 ms later, and is ignored.

## Status

Phases 0-3 are built: probe, daemon, mode actions, wheel and HUD overlay, plus Workspaces, Apps and Windows through COSMIC protocols. Hardware-tested: turn, press, double press, parking, picking, mode switching. Lightly tested: the key and scroll modes in real apps, reconnecting the buds.

Not done: the Apps card fan (Apps shows its pick in the HUD only), the light theme, a triple-press gesture.

## Working rules

- Run `cargo test` in `daemon/` before committing. Use `--no-buds` (type `r l p d +3`) and `--snapshot DIR` (PNG render) to check work without the buds.
- No new system changes (packages, udev rules, group changes) without asking Art.
- **Stop at the end of every phase**, summarise what changed and what Art should test, and commit with the phase in the message.
- Don't kill `dial-daemon` processes you didn't start: Art may be running one.
- Only Linux code lives here; don't edit `../dial`.
