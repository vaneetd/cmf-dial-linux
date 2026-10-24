# dial-probe (Linux)

Phase 0 logger. It answers one question: what does the CMF Buds Pro 2 dial look like to a Linux desktop?

```
python3 dial_probe.py                  # log only
python3 dial_probe.py --park 0.5       # also park the sink volume after each turn
python3 dial_probe.py --park-transport 64   # park the AVRCP absolute volume instead
```

No installs needed on Pop!_OS 24.04 (python3, dbus-python, PyGObject, pactl).

Sources: `[bluez]` MediaTransport1 Volume (0-127) and device state, `[sink]` Bluetooth sink volume from
PipeWire, `[evdev]` the buds' virtual AVRCP keyboard (press, double press).

## Runs

Pair and connect the buds, set Nothing X gestures (press = Play/Pause, double press = Next, rotate = Volume),
**close the case lid**, choose the buds as sound output. Save each log: `... | tee -i ~/probe-A.log`.

1. A: nothing playing. Turn right 5 slowly, left 5 slowly, right 5 fast, left 5 fast. Press once, press twice
   (pause 5 s between), double press once.
2. B: same, with Spotify or a browser playing audio.
3. C: `--park 0.5`, turn right 10 slowly, left 10 slowly, right 10 fast.
4. D: `--park-transport 64`, same as C.

If `[evdev]` says a device is "not readable", run the `setfacl` line it prints, then press again (it resets on reboot).
Say which of the three sources showed each gesture.
