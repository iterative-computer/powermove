use gpui::{
    ColorExt, Div, Font, FontWeight, TextAlign, TextRun, Window, canvas, div, point, prelude::*,
    px, rgb,
};
use pm_design::{
    Text, Tokens,
    motion::{Animated, Clock, Spring},
};
use std::time::Instant;

const DURATION: f64 = 0.55;
const TRAVEL_EM: f32 = 0.35;
const STAGGER: f32 = 0.30;

#[derive(Clone)]
struct Glyph {
    index: usize,
    character: char,
    delay: f32,
    position: Animated<3>,
}
impl Glyph {
    fn frame(&mut self, dt: f32) {
        let waiting = dt.min(self.delay);
        self.delay -= waiting;
        self.position.step(dt - waiting);
    }
}

/// A vertically rolling title or count. Unchanged glyphs retain their place
/// and interrupted glyphs keep their current position and velocity.
#[derive(Default)]
pub struct RollingLabel {
    value: String,
    glyphs: Vec<Glyph>,
    clock: Clock,
}

fn place(
    value: &str,
    font: &Font,
    font_size: f32,
    line: f32,
    tracking: f32,
    window: &Window,
) -> Vec<Glyph> {
    let run = TextRun {
        len: value.len(),
        font: font.clone(),
        letter_spacing: Some(px(tracking)),
        ..TextRun::default()
    };
    let Ok(layout) =
        window
            .text_system()
            .shape_text(value.to_owned(), px(font_size), &[run], None, None)
    else {
        return Vec::new();
    };
    value
        .char_indices()
        .enumerate()
        .filter_map(|(index, (byte, character))| {
            let x = layout.visual_position_for_byte_index(byte, px(line))?.x;
            Some(Glyph {
                index,
                character,
                delay: 0.,
                position: Animated::new([f32::from(x), 0., 1.], Spring::SNAP, 0.01),
            })
        })
        .collect()
}

impl RollingLabel {
    pub fn render(
        &mut self,
        value: &str,
        tokens: &Tokens,
        role: Text,
        color: u32,
        window: &mut Window,
    ) -> Div {
        if self.glyphs.is_empty() && self.value == value {
            return div().child(self.value.clone());
        }
        let dt = self.clock.tick(Instant::now());
        self.glyphs.retain_mut(|glyph| {
            glyph.frame(dt);
            glyph.position.moving() || glyph.position.target[2] == 1.
        });

        let spec = tokens.text(role);
        let line_height = (spec.size * spec.leading).round();
        let mut font = gpui::font(pm_design::TEXT_FONT);
        font.weight = FontWeight(spec.weight);
        if spec.tabular {
            font.features = crate::kit::tabular();
        }
        if self.value.is_empty() || tokens.accessibility.reduce_motion {
            self.glyphs.clear();
            self.value = value.to_owned();
        } else if self.value != value {
            let shape = |text| place(text, &font, spec.size, line_height, spec.tracking, window);
            if self.glyphs.is_empty() {
                self.glyphs = shape(&self.value);
            }
            let incoming = shape(value);
            let step =
                DURATION as f32 * STAGGER / incoming.len().max(self.glyphs.len()).max(1) as f32;
            let spring = Spring::perceptual(DURATION, 0.18);
            let distance = spec.size * TRAVEL_EM;
            let mut outgoing = std::mem::take(&mut self.glyphs);
            for mut glyph in incoming {
                let target = glyph.position.value;
                if let Some(i) = outgoing.iter().position(|old| {
                    old.position.target[2] == 1.
                        && old.index == glyph.index
                        && old.character == glyph.character
                }) {
                    glyph = outgoing.remove(i);
                } else {
                    glyph.position.snap([target[0], distance, 0.]);
                    glyph.delay = glyph.index as f32 * step;
                }
                glyph.position.animate_to(target, spring);
                self.glyphs.push(glyph);
            }
            self.glyphs
                .extend(outgoing.into_iter().filter_map(|mut glyph| {
                    if glyph.position.target[2] == 0. {
                        return Some(glyph);
                    }
                    if glyph.position.value[2] <= 0. {
                        return None;
                    }
                    glyph.delay = if glyph.position.moving() {
                        0.
                    } else {
                        glyph.index as f32 * step
                    };
                    let x = glyph.position.value[0];
                    glyph.position.animate_to([x, -distance, 0.], spring);
                    Some(glyph)
                }));
            self.value = value.to_owned();
        }

        if !self.glyphs.iter().any(|glyph| glyph.position.moving()) {
            self.glyphs.clear();
            self.clock.rest();
            return div().child(self.value.clone());
        }

        window.request_animation_frame();
        let glyphs = self.glyphs.clone();
        div()
            .relative()
            .overflow_hidden()
            .h(px(line_height))
            .child(
                div()
                    .id("roll-size")
                    .aria_hidden()
                    .text_color(rgb(color).opacity(0.))
                    .whitespace_nowrap()
                    .child(self.value.clone()),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        for glyph in glyphs {
                            let [x, y, alpha] = glyph.position.value;
                            if alpha <= 0. || glyph.character.is_whitespace() {
                                continue;
                            }
                            let character = glyph.character.to_string();
                            let run = TextRun {
                                len: character.len(),
                                font: font.clone(),
                                color: gpui::rgb_to_hsla(rgb(color).opacity(alpha)),
                                ..TextRun::default()
                            };
                            let line = window.text_system().shape_line(
                                character.into(),
                                px(spec.size),
                                &[run],
                            );
                            let origin = bounds.origin + point(px(x), px(y));
                            let _ = line.paint(
                                origin,
                                px(line_height),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}
