//! Line icons for the nine modes, drawn from a 24 x 24 grid with round caps and joins.

use tiny_skia::{Color, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::event::ModeId;

pub fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = 0.5523 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}

/// Draws `mode`'s icon centred on (cx, cy), `size` points wide, `weight` points of stroke.
pub fn draw(pm: &mut Pixmap, tf: Transform, mode: ModeId, cx: f32, cy: f32, size: f32, color: Color, weight: f32) {
    let k = size / 24.0;
    let tf = tf.pre_translate(cx - size / 2.0, cy - size / 2.0).pre_scale(k, k);
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke { width: weight / k, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    let mut go = |p: Option<Path>| {
        if let Some(p) = p {
            pm.stroke_path(&p, &paint, &stroke, tf, None);
        }
    };
    match mode {
        ModeId::Volume => {
            let mut pb = PathBuilder::new();
            pb.move_to(3.5, 9.5);
            pb.line_to(7.5, 9.5);
            pb.line_to(12.5, 5.0);
            pb.line_to(12.5, 19.0);
            pb.line_to(7.5, 14.5);
            pb.line_to(3.5, 14.5);
            pb.close();
            go(pb.finish());
            let mut w1 = PathBuilder::new();
            w1.move_to(16.0, 9.0);
            w1.quad_to(18.2, 12.0, 16.0, 15.0);
            go(w1.finish());
            let mut w2 = PathBuilder::new();
            w2.move_to(18.6, 6.0);
            w2.quad_to(22.6, 12.0, 18.6, 18.0);
            go(w2.finish());
        }
        ModeId::Brightness => {
            go(PathBuilder::from_circle(12.0, 12.0, 4.0));
            let mut pb = PathBuilder::new();
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                pb.move_to(12.0 + 7.0 * a.sin(), 12.0 - 7.0 * a.cos());
                pb.line_to(12.0 + 9.5 * a.sin(), 12.0 - 9.5 * a.cos());
            }
            go(pb.finish());
        }
        ModeId::Apps => {
            for row in 0..3 {
                for col in 0..3 {
                    go(Some(rrect(3.5 + col as f32 * 6.5, 3.5 + row as f32 * 6.5, 4.5, 4.5, 1.2)));
                }
            }
        }
        ModeId::Windows => {
            go(Some(rrect(3.0, 4.0, 13.0, 10.0, 2.0)));
            go(Some(rrect(8.0, 10.0, 13.0, 10.0, 2.0)));
        }
        ModeId::Desktops => {
            go(Some(rrect(3.0, 3.5, 14.0, 10.0, 2.0)));
            go(Some(rrect(7.0, 10.5, 14.0, 10.0, 2.0)));
        }
        ModeId::Tabs => {
            go(Some(rrect(3.0, 4.5, 18.0, 15.0, 2.5)));
            let mut pb = PathBuilder::new();
            pb.move_to(3.0, 9.5);
            pb.line_to(21.0, 9.5);
            pb.move_to(9.5, 4.5);
            pb.line_to(9.5, 9.5);
            go(pb.finish());
        }
        ModeId::Scroll => {
            go(Some(rrect(6.5, 2.5, 11.0, 19.0, 5.5)));
            let mut pb = PathBuilder::new();
            pb.move_to(12.0, 7.0);
            pb.line_to(12.0, 10.5);
            go(pb.finish());
        }
        ModeId::UndoRedo => {
            let mut pb = PathBuilder::new();
            pb.move_to(9.0, 6.5);
            pb.line_to(4.5, 11.0);
            pb.line_to(9.0, 15.5);
            go(pb.finish());
            let mut arc = PathBuilder::new();
            arc.move_to(4.5, 11.0);
            arc.line_to(14.0, 11.0);
            arc.cubic_to(18.0, 11.0, 20.0, 13.5, 20.0, 16.0);
            arc.cubic_to(20.0, 18.5, 18.0, 20.0, 15.0, 20.0);
            go(arc.finish());
        }
        ModeId::Slides => {
            go(Some(rrect(3.0, 4.5, 18.0, 15.0, 2.5)));
            let mut pb = PathBuilder::new();
            pb.move_to(10.0, 9.0);
            pb.line_to(15.5, 12.0);
            pb.line_to(10.0, 15.0);
            pb.close();
            go(pb.finish());
        }
    }
}
