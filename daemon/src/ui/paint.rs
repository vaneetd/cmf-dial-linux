//! Software painting of the dial and the HUD pill with tiny-skia, following
//! `../dial/design/DESIGN.md` (dark theme). The dial is drawn in a 260 x 260 space.

use std::f32::consts::PI;

use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, LineCap, LineJoin, Mask, MaskType, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, RadialGradient, Rect, SpreadMode, Stroke, Transform,
};

use crate::event::ModeId;

pub const SURFACE_W: u32 = 300;
pub const SURFACE_H: u32 = 292;
/// Where the 260 x 260 dial space sits in the surface.
const DIAL_X: f32 = 20.0;
const DIAL_Y: f32 = 12.0;

const GAP_DEG: f32 = 48.0;
pub const SEGMENT_DEG: f32 = (360.0 - GAP_DEG) / 9.0;
const START_DEG: f32 = GAP_DEG / 2.0;
const C: f32 = 130.0;

fn rgb(hex: u32) -> Color {
    Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255)
}
fn rgba(hex: u32, a: f32) -> Color {
    let c = rgb(hex);
    Color::from_rgba(c.red(), c.green(), c.blue(), a.clamp(0.0, 1.0)).unwrap()
}

mod col {
    pub const RING: u32 = 0x2a2c31;
    pub const WELL: u32 = 0x141518;
    pub const DIVIDER: u32 = 0x3a3c42;
    pub const SEGMENT_ACTIVE: u32 = 0x4a4d54;
    pub const HATCH: u32 = 0x5c5f67;
    pub const ICON_IDLE: u32 = 0x9d9fa5;
    pub const ICON_ACTIVE: u32 = 0xffffff;
    pub const KNOB_FACE: u32 = 0xf3f3f1;
    pub const KNOB_SHADE: u32 = 0xd6d6d2;
    pub const KNOB_KNURL: u32 = 0xb9b9b4;
    pub const KNOB_INK: u32 = 0x26304f;
    pub const GLOW: u32 = 0xff7b3d;
    pub const INK: u32 = 0xe8e9ec;
    pub const INK_MUTED: u32 = 0x8e9197;
    pub const CARD: u32 = 0x26282d;
    pub const CARD_BORDER: u32 = 0x3a3d44;
    pub const HAIRLINE: u32 = 0x2f3136;
}

pub struct Fonts {
    pub label: fontdue::Font,
    pub mono: fontdue::Font,
}

