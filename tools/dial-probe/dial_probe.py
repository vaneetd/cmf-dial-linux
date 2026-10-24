#!/usr/bin/env python3
"""dial-probe (Linux): logs every signal the CMF Buds Pro 2 dial can produce.

Three sources, all with millisecond timestamps:
  [bluez]  org.bluez PropertiesChanged on the system bus (MediaTransport1.Volume 0-127,
           Device1.Connected, MediaPlayer1, ...)
  [sink]   PipeWire/PulseAudio sink volume of Bluetooth sinks, via `pactl subscribe`
  [evdev]  key events from BlueZ's virtual AVRCP input device (press, double press)

Options:
  --park 0.5            after each turn seen on the sink, write that sink volume back
                        (echo of our own write is flagged and ignored, like the macOS probe)
  --park-transport 64   same, but write MediaTransport1.Volume (0-127) over D-Bus
  --evdev-match TEXT    extra substring for input device names (default: cmf, buds, avrcp)
  --no-evdev            skip the evdev source

Needs only python3, dbus-python, PyGObject and pactl (all present on Pop!_OS 24.04).
"""
import argparse
import json
import os
import struct
import subprocess
import sys
import time

import dbus
import dbus.mainloop.glib
from gi.repository import GLib

T0 = time.time()
ECHO_WINDOW = 0.4    # seconds after a write in which a matching value is our own echo
ECHO_TOLERANCE = 0.032


def log(tag, msg):
    now = time.time()
    stamp = time.strftime("%H:%M:%S", time.localtime(now)) + ".%03d" % int((now % 1) * 1000)
    print(f"{stamp} +{now - T0:8.3f}s [{tag:6}] {msg}", flush=True)


# ---------------------------------------------------------------- BlueZ over D-Bus

class BluezWatcher:
    def __init__(self, bus, park_transport):
        self.bus = bus
        self.park_transport = park_transport
        self.last_write = (0.0, None)
        self.last_transport_volume = {}
        bus.add_signal_receiver(self.on_props, signal_name="PropertiesChanged",
                                dbus_interface="org.freedesktop.DBus.Properties",
                                path_keyword="path", sender_keyword="sender",
                                bus_name="org.bluez")
        bus.add_signal_receiver(self.on_added, signal_name="InterfacesAdded",
                                dbus_interface="org.freedesktop.DBus.ObjectManager",
                                bus_name="org.bluez")
        bus.add_signal_receiver(self.on_removed, signal_name="InterfacesRemoved",
                                dbus_interface="org.freedesktop.DBus.ObjectManager",
                                bus_name="org.bluez")
        self.dump_existing()

    def dump_existing(self):
        try:
            mgr = dbus.Interface(self.bus.get_object("org.bluez", "/"),
                                 "org.freedesktop.DBus.ObjectManager")
            objects = mgr.GetManagedObjects()
        except dbus.DBusException as e:
            log("bluez", f"BlueZ not reachable yet: {e.get_dbus_message()}")
            return
        for path, ifaces in objects.items():
            dev = ifaces.get("org.bluez.Device1")
            if dev:
                log("bluez", f"device {path} name={str(dev.get('Name'))!r} "
                             f"connected={bool(dev.get('Connected'))} paired={bool(dev.get('Paired'))}")
            tr = ifaces.get("org.bluez.MediaTransport1")
            if tr:
                log("bluez", f"transport {path} state={tr.get('State')} "
                             f"codec={tr.get('Codec')} volume={tr.get('Volume')}")

    def on_added(self, path, ifaces):
        for name, props in ifaces.items():
            if name.startswith("org.bluez.") and name not in ("org.bluez.GattManager1",):
                log("bluez", f"+iface {name} at {path} {self.fmt(props)}")

    def on_removed(self, path, ifaces):
        for name in ifaces:
            if name.startswith("org.bluez."):
                log("bluez", f"-iface {name} at {path}")

    @staticmethod
    def fmt(props):
        return {str(k): (int(v) if isinstance(v, (dbus.Byte, dbus.UInt16, dbus.Int16, dbus.UInt32))
                         else str(v)) for k, v in props.items()}

    def on_props(self, iface, changed, invalidated, path=None, sender=None):
        if not iface.startswith("org.bluez."):
            return
        if iface == "org.bluez.MediaTransport1" and "Volume" in changed:
            vol = int(changed["Volume"])
            prev = self.last_transport_volume.get(str(path))
            self.last_transport_volume[str(path)] = vol
            written_at, written = self.last_write
            echo = (written is not None and time.time() - written_at < ECHO_WINDOW
                    and abs(vol - written) <= 2)
            delta = "" if prev is None else f" delta={vol - prev:+d}"
            log("bluez", f"TRANSPORT VOLUME {vol}/127 ({vol / 127:.4f}){delta}"
                         f"{'  [echo of our write, ignored]' if echo else ''} {path}")
            if not echo and self.park_transport is not None:
                self.write_transport(path, self.park_transport)
            return
        log("bluez", f"{iface.split('.')[-1]} {path} {self.fmt(changed)}"
                     + (f" invalidated={list(map(str, invalidated))}" if invalidated else ""))

    def write_transport(self, path, value):
        try:
            props = dbus.Interface(self.bus.get_object("org.bluez", path),
                                   "org.freedesktop.DBus.Properties")
            self.last_write = (time.time(), value)
            props.Set("org.bluez.MediaTransport1", "Volume", dbus.UInt16(value))
            log("park", f"wrote MediaTransport1.Volume={value}")
        except dbus.DBusException as e:
            log("park", f"transport write failed: {e.get_dbus_message()}")


