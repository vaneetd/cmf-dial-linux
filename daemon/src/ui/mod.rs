//! The wheel and the HUD: a Wayland layer-shell overlay, drawn in software.
//!
//! One surface, anchored top centre on the overlay layer, with no keyboard interactivity and an
//! empty input region, so it never takes focus or clicks from the app being controlled. It maps
//! only while something is showing. The window runs on its own thread and gets `UiMsg`s over a
//! calloop channel.

mod icons;
mod paint;

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Event, Sender};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use tiny_skia::Pixmap;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};

use crate::event::{ModeId, log};
use paint::{Fonts, Hud, Painter, Scene, SEGMENT_DEG, SURFACE_H, SURFACE_W};

const HUD_LINGER: Duration = Duration::from_millis(1200);
const PRESS_FOLD_DELAY: Duration = Duration::from_millis(170);
const PRESS_ANIM: Duration = Duration::from_millis(560);
const HUD_AFTER_FOLD: Duration = Duration::from_millis(290);
const GAP_DEG: f32 = 48.0;

#[derive(Debug)]
pub enum UiMsg {
    /// Open or move the wheel to this mode, or close it.
    Wheel(Option<ModeId>),
    /// The knob was pressed.
    Press,
    /// Show the HUD pill. `level` is 0..1 for level modes.
    Hud { mode: ModeId, text: String, level: Option<f32> },
}

#[derive(Clone)]
pub struct UiHandle(Sender<UiMsg>);

impl UiHandle {
    pub fn send(&self, msg: UiMsg) {
        let _ = self.0.send(msg);
    }
}

pub fn spawn() -> UiHandle {
    let (tx, rx) = channel::channel();
    std::thread::Builder::new()
        .name("dial-ui".into())
        .spawn(move || {
            if let Err(e) = run(rx) {
                log("ui", format!("stopped: {e}"));
            }
        })
        .expect("spawn ui thread");
    UiHandle(tx)
}

struct Hud_ {
    mode: ModeId,
    text: String,
    level: Option<f32>,
    show_at: Instant,
    hide_at: Instant,
    t: f32,
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    configured: bool,
    /// Showing a fully transparent frame. The surface is never unmapped: attaching a null buffer
    /// resets a layer surface's size and anchor, and the compositor then kills the connection.
    blank: bool,
    scale: i32,
    painter: Painter,

    selected: ModeId,
    highlight: f32,
    highlight_target: f32,
    knob: f32,
    dial: f32,
    dial_target: f32,
    press_at: Option<Instant>,
    close_at: Option<Instant>,
    hud: Option<Hud_>,
    last_tick: Instant,
    /// What the last frame showed. A frame is drawn only when this changes; otherwise the
    /// compositor's buffer-release events would keep waking us into redrawing forever.
    last_drawn: Option<[f32; 6]>,
    last_draw: Instant,
}

fn run(rx: Channel<UiMsg>) -> Result<(), Box<dyn std::error::Error>> {
    let fonts = Fonts::load().ok_or("no usable font found")?;
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| format!("no layer-shell: {e}"))?;
    let shm = Shm::bind(&globals, &qh)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("dial"), None);
    layer.set_anchor(Anchor::TOP);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(SURFACE_W, SURFACE_H);
    layer.set_exclusive_zone(0);
    layer.set_margin(6, 0, 0, 0);
    // An empty input region: every click and touch goes to the window underneath.
    let empty = Region::new(&compositor)?;
    layer.wl_surface().set_input_region(Some(empty.wl_region()));
    layer.commit();

    let pool = SlotPool::new((SURFACE_W * SURFACE_H * 4) as usize, &shm)?;
    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        configured: false,
        blank: false,
        scale: 1,
        painter: Painter::new(fonts),
        selected: ModeId::Volume,
        highlight: paint::segment_centre(0),
        highlight_target: paint::segment_centre(0),
        knob: paint::segment_centre(0),
        dial: 0.0,
        dial_target: 0.0,
        press_at: None,
        close_at: None,
        hud: None,
        last_tick: Instant::now(),
        last_drawn: None,
        last_draw: Instant::now(),
    };

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle())?;
    event_loop
        .handle()
        .insert_source(rx, |event, _, app| match event {
            Event::Msg(m) => app.on_msg(m),
            Event::Closed => {}
        })
        .map_err(|e| format!("channel: {e}"))?;
    log("ui", "overlay ready (layer-shell, overlay layer, no focus)");

    loop {
        let timeout = app.next_timeout();
        event_loop.dispatch(timeout, &mut app)?;
        if let Some(err) = conn.protocol_error() {
            return Err(format!("wayland protocol error: {err}").into());
        }
        app.tick(&qh);
    }
}

