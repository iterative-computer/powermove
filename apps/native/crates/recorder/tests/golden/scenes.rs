use super::fonts;
use pm_recorder::*;
use std::f64::consts::{PI, TAU};
use std::sync::Arc;
pub const MODES: [&str; 26] = [
    "source-over",
    "source-in",
    "source-out",
    "source-atop",
    "destination-over",
    "destination-in",
    "destination-out",
    "destination-atop",
    "lighter",
    "copy",
    "xor",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];
pub fn record(name: &str) -> Canvas {
    let (w, h) = match name {
        "composites" => (26, 3),
        "analytic-circle" => (8, 8),
        "analytic-bilinear" => (12, 12),
        "analytic-rect" => (2, 2),
        "analytic-beziers" => (32, 12),
        "analytic-roundrect" => (16, 24),
        "analytic-ellipse" => (16, 12),
        _ => (32, 24),
    };
    let mut c = Canvas::new(7, w, h).unwrap();
    match name {
        "text" => {
            // The oracle is headless Chromium without macOS font smoothing:
            // plain coverage. Today's weight has its own gate (`text_weight`).
            c.set_text_smoothing(Smoothing::Exact);
            let mut font = fonts::engine();
            c.set_font("10px 'Recorder Test'").unwrap();
            c.fill_text(
                &"AB".encode_utf16().collect::<Vec<_>>(),
                2.,
                10.,
                None,
                &mut font,
            )
            .unwrap();
            c.fill_text(&[65, 65], 2., 22., Some(8.), &mut font)
                .unwrap();
            c.set_text_align(TextAlign::Center);
            c.set_text_baseline(TextBaseline::Top);
            c.fill_text(&[65], 24., 2., None, &mut font).unwrap();
            c.set_text_align(TextAlign::Left);
            c.set_text_baseline(TextBaseline::Alphabetic);
            c.stroke_text(&[66], 20., 22., None, &mut font).unwrap();
        }
        "analytic-beziers" => {
            c.move_to(1., 10.);
            c.bezier_curve_to(
                Point::new(1., 1.),
                Point::new(15., 1.),
                Point::new(15., 10.),
            );
            c.close_path();
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.move_to(17., 10.);
            c.quadratic_curve_to(Point::new(24., -4.), Point::new(31., 10.));
            c.close_path();
            c.fill(FillRule::NonZero).unwrap();
        }
        "analytic-roundrect" => {
            c.round_rect(1., 1., 14., 21., &[Vec2::new(4., 4.)])
                .unwrap();
            c.fill(FillRule::NonZero).unwrap();
        }
        "analytic-ellipse" => {
            c.ellipse(Point::new(8., 6.), Vec2::new(5., 3.), 0.35, 0., TAU, false)
                .unwrap();
            c.fill(FillRule::NonZero).unwrap();
        }
        "analytic-circle" => {
            c.arc(Point::new(4., 4.), 3.25, 0., TAU, false).unwrap();
            c.fill(FillRule::NonZero).unwrap();
        }
        "analytic-bilinear" => {
            c.draw_image(
                source(),
                Rect::new(0., 0., 2., 2.),
                Rect::new(0., 0., 12., 12.),
            )
            .unwrap();
        }
        "analytic-rect" => {
            c.set_fill_color("red").unwrap();
            c.fill_rect(0.25, 0.25, 1., 1.).unwrap();
        }
        "rects" => {
            c.set_fill_color("#e03010").unwrap();
            c.fill_rect(1., 1., 12., 10.).unwrap();
            c.set_fill_color("#2080e0").unwrap();
            c.set_global_alpha(0.5);
            c.fill_rect(7., 7., 16., 12.).unwrap();
            c.clear_rect(9., 9., 3., 3.).unwrap();
            c.set_global_alpha(1.);
            c.set_fill_color("#10b060").unwrap();
            c.fill_rect(25.25, 2.25, 4.5, 4.5).unwrap();
            c.set_stroke_color("black").unwrap();
            c.set_line_width(2.);
            c.stroke_rect(2., 15., 4., 6.).unwrap();
        }
        "curves" => {
            c.set_fill_color("#e03010").unwrap();
            c.move_to(1., 2.);
            c.line_to(15., 3.);
            c.line_to(3., 11.);
            c.close_path();
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.move_to(17., 2.);
            c.quadratic_curve_to(Point::new(30., 1.), Point::new(30., 11.));
            c.bezier_curve_to(
                Point::new(20., 16.),
                Point::new(25., 8.),
                Point::new(17., 2.),
            );
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.rect(2., 15., 12., 7.);
            c.rect(5., 17., 6., 3.);
            c.fill(FillRule::EvenOdd).unwrap();
        }
        "arcs" => {
            c.arc(Point::new(8., 8.), 5., 0., TAU, false).unwrap();
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.ellipse(Point::new(23., 7.), Vec2::new(6., 3.), 0.3, 0., TAU, false)
                .unwrap();
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.move_to(2., 21.);
            c.arc_to(Point::new(2., 14.), Point::new(13., 14.), 3.)
                .unwrap();
            c.line_to(13., 21.);
            c.close_path();
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.arc(Point::new(24., 19.), 4., PI, 0., true).unwrap();
            c.stroke().unwrap();
        }
        "roundrect" => {
            c.round_rect(1., 1., 14., 21., &[Vec2::new(4., 4.)])
                .unwrap();
            c.fill(FillRule::NonZero).unwrap();
            c.begin_path();
            c.round_rect(
                30.,
                22.,
                -13.,
                -20.,
                &[Vec2::new(2., 4.), Vec2::new(5., 3.)],
            )
            .unwrap();
            c.fill(FillRule::NonZero).unwrap();
        }
        "strokes" => {
            for (i, cap) in [Cap::Butt, Cap::Round, Cap::Square].iter().enumerate() {
                c.begin_path();
                c.set_line_cap(*cap);
                c.set_line_width(3.);
                c.move_to(3., 3. + i as f64 * 7.);
                c.line_to(12., 3. + i as f64 * 7.);
                c.stroke().unwrap();
            }
            for (i, join) in [Join::Miter, Join::Round, Join::Bevel].iter().enumerate() {
                c.begin_path();
                c.set_line_cap(Cap::Butt);
                c.set_line_join(*join);
                c.set_miter_limit(2.);
                c.move_to(18., 5. + i as f64 * 7.);
                c.line_to(23., 2. + i as f64 * 7.);
                c.line_to(28., 5. + i as f64 * 7.);
                c.stroke().unwrap();
            }
            c.begin_path();
            c.set_line_width(1.);
            c.set_line_dash(&[2., 1., 3.]);
            c.set_line_dash_offset(1.);
            c.move_to(1., 22.5);
            c.line_to(31., 22.5);
            c.stroke().unwrap();
        }
        "clip-transform" => {
            c.translate(16., 12.);
            c.rotate(0.3);
            c.scale(1.2, 0.8);
            c.rect(-10., -10., 20., 20.);
            c.clip(FillRule::NonZero);
            c.save();
            c.begin_path();
            c.arc(Point::new(0., 0.), 9., 0., TAU, false).unwrap();
            c.clip(FillRule::NonZero);
            c.set_fill_color("#2080e0").unwrap();
            c.fill_rect(-30., -30., 60., 60.).unwrap();
            c.restore();
            c.reset_transform();
            c.set_composite("destination-out");
            c.fill_rect(14., 0., 4., 24.).unwrap();
        }
        "linear" | "radial" | "conic" => {
            let kind = match name {
                "linear" => GradientKind::Linear(Point::new(0., 0.), Point::new(32., 0.)),
                "radial" => GradientKind::Radial {
                    start: Point::new(16., 12.),
                    r0: 0.,
                    end: Point::new(16., 12.),
                    r1: 16.,
                },
                _ => GradientKind::Conic {
                    angle: 0.,
                    center: Point::new(16., 12.),
                },
            };
            let g = Gradient::new(kind).unwrap();
            g.add_color_stop(0., "red").unwrap();
            g.add_color_stop(0.5, "lime").unwrap();
            g.add_color_stop(1., "blue").unwrap();
            c.set_fill(Paint::Gradient(g));
            c.fill_rect(0., 0., 32., 24.).unwrap();
        }
        "patterns" => {
            c.set_image_smoothing(false);
            for (i, rep) in [
                Repetition::Repeat,
                Repetition::RepeatX,
                Repetition::RepeatY,
                Repetition::NoRepeat,
            ]
            .into_iter()
            .enumerate()
            {
                let pattern = Pattern::new(source(), rep);
                pattern
                    .set_transform(Affine::translate((2. + i as f64 * 8., 2.)) * Affine::scale(2.))
                    .unwrap();
                c.set_fill(Paint::Pattern(pattern));
                c.fill_rect(i as f64 * 8., 0., 8., 24.).unwrap();
            }
        }
        "images" => {
            c.set_image_smoothing(false);
            c.draw_image(
                source(),
                Rect::new(0., 0., 2., 2.),
                Rect::new(1., 1., 13., 13.),
            )
            .unwrap();
            c.set_image_smoothing(true);
            c.draw_image(
                source(),
                Rect::new(0., 0., 2., 2.),
                Rect::new(17., 1., 29., 13.),
            )
            .unwrap();
            c.put_image_data((*source().pixels).clone(), 3, 18, Some([2, 0, -1, 2]))
                .unwrap();
        }
        "composites" => {
            for (x, mode) in MODES.into_iter().enumerate() {
                c.save();
                c.begin_path();
                c.rect(x as f64, 0., 1., 3.);
                c.clip(FillRule::NonZero);
                c.set_fill_color("rgba(60,120,180,0.6)").unwrap();
                c.fill_rect(x as f64, 0., 1., 2.).unwrap();
                c.set_composite(mode);
                c.set_fill_color("rgba(200,80,40,0.4)").unwrap();
                c.fill_rect(x as f64, 1., 1., 2.).unwrap();
                c.restore();
            }
        }
        _ => panic!("unknown scene {name}"),
    }
    c
}
fn source() -> Source {
    Source {
        key: SourceKey {
            id: 123,
            version: 1,
        },
        pixels: Arc::new(
            ImageData::from_rgba(
                2,
                2,
                vec![
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
            )
            .unwrap(),
        ),
    }
}
pub fn render(name: &str) -> ImageData {
    let mut c = record(name);
    let (w, h) = c.size();
    c.get_image_data(0, 0, w as i32, h as i32).unwrap()
}
