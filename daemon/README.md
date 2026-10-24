# dial-daemon

Captures the dial, decodes it, runs the state machine, and performs the mode actions.

```
cargo run --release                 # with the buds (case lid closed)
cargo run --release -- --no-buds    # drive it from the keyboard instead
cargo test                          # decoder + state machine tests
```

Typed into the terminal (works in both modes): `r` / `l` one detent right/left, `+3` / `-2` several,
`p` press, `d` double press.

Sources: BlueZ `MediaTransport1.Volume` (turn), the AVRCP evdev device (press = PLAYCD/PAUSECD/PLAYPAUSE,
double press = NEXTSONG). Parking writes `MediaTransport1.Volume = 64` outside Rest and Volume mode.
See `../docs/phase0-findings.md`.

## Modes (turn / press)

| Mode | Turn | Press |
|---|---|---|
| Volume | the buds move it | media player (untouched) |
| Brightness | +/-6% via logind `SetBrightness` (never below 1%) | nothing |
| Apps, Windows | not built yet | not built yet |
| Workspaces | Super+Ctrl+Right / Left | Super+W (overview) |
| Tabs | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+T |
| Scroll | 3 lines per detent (virtual wheel) | vertical / sideways |
| Undo / Redo | Ctrl+Shift+Z / Ctrl+Z | Ctrl+S |
| Slides | Right / Left | F5 start, Esc end |

Keys and scroll are sent through two virtual devices (`dial-linux keyboard`, `dial-linux wheel`) on `/dev/uinput`.
