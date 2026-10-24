# dial-daemon (Phase 1)

Captures the dial, decodes it, runs the state machine. Mode actions only log "would act here" for now.

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