# ---------------------------------------------------------------- PipeWire / Pulse sinks

class SinkWatcher:
    def __init__(self, park):
        self.park = park
        self.state = {}          # sink name -> (volume, mute)
        self.default = None
        self.last_write = (0.0, None)
        self.proc = None
        self.start()

    def start(self):
        try:
            self.proc = subprocess.Popen(["pactl", "subscribe"], stdout=subprocess.PIPE,
                                         text=True, bufsize=1)
        except FileNotFoundError:
            log("sink", "pactl not found, sink watcher disabled")
            return
        GLib.io_add_watch(self.proc.stdout, GLib.IO_IN | GLib.IO_HUP, self.on_line)
        self.refresh(initial=True)

    def pactl_json(self, *args):
        out = subprocess.run(["pactl", "--format=json", *args], capture_output=True, text=True)
        return json.loads(out.stdout) if out.returncode == 0 and out.stdout else None

    @staticmethod
    def is_bluez(sink):
        props = sink.get("properties", {})
        return props.get("device.api") == "bluez5" or sink.get("name", "").startswith("bluez_")

    @staticmethod
    def volume(sink):
        vols = [ch["value"] / 65536 for ch in sink.get("volume", {}).values()]
        return sum(vols) / len(vols) if vols else None

    def on_line(self, source, cond):
        line = source.readline()
        if not line:
            log("sink", "pactl subscribe ended")
            return False
        if "'change' on sink" in line or "'new' on sink" in line or "'remove' on sink" in line \
                or "on server" in line:
            self.refresh()
        return True

    def refresh(self, initial=False):
        sinks = self.pactl_json("list", "sinks") or []
        info = self.pactl_json("info") or {}
        default = info.get("default_sink_name")
        if default != self.default:
            log("sink", f"default output -> {default}")
            self.default = default
        seen = set()
        for sink in sinks:
            if not self.is_bluez(sink):
                continue
            name = sink["name"]
            seen.add(name)
            vol, mute = self.volume(sink), sink.get("mute")
            prev = self.state.get(name)
            self.state[name] = (vol, mute)
            if initial or prev is None:
                log("sink", f"bluetooth sink {name} desc={sink.get('description')!r} "
                            f"volume={vol:.4f} mute={mute}")
                continue
            if prev == (vol, mute):
                continue
            self.on_change(name, prev, (vol, mute))
        for gone in set(self.state) - seen:
            log("sink", f"bluetooth sink gone: {gone}")
            del self.state[gone]

    def on_change(self, name, prev, cur):
        (pv, pm), (v, m) = prev, cur
        if m != pm:
            log("sink", f"MUTE {pm} -> {m} on {name}")
        if pv is None or v is None or abs(v - pv) < 1e-4:
            return
        written_at, written = self.last_write
        if written is not None and time.time() - written_at < ECHO_WINDOW \
                and abs(v - written) <= ECHO_TOLERANCE:
            log("sink", f"volume {v:.4f} [echo of our write, ignored]")
            return
        direction = "RIGHT/up" if v > pv else "LEFT/down"
        log("sink", f"SINK VOLUME {pv:.4f} -> {v:.4f} delta={v - pv:+.4f} "
                    f"steps~{round(abs(v - pv) / 0.0625)} {direction}")
        if self.park is not None:
            self.last_write = (time.time(), self.park)
            subprocess.run(["wpctl", "set-volume", f"@DEFAULT_AUDIO_SINK@" if name == self.default
                            else name, f"{self.park:.4f}"], capture_output=True)
            log("park", f"wrote sink volume {self.park:.4f}")