impl App {
    fn on_msg(&mut self, msg: UiMsg) {
        let now = Instant::now();
        self.last_drawn = None;
        match msg {
            UiMsg::Wheel(Some(mode)) => {
                self.close_at = None;
                self.hud = None;
                if self.dial_target < 0.5 {
                    // Opening: start with the highlight on the mode, no spin from the last time.
                    self.selected = mode;
                    let c = paint::segment_centre(paint::mode_index(mode));
                    self.highlight = c;
                    self.highlight_target = c;
                    self.knob = c;
                } else {
                    self.advance_to(mode);
                }
                self.dial_target = 1.0;
            }
            UiMsg::Wheel(None) => {
                let when = match self.press_at {
                    Some(p) if now.duration_since(p) < PRESS_FOLD_DELAY => p + PRESS_FOLD_DELAY,
                    _ => now,
                };
                self.close_at = Some(when);
            }
            UiMsg::Press => self.press_at = Some(now),
            UiMsg::Hud { mode, text, level } => {
                let delay = if self.dial_target > 0.5 || self.close_at.is_some() { HUD_AFTER_FOLD } else { Duration::ZERO };
                let show_at = now + delay;
                let t = self.hud.as_ref().filter(|h| h.mode == mode).map_or(0.0, |h| h.t);
                self.hud = Some(Hud_ { mode, text, level, show_at, hide_at: show_at + HUD_LINGER, t });
            }
        }
    }

    /// Moves the highlight one segment at a time the short way round, adding the gap when it
    /// crosses 12 o'clock, so the angle keeps accumulating and never spins backwards.
    fn advance_to(&mut self, to: ModeId) {
        let n = ModeId::ALL.len() as i32;
        let from_i = paint::mode_index(self.selected) as i32;
        let to_i = paint::mode_index(to) as i32;
        let fwd = (to_i - from_i).rem_euclid(n);
        if fwd == 0 {
            return;
        }
        let (dir, count) = if fwd <= n / 2 { (1, fwd) } else { (-1, n - fwd) };
        let mut i = from_i;
        for _ in 0..count {
            let next = (i + dir).rem_euclid(n);
            let crosses = (dir > 0 && next < i) || (dir < 0 && next > i);
            self.highlight_target += dir as f32 * (SEGMENT_DEG + if crosses { GAP_DEG } else { 0.0 });
            i = next;
        }
        self.selected = to;
    }