impl Fonts {
    pub fn load() -> Option<Fonts> {
        let read = |paths: &[&str]| {
            paths.iter().find_map(|p| {
                let bytes = std::fs::read(p).ok()?;
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
            })
        };
        Some(Fonts {
            label: read(&[
                "/usr/share/fonts/opentype/fira/FiraSans-Medium.otf",
                "/usr/share/fonts/opentype/fira/FiraSans-Bold.otf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            ])?,
            mono: read(&[
                "/usr/share/fonts/opentype/fira/FiraMono-Medium.otf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            ])?,
        })
    }
}

/// Everything the painter needs, already animated by the caller.
pub struct Scene {
    /// 0 = dial folded away, 1 = fully summoned.
    pub dial: f32,
    pub highlight_deg: f32,
    pub knob_deg: f32,
    pub selected: ModeId,
    /// 0..1, the press animation (knob sinks, glow pulses).
    pub press: f32,
    pub hud: Option<Hud>,
}

pub struct Hud {
    pub mode: ModeId,
    pub text: String,
    /// 0..1 meter for level modes.
    pub level: Option<f32>,
    /// 0..1 appearance progress.
    pub t: f32,
}

pub fn segment_centre(index: usize) -> f32 {
    START_DEG + SEGMENT_DEG * (index as f32 + 0.5)
}

pub fn mode_index(mode: ModeId) -> usize {
    ModeId::ALL.iter().position(|&m| m == mode).unwrap_or(0)
}

fn polar(r: f32, deg: f32) -> (f32, f32) {
    let a = deg.to_radians();
    (C + r * a.sin(), C - r * a.cos())
}

fn sector(r0: f32, r1: f32, a0: f32, a1: f32) -> Path {
    let mut pb = PathBuilder::new();
    let steps = (((a1 - a0).abs() / 2.0).ceil() as usize).max(1);
    let (x, y) = polar(r1, a0);
    pb.move_to(x, y);
    for i in 1..=steps {
        let (x, y) = polar(r1, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    for i in (0..=steps).rev() {
        let (x, y) = polar(r0, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish().unwrap()
}

/// A sector with rounded corners: inset by `rad`, filled, then stroked `2 * rad` wide with round joins.
fn rounded_sector(r0: f32, r1: f32, a0: f32, a1: f32, rad: f32) -> (Path, f32) {
    let inset = (rad / ((r0 + r1) / 2.0)).asin().to_degrees();
    (sector(r0 + rad, r1 - rad, a0 + inset, a1 - inset), rad * 2.0)
}

fn fill_rounded(pm: &mut Pixmap, tf: Transform, shape: &(Path, f32), paint: &Paint) {
    pm.fill_path(&shape.0, paint, FillRule::Winding, tf, None);
    let stroke = Stroke { width: shape.1, line_join: LineJoin::Round, ..Stroke::default() };
    pm.stroke_path(&shape.0, paint, &stroke, tf, None);
}

fn solid(color: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color);
    p.anti_alias = true;
    p
}

fn circle(cx: f32, cy: f32, r: f32) -> Path {
    PathBuilder::from_circle(cx, cy, r).unwrap()
}

/// Cheap soft shadow: stacked translucent shapes growing outwards.
fn soft_shadow_circle(pm: &mut Pixmap, tf: Transform, cx: f32, cy: f32, r: f32, dy: f32, blur: f32, alpha: f32) {
    let layers = 8;
    for i in 0..layers {
        let k = i as f32 / layers as f32;
        let path = circle(cx, cy + dy, r + blur * (1.0 - k) * 0.6);
        pm.fill_path(&path, &solid(rgba(0x000000, alpha / layers as f32 * 1.4)), FillRule::Winding, tf, None);
    }
}

/// Draws frames. The parts of the dial that never change (ring, selected-segment sprite, knob
/// face) are rendered once per scale and composited each frame, so a frame costs about a millisecond.
pub struct Painter {
    pub fonts: Fonts,
    scale: f32,
    ring: Option<Pixmap>,
    sel: Option<Pixmap>,
    knob: Option<Pixmap>,
    dial_layer: Option<Pixmap>,
    hud_layer: Option<Pixmap>,
}

impl Painter {
    pub fn new(fonts: Fonts) -> Self {
        Painter { fonts, scale: 0.0, ring: None, sel: None, knob: None, dial_layer: None, hud_layer: None }
    }

    fn prepare(&mut self, w: u32, h: u32, scale: f32) {
        if self.scale == scale && self.ring.as_ref().is_some_and(|r| r.width() == w && r.height() == h) {
            return;
        }
        self.scale = scale;
        let tf = Transform::from_scale(scale, scale).pre_translate(DIAL_X, DIAL_Y);
        self.ring = Some(build_ring(w, h, tf));
        self.sel = Some(build_selected(w, h, tf));
        self.knob = Some(build_knob(w, h, tf));
        self.dial_layer = Pixmap::new(w, h);
        self.hud_layer = Pixmap::new(w, h);
    }

    pub fn paint(&mut self, pm: &mut Pixmap, scale: f32, s: &Scene) {
        self.prepare(pm.width(), pm.height(), scale);
        pm.fill(Color::TRANSPARENT);
        let base = Transform::from_scale(scale, scale);

        if s.dial > 0.01 {
            let mut layer = self.dial_layer.take().unwrap();
            layer.fill(Color::TRANSPARENT);
            self.paint_dial(&mut layer, scale, s);
            // Summon: from above, 70% to 100% scale around the top centre, fading in.
            let t = ease_out(s.dial);
            let sc = 0.7 + 0.3 * t;
            let (px, py) = (C + DIAL_X, DIAL_Y);
            let tf = base
                .pre_translate(px, py - 28.0 * (1.0 - t))
                .pre_scale(sc, sc)
                .pre_translate(-px, -py)
                .pre_scale(1.0 / scale, 1.0 / scale);
            let paint = PixmapPaint { opacity: t, quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
            pm.draw_pixmap(0, 0, layer.as_ref(), &paint, tf, None);
            self.dial_layer = Some(layer);
        }
        if let Some(h) = &s.hud {
            let mut layer = self.hud_layer.take().unwrap();
            layer.fill(Color::TRANSPARENT);
            paint_hud(pm, &mut layer, base, scale, &self.fonts, h);
            self.hud_layer = Some(layer);
        }
    }

    fn paint_dial(&self, pm: &mut Pixmap, scale: f32, s: &Scene) {
        let tf = Transform::from_scale(scale, scale).pre_translate(DIAL_X, DIAL_Y);
        let centre = ((DIAL_X + C) * scale, (DIAL_Y + C) * scale);
        let sprite = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
        let plain = PixmapPaint::default();

        pm.draw_pixmap(0, 0, self.ring.as_ref().unwrap().as_ref(), &plain, Transform::identity(), None);

        // Selected segment: the sprite is drawn at segment 0 and rotated into place.
        let rot = Transform::from_rotate_at(s.highlight_deg - segment_centre(0), centre.0, centre.1);
        pm.draw_pixmap(0, 0, self.sel.as_ref().unwrap().as_ref(), &sprite, rot, None);

        // Icons, always upright.
        for (i, mode) in ModeId::ALL.iter().enumerate() {
            let (x, y) = polar(90.0, segment_centre(i));
            let colour = if *mode == s.selected { col::ICON_ACTIVE } else { col::ICON_IDLE };
            super::icons::draw(pm, tf, *mode, x, y, 18.0, rgb(colour), 1.5);
        }

        // Knob
        let press = press_curve(s.press);
        let sink = 1.0 - 0.08 * press;
        let shadow_dy = 8.0 - 6.0 * press;
        soft_shadow_circle(pm, tf, C, C, 49.0 * sink, shadow_dy, 8.0, 0.40);
        if s.press > 0.0 {
            let r = 49.0 * (1.0 + 0.24 * s.press);
            let stroke = Stroke { width: 2.0, ..Stroke::default() };
            pm.stroke_path(&circle(C, C, r), &solid(rgba(col::GLOW, 0.9 * (1.0 - s.press))), &stroke, tf, None);
        }
        let ktf = Transform::from_translate(centre.0, centre.1).pre_scale(sink, sink).pre_translate(-centre.0, -centre.1);
        pm.draw_pixmap(0, 0, self.knob.as_ref().unwrap().as_ref(), &sprite, ktf, None);

        // Pointer marks: 0 and 180 degrees relative to the knob angle.
        let ltf = tf.pre_translate(C, C).pre_scale(sink, sink).pre_translate(-C, -C);
        let mut marks = PathBuilder::new();
        for off in [0.0, 180.0] {
            let (x0, y0) = polar(34.0, s.knob_deg + off);
            let (x1, y1) = polar(41.0, s.knob_deg + off);
            marks.move_to(x0, y0);
            marks.line_to(x1, y1);
        }
        pm.stroke_path(
            &marks.finish().unwrap(),
            &solid(rgb(col::KNOB_INK)),
            &Stroke { width: 2.0, line_cap: LineCap::Round, ..Stroke::default() },
            ltf,
            None,
        );

        // Label: icon and short name, upright.
        super::icons::draw(pm, ltf, s.selected, C, 116.0, 18.0, rgb(col::KNOB_INK), 1.5);
        draw_text(pm, &self.fonts.label, 10.0 * scale, short_label(s.selected), centre.0, (DIAL_Y + 141.0 + 3.5) * scale, col::KNOB_INK, 1.0, Align::Centre);
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Ring shadow, ring body, well and dividers.
fn build_ring(w: u32, h: u32, tf: Transform) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    for i in 0..6 {
        let k = i as f32 / 6.0;
        let shape = rounded_sector(68.0 - 10.0 * k, 112.0 + 10.0 * k, START_DEG, 360.0 - START_DEG, 10.0);
        fill_rounded(&mut pm, tf.pre_translate(0.0, 10.0), &shape, &solid(rgba(0x000000, 0.07)));
    }
    let ring = rounded_sector(68.0, 112.0, START_DEG, 360.0 - START_DEG, 10.0);
    fill_rounded(&mut pm, tf, &ring, &solid(rgb(col::RING)));
    pm.fill_path(&circle(C, C, 62.0), &solid(rgb(col::WELL)), FillRule::Winding, tf, None);
    let divider = solid(rgb(col::DIVIDER));
    for i in 1..9 {
        let a = START_DEG + SEGMENT_DEG * i as f32;
        let (x0, y0) = polar(75.0, a);
        let (x1, y1) = polar(105.0, a);
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        pm.stroke_path(&pb.finish().unwrap(), &divider, &Stroke { width: 1.0, ..Stroke::default() }, tf, None);
    }
    pm
}

/// The lifted, hatched segment at position 0.
fn build_selected(w: u32, h: u32, tf: Transform) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    let sel = rounded_sector(67.0, 118.0, START_DEG, START_DEG + SEGMENT_DEG, 6.0);
    fill_rounded(&mut pm, tf, &sel, &solid(rgb(col::SEGMENT_ACTIVE)));
    if let Some(mask) = shape_mask(w, h, tf, &sel) {
        let mut pb = PathBuilder::new();
        let mut off = -260.0;
        while off < 260.0 {
            pb.move_to(off, 260.0);
            pb.line_to(off + 260.0, 0.0);
            off += 5.0;
        }
        pm.stroke_path(&pb.finish().unwrap(), &solid(rgba(col::HATCH, 0.6)), &Stroke { width: 1.6, ..Stroke::default() }, tf, Some(&mask));
    }
    pm
}

/// Knurled band, face gradient, inner rim and the upper-left rim light.
fn build_knob(w: u32, h: u32, tf: Transform) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    pm.fill_path(&circle(C, C, 49.0), &solid(rgb(col::KNOB_KNURL)), FillRule::Winding, tf, None);
    let mut teeth = PathBuilder::new();
    for i in 0..90 {
        let a = i as f32 * 4.0;
        let (x0, y0) = polar(45.4, a);
        let (x1, y1) = polar(48.4, a);
        teeth.move_to(x0, y0);
        teeth.line_to(x1, y1);
    }
    pm.stroke_path(&teeth.finish().unwrap(), &solid(rgb(col::KNOB_FACE)), &Stroke { width: 1.1, ..Stroke::default() }, tf, None);

    let focus = Point::from_xy(C - 44.0 * 0.24, C - 44.0 * 0.36);
    let grad = RadialGradient::new(
        focus,
        0.0,
        focus,
        44.0 * 1.35,
        vec![GradientStop::new(0.0, rgb(col::KNOB_FACE)), GradientStop::new(1.0, rgb(col::KNOB_SHADE))],
        SpreadMode::Pad,
        Transform::identity(),
    );
    match grad {
        Some(g) => {
            let mut p = Paint::default();
            p.shader = g;
            p.anti_alias = true;
            pm.fill_path(&circle(C, C, 44.0), &p, FillRule::Winding, tf, None);
        }
        None => pm.fill_path(&circle(C, C, 44.0), &solid(rgb(col::KNOB_FACE)), FillRule::Winding, tf, None),
    }
    pm.stroke_path(&circle(C, C, 38.0), &solid(rgb(col::KNOB_SHADE)), &Stroke { width: 1.0, ..Stroke::default() }, tf, None);

    let mut arc = PathBuilder::new();
    let mut a = 280.0;
    let (x, y) = polar(43.5, a);
    arc.move_to(x, y);
    while a < 345.0 {
        a += 3.0;
        let (x, y) = polar(43.5, a.min(345.0));
        arc.line_to(x, y);
    }
    pm.stroke_path(
        &arc.finish().unwrap(),
        &solid(rgba(col::GLOW, 0.55)),
        &Stroke { width: 2.0, line_cap: LineCap::Round, ..Stroke::default() },
        tf,
        None,
    );
    pm
}

fn press_curve(p: f32) -> f32 {
    // Down fast, back slower: 0 -> 1 -> 0 over the first 40% of the animation.
    if p <= 0.0 {
        0.0
    } else if p < 0.2 {
        p / 0.2
    } else {
        (1.0 - (p - 0.2) / 0.8).max(0.0)
    }
}

pub fn short_label(m: ModeId) -> &'static str {
    match m {
        ModeId::Volume => "Volume",
        ModeId::Brightness => "Bright",
        ModeId::Apps => "Apps",
        ModeId::Windows => "Windows",
        ModeId::Desktops => "Spaces",
        ModeId::Tabs => "Tabs",
        ModeId::Scroll => "Scroll",
        ModeId::UndoRedo => "Undo",
        ModeId::Slides => "Slides",
    }
}

fn shape_mask(w: u32, h: u32, tf: Transform, shape: &(Path, f32)) -> Option<Mask> {
    let mut tmp = Pixmap::new(w, h)?;
    fill_rounded(&mut tmp, tf, shape, &solid(Color::WHITE));
    Some(Mask::from_pixmap(tmp.as_ref(), MaskType::Alpha))
}

fn paint_hud(pm: &mut Pixmap, layer: &mut Pixmap, base: Transform, scale: f32, fonts: &Fonts, h: &Hud) {
    let t = ease_out(h.t);
    let label = h.mode.label();
    let label_w = text_width(&fonts.label, 12.0 * scale, label) / scale;
    let value_w = if h.level.is_some() {
        72.0 + 10.0 + text_width(&fonts.mono, 12.0 * scale, &h.text) / scale
    } else {
        text_width(&fonts.mono, 12.0 * scale, &h.text) / scale
    };
    let gap = if h.text.is_empty() && h.level.is_none() { 0.0 } else { 10.0 };
    let w = 14.0 + 16.0 + 8.0 + label_w + gap + value_w + 14.0;
    let ht = 34.0;
    let x = (SURFACE_W as f32 - w) / 2.0;
    let y = 8.0 - 8.0 * (1.0 - t);
    // 90% scale in, around the top centre.
    let cx = SURFACE_W as f32 / 2.0;
    let sc = 0.9 + 0.1 * t;
    let tf = base.pre_translate(cx, y).pre_scale(sc, sc).pre_translate(-cx, -y);

    let lt = Transform::from_scale(scale, scale);
    // shadow
    for i in 0..5 {
        let k = i as f32;
        let r = pill(x - k, y + 4.0 - k, w + 2.0 * k, ht + 2.0 * k, (ht + 2.0 * k) / 2.0);
        layer.fill_path(&r, &solid(rgba(0x000000, 0.045)), FillRule::Winding, lt, None);
    }
    let body = pill(x, y, w, ht, ht / 2.0);
    layer.fill_path(&body, &solid(rgb(col::CARD)), FillRule::Winding, lt, None);
    layer.stroke_path(&body, &solid(rgb(col::CARD_BORDER)), &Stroke { width: 1.0, ..Stroke::default() }, lt, None);

    let mid = y + ht / 2.0;
    super::icons::draw(layer, lt, h.mode, x + 14.0 + 8.0, mid, 16.0, rgb(col::INK), 1.5);
    let mut cursor = x + 14.0 + 16.0 + 8.0;
    draw_text(layer, &fonts.label, 12.0 * scale, label, cursor * scale, (mid + 4.2) * scale, col::INK, 1.0, Align::Left);
    cursor += label_w + gap;
    if let Some(level) = h.level {
        let track = pill(cursor, mid - 2.0, 72.0, 4.0, 2.0);
        layer.fill_path(&track, &solid(rgb(col::HAIRLINE)), FillRule::Winding, lt, None);
        let fill = pill(cursor, mid - 2.0, (72.0 * level.clamp(0.0, 1.0)).max(4.0), 4.0, 2.0);
        layer.fill_path(&fill, &solid(rgb(col::INK)), FillRule::Winding, lt, None);
        cursor += 72.0 + 10.0;
    }
    if !h.text.is_empty() {
        draw_text(layer, &fonts.mono, 12.0 * scale, &h.text, cursor * scale, (mid + 4.2) * scale, col::INK_MUTED, 1.0, Align::Left);
    }
    let paint = PixmapPaint { opacity: t, ..PixmapPaint::default() };
    pm.draw_pixmap(0, 0, layer.as_ref(), &paint, tf.pre_scale(1.0 / scale, 1.0 / scale), None);
}

pub fn pill(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    super::icons::rrect(x, y, w, h, r)
}

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Centre,
}

pub fn text_width(font: &fontdue::Font, px: f32, text: &str) -> f32 {
    text.chars().map(|c| font.metrics(c, px).advance_width).sum()
}

/// Draws `text` in physical pixels. `y` is the baseline.
pub fn draw_text(pm: &mut Pixmap, font: &fontdue::Font, px: f32, text: &str, x: f32, y: f32, hex: u32, alpha: f32, align: Align) {
    let width = text_width(font, px, text);
    let mut pen = match align {
        Align::Left => x,
        Align::Centre => x - width / 2.0,
    };
    let (r, g, b) = ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for c in text.chars() {
        let (m, bitmap) = font.rasterize(c, px);
        let gx = pen.round() as i32 + m.xmin;
        let gy = y.round() as i32 - m.height as i32 - m.ymin;
        for row in 0..m.height {
            for colm in 0..m.width {
                let cov = bitmap[row * m.width + colm] as f32 / 255.0 * alpha;
                if cov <= 0.0 {
                    continue;
                }
                let (px_, py_) = (gx + colm as i32, gy + row as i32);
                if px_ < 0 || py_ < 0 || px_ >= pw || py_ >= ph {
                    continue;
                }
                let i = ((py_ * pw + px_) * 4) as usize;
                // premultiplied "source over"
                let inv = 1.0 - cov;
                data[i] = (r as f32 * cov + data[i] as f32 * inv) as u8;
                data[i + 1] = (g as f32 * cov + data[i + 1] as f32 * inv) as u8;
                data[i + 2] = (b as f32 * cov + data[i + 2] as f32 * inv) as u8;
                data[i + 3] = (255.0 * cov + data[i + 3] as f32 * inv) as u8;
            }
        }
        pen += m.advance_width;
    }
}

#[allow(dead_code)]
fn _unused(_: BlendMode, _: Rect) -> f32 {
    PI
}