# ---------------------------------------------------------------- evdev (AVRCP passthrough)

EV_KEY = 0x01
KEY_NAMES = {113: "KEY_MUTE", 114: "KEY_VOLUMEDOWN", 115: "KEY_VOLUMEUP", 119: "KEY_PAUSE",
             128: "KEY_STOP", 163: "KEY_NEXTSONG", 164: "KEY_PLAYPAUSE", 165: "KEY_PREVIOUSSONG",
             166: "KEY_STOPCD", 200: "KEY_PLAYCD", 201: "KEY_PAUSECD", 207: "KEY_PLAY",
             208: "KEY_FASTFORWARD", 168: "KEY_REWIND"}
EVENT_FMT = "llHHi"
EVENT_SIZE = struct.calcsize(EVENT_FMT)


class EvdevWatcher:
    def __init__(self, patterns):
        self.patterns = [p.lower() for p in patterns]
        self.open = {}        # event path -> (fd, source id)
        self.denied = set()
        self.scan()
        GLib.timeout_add_seconds(2, self.scan)

    def device_name(self, event):
        try:
            with open(f"/sys/class/input/{event}/device/name") as f:
                return f.read().strip()
        except OSError:
            return ""

    def scan(self):
        present = set()
        for event in sorted(os.listdir("/dev/input")):
            if not event.startswith("event"):
                continue
            name = self.device_name(event)
            if not any(p in name.lower() for p in self.patterns):
                continue
            path = f"/dev/input/{event}"
            present.add(path)
            if path in self.open:
                continue
            try:
                fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
            except PermissionError:
                if path not in self.denied:
                    self.denied.add(path)
                    log("evdev", f"{path} ({name!r}) found but not readable. Run once:  "
                                 f"sudo setfacl -m u:$USER:r {path}")
                continue
            sid = GLib.io_add_watch(fd, GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR, self.on_data, path)
            self.open[path] = (fd, sid)
            self.denied.discard(path)
            log("evdev", f"listening on {path} ({name!r})")
        for path in set(self.open) - present:
            fd, sid = self.open.pop(path)
            GLib.source_remove(sid)
            os.close(fd)
            log("evdev", f"{path} disappeared")
        return True

    def on_data(self, fd, cond, path):
        if cond & (GLib.IO_HUP | GLib.IO_ERR):
            return False
        try:
            data = os.read(fd, EVENT_SIZE * 32)
        except (BlockingIOError, OSError):
            return True
        for off in range(0, len(data) - EVENT_SIZE + 1, EVENT_SIZE):
            _, _, etype, code, value = struct.unpack_from(EVENT_FMT, data, off)
            if etype != EV_KEY:
                continue
            state = {0: "up", 1: "DOWN", 2: "repeat"}.get(value, str(value))
            log("evdev", f"{KEY_NAMES.get(code, f'KEY_{code}')} {state}  ({path})")
        return True


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--park", type=float, metavar="V", help="park sink volume at V (0-1)")
    ap.add_argument("--park-transport", type=int, metavar="N", help="park MediaTransport1.Volume at N (0-127)")
    ap.add_argument("--evdev-match", action="append", default=[], metavar="TEXT")
    ap.add_argument("--no-evdev", action="store_true")
    args = ap.parse_args()

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    log("probe", f"dial-probe starting, pid {os.getpid()}, "
                 f"session={os.environ.get('XDG_SESSION_TYPE')} desktop={os.environ.get('XDG_CURRENT_DESKTOP')}")
    log("probe", "lid must be CLOSED for the dial to send anything. Ctrl-C to stop.")

    BluezWatcher(dbus.SystemBus(), args.park_transport)
    SinkWatcher(args.park)
    if not args.no_evdev:
        EvdevWatcher(["cmf", "buds", "avrcp"] + args.evdev_match)

    try:
        GLib.MainLoop().run()
    except KeyboardInterrupt:
        log("probe", "bye")


if __name__ == "__main__":
    sys.exit(main())