    fn press_progress(&self) -> f32 {
        match self.press_at {
            Some(p) => (p.elapsed().as_secs_f32() / PRESS_ANIM.as_secs_f32()).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    fn animating(&self) -> bool {
        let hud_moving = self.hud.as_ref().is_some_and(|h| {
            let now = Instant::now();
            let target = if now >= h.show_at && now < h.hide_at { 1.0 } else { 0.0 };
            h.t != target
        });
        (self.dial - self.dial_target).abs() > 0.002
            || (self.highlight - self.highlight_target).abs() > 0.05
            || self.press_at.is_some_and(|p| p.elapsed() < PRESS_ANIM)
            || hud_moving
    }

    fn next_timeout(&self) -> Option<Duration> {
        if self.animating() {
            return Some(Duration::from_millis(16));
        }
        let now = Instant::now();
        let mut next: Option<Instant> = self.close_at;
        if let Some(h) = &self.hud {
            let t = if now < h.show_at { h.show_at } else { h.hide_at };
            next = Some(next.map_or(t, |n| n.min(t)));
        }
        next.map(|t| t.saturating_duration_since(now).max(Duration::from_millis(1)))
    }

    fn tick(&mut self, qh: &QueueHandle<App>) {
        let now = Instant::now();
        // Buffer-release events wake the loop between frames; hold the pace to about 60 fps.
        if self.animating() && now.duration_since(self.last_draw) < Duration::from_millis(12) {
            return;
        }
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.1);
        self.last_tick = now;

        if self.close_at.is_some_and(|t| now >= t) {
            self.close_at = None;
            self.dial_target = 0.0;
        }
        let approach = |v: f32, target: f32, tau: f32| v + (target - v) * (1.0 - (-dt / tau).exp());
        self.dial = approach(self.dial, self.dial_target, 0.07);
        if (self.dial - self.dial_target).abs() < 0.004 {
            self.dial = self.dial_target;
        }
        self.highlight = approach(self.highlight, self.highlight_target, 0.045);
        if (self.highlight - self.highlight_target).abs() < 0.05 {
            self.highlight = self.highlight_target;
        }
        self.knob = self.highlight;
        if self.press_at.is_some_and(|p| p.elapsed() >= PRESS_ANIM) {
            self.press_at = None;
        }
        if let Some(h) = &mut self.hud {
            let visible = now >= h.show_at && now < h.hide_at;
            let target = if visible { 1.0 } else { 0.0 };
            h.t = approach(h.t, target, 0.05);
            if (h.t - target).abs() < 0.002 {
                h.t = target;
            }
            if now >= h.hide_at && h.t == 0.0 {
                self.hud = None;
            }
        }
        self.draw(qh);
    }

    fn visible(&self) -> bool {
        self.dial > 0.01 || self.hud.as_ref().is_some_and(|h| h.t > 0.01)
    }

    fn draw(&mut self, _qh: &QueueHandle<App>) {
        if !self.configured {
            return;
        }
        let key = [
            self.dial,
            self.highlight,
            self.press_progress(),
            self.hud.as_ref().map_or(-1.0, |h| h.t),
            self.hud.as_ref().map_or(0.0, |h| h.text.len() as f32 + h.level.unwrap_or(0.0)),
            paint::mode_index(self.selected) as f32,
        ];
        if self.last_drawn == Some(key) {
            return;
        }
        self.last_drawn = Some(key);
        self.last_draw = Instant::now();
        if !self.visible() {
            if !self.blank {
                self.present(&Pixmap::new(SURFACE_W, SURFACE_H).unwrap(), 1);
                self.blank = true;
            }
            return;
        }
        self.blank = false;
        let scale = self.scale.max(1);
        let (w, h) = (SURFACE_W as i32 * scale, SURFACE_H as i32 * scale);
        let Some(mut pm) = Pixmap::new(w as u32, h as u32) else { return };
        let scene = Scene {
            dial: self.dial,
            highlight_deg: self.highlight,
            knob_deg: self.knob,
            selected: self.selected,
            press: self.press_progress(),
            hud: self.hud.as_ref().filter(|h| h.t > 0.001).map(|h| Hud {
                mode: h.mode,
                text: h.text.clone(),
                level: h.level,
                t: h.t,
            }),
        };
        self.painter.paint(&mut pm, scale as f32, &scene);

        self.present(&pm, scale);
    }

    fn present(&mut self, pm: &Pixmap, scale: i32) {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let stride = w * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(w, h, stride, wl_shm::Format::Argb8888) else {
            return;
        };
        // tiny-skia is premultiplied RGBA; wl_shm Argb8888 is premultiplied BGRA bytes on little-endian.
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = src[3];
        }
        let surface = self.layer.wl_surface();
        let _ = surface.set_buffer_scale(scale);
        surface.damage_buffer(0, 0, w, h);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, new_factor: i32) {
        self.scale = new_factor.max(1);
        self.last_drawn = None;
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        log("ui", "layer surface closed by the compositor");
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let _ = NonZeroU32::new(configure.new_size.0);
        self.configured = true;
        self.last_drawn = None;
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_dispatch2!(App);

/// `--snapshot DIR`: renders the main states to PNGs (2x, on a dark ground) for design review.
pub fn snapshot(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let mut painter = Painter::new(Fonts::load().ok_or("no usable font found")?);
    {
        let mut pm = Pixmap::new(SURFACE_W, SURFACE_H).unwrap();
        let sc = scene(ModeId::Volume, 0.0, Some(("", "62", Some(0.6))));
        let t = Instant::now();
        for _ in 0..50 {
            painter.paint(&mut pm, 1.0, &sc);
        }
        println!("paint: {:?} per frame at 1x", t.elapsed() / 50);
    }
    let shots: Vec<(&str, Scene)> = vec![
        ("wheel-volume", scene(ModeId::Volume, 0.0, None)),
        ("wheel-windows", scene(ModeId::Windows, 0.0, None)),
        ("wheel-slides-press", scene(ModeId::Slides, 0.2, None)),
        ("hud-volume", scene(ModeId::Volume, 0.0, Some(("Volume", "62", Some(0.62))))),
        ("hud-scroll", scene(ModeId::Scroll, 0.0, Some(("Scroll", "↓ 6 lines", None)))),
        ("hud-undo", scene(ModeId::UndoRedo, 0.0, Some(("Undo / Redo", "Undo ×2", None)))),
    ];
    for (name, mut sc) in shots {
        sc.dial = if name.starts_with("wheel") { 1.0 } else { 0.0 };
        let scale = 2;
        let (w, h) = (SURFACE_W * scale, SURFACE_H * scale);
        let mut pm = Pixmap::new(w, h).unwrap();
        painter.paint(&mut pm, scale as f32, &sc);
        let mut bg = Pixmap::new(w, h).unwrap();
        bg.fill(tiny_skia::Color::from_rgba8(0x1e, 0x20, 0x24, 255));
        bg.draw_pixmap(0, 0, pm.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        bg.save_png(dir.join(format!("{name}.png")))?;
    }
    Ok(())
}

fn scene(mode: ModeId, press: f32, hud: Option<(&str, &str, Option<f32>)>) -> Scene {
    let c = paint::segment_centre(paint::mode_index(mode));
    let accumulated = if mode == ModeId::Slides { c } else { c };
    Scene {
        dial: 1.0,
        highlight_deg: accumulated,
        knob_deg: accumulated,
        selected: mode,
        press,
        hud: hud.map(|(_, text, level)| Hud { mode, text: text.into(), level, t: 1.0 }),
    }
}
