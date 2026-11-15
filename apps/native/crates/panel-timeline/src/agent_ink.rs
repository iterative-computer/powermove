//! Timeline-only agent ink. This never mutates the scene, hit tests or document.
//! The host supplies time and requests frames only while `moving` is true.
use crate::{
    RowId,
    render::{Frame, Op, Rect, Scene},
    v4,
};
use pm_design::Tokens;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Focus,
    Create,
    Key,
    Set,
    Remove,
    Held,
    Dropped,
    End,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub layer: String,
    pub property: Option<String>,
    pub kind: Kind,
    pub times: Vec<f64>,
}
#[derive(Clone, Debug)]
struct Active {
    event: Event,
    started: u64,
}
#[derive(Default)]
pub struct AgentInk {
    events: VecDeque<Active>,
}
impl AgentInk {
    pub fn receive(&mut self, event: Event, now: u64) {
        if event.kind != Kind::Held {
            self.events
                .retain(|a| a.event.layer != event.layer || a.event.kind != Kind::Held);
        }
        if event.kind == Kind::End {
            self.events.clear();
            return;
        }
        if let Some(old) = self
            .events
            .iter_mut()
            .find(|a| a.event.layer == event.layer && a.event.kind == event.kind)
        {
            *old = Active {
                event,
                started: now,
            };
            return;
        }
        if self.events.len() == 6 {
            self.events.pop_front();
        }
        self.events.push_back(Active {
            event,
            started: now,
        });
    }
    pub fn moving(&mut self, now: u64) -> bool {
        self.events
            .retain(|a| a.event.kind == Kind::Held || now.saturating_sub(a.started) < 1600);
        self.events
            .iter()
            .any(|a| a.event.kind != Kind::Held && now.saturating_sub(a.started) < 1600)
    }
    pub fn empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn paint(&self, frame: &mut Frame, scene: &Scene, tokens: &Tokens, now: u64) {
        if self.events.is_empty() {
            return;
        }
        let palette = tokens.sketch();
        let reduced = tokens.accessibility.reduce_motion;
        let v = frame.viewport;
        for (event_index, active) in self.events.iter().enumerate() {
            let age = now.saturating_sub(active.started) as f64;
            if age >= 1600. && active.event.kind != Kind::Held {
                continue;
            }
            let row = v4::row_id(&match &active.event.property {
                Some(property) => RowId::Property {
                    layer_id: active.event.layer.clone(),
                    channel: property.clone(),
                },
                None => RowId::Layer(active.event.layer.clone()),
            });
            let row = if scene.row(&row).is_some() {
                row
            } else {
                v4::row_id(&RowId::Layer(active.event.layer.clone()))
            };
            let Some(top) = scene.scroll_for(&row) else {
                continue;
            };
            let y = tokens.metrics.ruler as f64 + top - v.scroll_y;
            let target = scene.row(&row);
            let strength = if target.is_some_and(|r| r.selected || r.hovered) {
                0.25
            } else {
                1.
            };
            let height = if matches!(v4::parse_row(&row), Ok(RowId::Property { .. })) {
                tokens.metrics.property_row
            } else {
                tokens.metrics.track_row
            } as f64;
            if y + height < tokens.metrics.ruler as f64 || y > v.height {
                let mut tick = Op::new(
                    format!("agent-edge:{}", active.event.layer),
                    "rect",
                    Rect::new(
                        0.,
                        if y < 0. {
                            tokens.metrics.ruler as f64
                        } else {
                            v.height - 4.
                        },
                        4.,
                        4.,
                    ),
                    palette.ink,
                );
                tick.opacity = (1. - ((age - 1200.) / 300.).clamp(0., 1.)).max(0.);
                frame.ops.push(tick);
                continue;
            }
            let fade = if reduced { 120. } else { 160. };
            let wash =
                strength * (age / fade).clamp(0., 1.) * (1. - ((age - 1200.) / 400.).clamp(0., 1.));
            // Twelve adjacent fills approximate the header gradient without a layer/buffer.
            for i in 0..12 {
                let width = v.layer_column * 0.6 / 12.;
                let mut op = Op::new(
                    format!("agent-wash:{}:{i}", active.event.layer),
                    "rect",
                    Rect::new(i as f64 * width, y, width + 0.5, height),
                    palette.wash,
                );
                op.opacity = wash * (1. - i as f64 / 12.);
                frame.ops.push(op);
            }
            if !reduced && event_index + 1 == self.events.len() {
                let mut core = Op::new(
                    format!("agent-core:{}", active.event.layer),
                    "circle",
                    Rect::new(2., y + height / 2. - 1.5, 3., 3.),
                    palette.core,
                );
                core.opacity = wash;
                frame.ops.push(core);
            }
            if active.event.kind == Kind::Held {
                let mut dot = Op::new(
                    format!("agent-held:{}", active.event.layer),
                    "circle",
                    Rect::new(v.layer_column - 8., y + height / 2. - 1.45, 2.9, 2.9),
                    palette.ink,
                );
                dot.opacity = 0.6;
                frame.ops.push(dot);
            }
            let mut animated = 0;
            for op in &mut frame.ops {
                if active.event.kind == Kind::Create
                    && op.id == format!("clip-content:{}", active.event.layer)
                    && op.text.is_some()
                {
                    op.opacity *= ((age - 80.) / 200.).clamp(0., 1.);
                }
                if active.event.kind == Kind::Create
                    && op.id == format!("clip:{}", active.event.layer)
                {
                    let dry = if reduced {
                        (age / 160.).clamp(0., 1.)
                    } else {
                        ((age - 680.) / 300.).clamp(0., 1.)
                    } as f32;
                    op.color = pm_design::color::mix(palette.ink, op.color, dry);
                    // Native clips use their gradient/rim detail, rather than
                    // the flat op colour. Dry that same paint, too.
                    for key in ["top", "bottom", "ring", "light", "grid"] {
                        if let Some(rgb) = op.detail[key].as_u64() {
                            op.detail[key] = serde_json::json!(pm_design::color::mix(
                                palette.ink,
                                rgb as u32,
                                dry,
                            ));
                        }
                    }
                    if !reduced {
                        if age < 400. {
                            op.opacity = 0.;
                        } else {
                            let grow = ((age - 400.) / 280.).clamp(0., 1.);
                            op.bounds.width *= 1. - (1. - grow).powi(3);
                        }
                    }
                }
                if active.event.kind == Kind::Key && op.id.starts_with("key:") && animated < 8 {
                    let Ok((key_row, _)) = serde_json::from_str::<(String, String)>(&op.id[4..])
                    else {
                        continue;
                    };
                    let Ok(key_row) = v4::parse_row(&key_row) else {
                        continue;
                    };
                    let layer = match key_row {
                        RowId::Layer(id) | RowId::Property { layer_id: id, .. } | RowId::Effect { layer_id: id, .. } => id,
                    };
                    if layer != active.event.layer {
                        continue;
                    }
                    let time = v.scroll_time
                        + (op.bounds.x + op.bounds.width / 2. - v.layer_column) / v.pps;
                    if !active.event.times.is_empty()
                        && !active.event.times.iter().any(|t| (*t - time).abs() < 0.01)
                    {
                        continue;
                    }
                    let age = (age - animated as f64 * 60.).max(0.);
                    animated += 1;
                    let dry = if reduced {
                        (age / 160.).clamp(0., 1.)
                    } else {
                        ((age - 280.) / 300.).clamp(0., 1.)
                    };
                    op.color = pm_design::color::mix(palette.ink, op.color, dry as f32);
                    if !reduced {
                        let scale = 0.7 + 0.3 * (1. - (1. - (age / 180.).clamp(0., 1.)).powi(3));
                        op.bounds.x += op.bounds.width * (1. - scale) / 2.;
                        op.bounds.y += op.bounds.height * (1. - scale) / 2.;
                        op.bounds.width *= scale;
                        op.bounds.height *= scale;
                    }
                }
            }
            if active.event.kind == Kind::Create && age < 400. && !reduced {
                if let Some(bar) = frame
                    .ops
                    .iter()
                    .find(|o| o.id == format!("clip:{}", active.event.layer))
                    .map(|o| o.bounds)
                {
                    let duration = (bar.width / 0.6).clamp(1., 400.);
                    let end = bar.x + bar.width * (age / duration).clamp(0., 1.);
                    let mut x = bar.x.max(v.layer_column);
                    let end = end.min(v.width);
                    let mut dots = 0;
                    while x < end && dots < 600 {
                        for y in [bar.y, bar.y + bar.height] {
                            let mut dot = Op::new(
                                format!("agent-pencil:{}:{x}:{y}", active.event.layer),
                                "circle",
                                Rect::new(x, y, 1.8, 1.8),
                                palette.pencil,
                            );
                            dot.opacity = 0.38;
                            frame.ops.push(dot);
                            dots += 1;
                        }
                        x += palette.dot_pitch;
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> Event {
        Event {
            layer: "L".into(),
            property: None,
            kind: Kind::Focus,
            times: vec![],
        }
    }
    #[test]
    fn no_run_allocates_or_schedules_nothing() {
        let mut ink = AgentInk::default();
        assert!(ink.empty());
        assert!(!ink.moving(0));
        assert_eq!(ink.events.capacity(), 0);
    }
    #[test]
    fn idle_has_zero_requested_frames_after_fifteen_seconds() {
        let mut ink = AgentInk::default();
        ink.receive(event(), 0);
        let mut frames = 0;
        for ms in (15_000..25_000).step_by(50) {
            if ink.moving(ms) {
                frames += 1;
            }
        }
        assert_eq!(frames, 0);
    }
    #[test]
    fn queue_is_bounded_and_merges_same_targets() {
        let mut ink = AgentInk::default();
        for i in 0..20 {
            let mut e = event();
            e.layer = format!("L{i}");
            ink.receive(e, i);
        }
        assert_eq!(ink.events.len(), 6);
        ink.receive(event(), 21);
        ink.receive(event(), 22);
        assert_eq!(ink.events.len(), 6);
    }
    #[test]
    fn held_dot_is_static() {
        let mut ink = AgentInk::default();
        let mut e = event();
        e.kind = Kind::Held;
        ink.receive(e, 0);
        assert!(!ink.moving(15_000));
        assert!(!ink.empty());
    }
}
