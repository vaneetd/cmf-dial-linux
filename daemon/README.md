# dial-daemon

Captures the dial, decodes it, runs the state machine, and performs the mode actions.

```
cargo run --release                 # with the buds (case lid closed)
cargo run --release -- --no-buds    # drive it from the keyboard instead
cargo test                          # decoder + state machine tests
cargo run --release -- --snapshot DIR   # render the wheel and HUD states to PNGs
--no-ui                             # log only, no overlay
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
| Apps | move through running apps, most recent first (HUD shows the name) | bring that app's latest window forward |
| Windows | bring the next / previous window forward, most recent first | bring the current pick forward |
| Workspaces | previous / next workspace (no wrap) | Super+W (overview) |
| Tabs | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+T |
| Scroll | 3 lines per detent (virtual wheel) | vertical / sideways |
| Undo / Redo | Ctrl+Shift+Z / Ctrl+Z | Ctrl+S |
| Slides | Right / Left | F5 start, Esc end |

Keys and scroll are sent through two virtual devices (`dial-linux keyboard`, `dial-linux wheel`) on `/dev/uinput`.

## Overlay

`src/ui/`: a Wayland layer-shell surface (overlay layer, top centre, no keyboard focus, empty input region)
drawn in software with tiny-skia, following `../dial/design/DESIGN.md` (dark theme). Maps only while the
wheel or the HUD pill is showing. Needs a compositor with `zwlr_layer_shell_v1` (COSMIC has it).

## Windows, apps and workspaces

`src/wm.rs` is a second Wayland connection using COSMIC's protocols: `ext_workspace_v1`,
`ext_foreign_toplevel_list_v1` with `zcosmic_toplevel_info_v1`, and `zcosmic_toplevel_manager_v1`
to bring a window forward. Window order is most recently active first, tracked from state changes.
`dial-daemon --wm-list` prints what the compositor reports.
