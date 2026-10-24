# Phase 0 findings (Linux)

Source: `probe-A.log` to `probe-D.log`, 2026-10-24, CMF Buds Pro 2 on Pop!_OS 24.04 / COSMIC / BlueZ + PipeWire.
Run A = nothing playing, B = media playing, C = `--park 0.5` (sink), D = `--park-transport 64`.
Runs A and B were done with gestures only; the logs don't tell us if the media app reacted to the keys (see "Not settled").

## Summary

| Question | Answer |
|---|---|
| Turn signal | `MediaTransport1.Volume` over D-Bus. The sink volume carries the same information about 225-235 ms later. |
| Press / double press | evdev on the buds' AVRCP device. Press = `KEY_PLAYCD` or `KEY_PAUSECD` (alternates), double press = `KEY_NEXTSONG`. Never `KEY_PLAYPAUSE`. |
| Parking | Works. Park by writing `MediaTransport1.Volume = 64` straight from the transport handler (run D). |
| Echo filter | Holds. Our write echoes once, 8-35 ms later. |
| Reconnect handling | Not tested. |

## Turn

- Every detent shows up as a `MediaTransport1.Volume` change (0-127). Sign of the delta is the direction.
- One detent = +/-8 or +/-7 on the 0-127 scale (1/16 of the range, quantised). A merged two-step report was seen once: `64 -> 79 = +15` (run A, first turn), which the probe read as `steps~2`.
- Step count: `round(|delta| * 16 / 127)`. This gives 1 for 7 and 8, and 2 for 15.
- The sink (`pactl`) mirrors every transport change, always 225-235 ms later. It adds nothing and costs latency. Use the transport only.
- The buds clamp at 0 and 127 (run B went `119 -> 127`), so parking is required outside Volume mode, as on macOS.
- Report rate limit: consecutive real reports were at least about 280-320 ms apart (A 302 ms, B 278 ms, D 319 ms). The same rate limit as macOS; fast spins can't be resolved better than that.

## Parking

- **Run C (park sink to 0.5):** works, but the park only happens after the sink reports the change, so it is written about 245 ms after the turn. That is close to the 280-320 ms report gap. The transport also reports the park (`63`, delta -48 or similar) and that is *not* flagged as an echo by the probe, because only the sink side was filtered. The park value lands at 63 (0.5 * 127 truncated), not 64.
- **Run D (park transport to 64):** the park is written 1-2 ms after the transport report. The echo (`64`) arrives 8-35 ms later. The sink never moved after the first event, so there is no audible volume blip. Direction was correct on every detent (`71` = right, `56` = left).
- Park at 64 on the transport. Treat any transport report within 400 ms of our write that lands within +/-2 of 64 as an echo.
- The first turn after starting the probe is measured against whatever the volume was left at, so the daemon must start by parking.

## Press and double press

- evdev device `/dev/input/event12` ("CMF Buds Pro 2 (AVRCP)"), delivered as down/up pairs about 200 ms apart for play/pause and the same millisecond for next.
- A press arrives as `KEY_PLAYCD` or `KEY_PAUSECD` depending on what the buds think the playback state is (they alternate: PAUSECD, PLAYCD, PAUSECD...). Treat both, plus `KEY_PLAYPAUSE` for safety, as **press**.
- A double press arrives as `KEY_NEXTSONG` only. No stray press event precedes it, so the buds do the double-click detection themselves. The window is inside the buds, so press latency is not measurable from the log.
- Reading the device needs the ACL (`setfacl`), which resets on reboot. The daemon needs a udev rule or the `input` group. Needs Art's approval.

## Not settled

1. **Do media players react to the keys?** The logs don't say whether Spotify or the browser paused or skipped in run B. Art needs to say. If they did, we grab the device with `EVIOCGRAB`, which is only tested by trying it.
2. **MPRIS:** not probed. evdev works, so MPRIS isn't needed.
3. **Reconnect:** the buds stayed connected for all four runs. The sink name stayed `bluez_output.3C_B0_ED_33_AA_74.1` and the transport path stayed `.../sep1/fd0`, but a reconnect or a case-lid cycle has not been tried. Plan for the daemon to watch `InterfacesAdded` and `InterfacesRemoved` for `MediaTransport1`, and to re-find the evdev device by name when it vanishes.
4. **Pre-park volume:** the buds' volume is left at 64 after Run D. Restoring the user's real volume when leaving a mode is a design decision for Phase 1.

## Recommended capture path

- **Turn:** BlueZ `MediaTransport1.Volume` via D-Bus (`zbus`), park with a transport write of 64 in the same handler, echo window 400 ms / +/-2.
- **Press, double press:** evdev read of the AVRCP device, found by name, with `EVIOCGRAB` if (1) above turns out to be a problem.
- **Do not use** the sink volume as an input.
- Simulated-input source (hotkeys) must exist so UI work doesn't depend on the buds.
