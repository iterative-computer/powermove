//! The grading controls a panel shows (kit `color-wheel` and `curve-editor`):
//! drawn by `pm_shell::grade`, driven by the pointer through the panel's
//! grouped binding, one kernel gesture per drag (Escape cancels it, as a
//! number field's scrub).
use super::{ScrubDrag, ScrubOp, SurfaceView};
use crate::v4_compile::Kit;
use gpui::{
    Bounds, Context, IntoElement, MouseDownEvent, MouseMoveEvent, Pixels, Point, Styled, Window,
    canvas, point, px,
};
use pm_shell::ease;
use pm_shell::grade::{self, Curve, Graph, Look};
use pm_surface::Id;
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Where each grading control was last drawn, for the next press.
pub(super) type Drawn = Rc<RefCell<HashMap<Id, Bounds<Pixels>>>>;

/// A drag on a grading control.
pub(super) enum GradeDrag {
    Wheel {
        /// The puck when the press began, and how far it travels per point.
        from: (f64, f64),
        reach: f32,
        /// The channels when the press began: their brightness is kept.
        keep: [f64; 3],
        neutral: f64,
        range: [f64; 2],
        /// What the wheel shows now (ahead of the kernel's answer).
        live: [f64; 3],
    },
    Curve {
        graph: Graph,
        /// The parameters when the press began (slots past the points keep
        /// what they held).
        rest: Vec<f64>,
        original: Curve,
        point: usize,
        /// The point's value where the press began.
        from: (f64, f64),
        /// Where the pointer pressed.
        at: Point<Pixels>,
        /// The curve as it stands now, and whether the point is off the
        /// graph (it goes on release).
        live: Curve,
        leaving: bool,
    },
    /// An easing curve's handle (0 outgoing, 1 incoming) under the pointer.
    Ease {
        graph: ease::Graph,
        knob: usize,
        /// The handle where the press began, and where the pointer pressed.
        from: (f64, f64),
        at: Point<Pixels>,
        /// The curve as the hand has it now.
        live: [f64; 4],
    },
}

/// Easing curves: when each last changed (its preview dot runs once from
/// then), and the curve a drag left until the panel publishes it.
#[derive(Clone, Default)]
pub(super) struct EaseState {
    seen: Rc<RefCell<HashMap<Id, ([f64; 4], std::time::Instant)>>>,
    pending: Rc<RefCell<HashMap<Id, ([f64; 4], std::time::Instant)>>>,
}
/// How long the preview dot takes to run the curve.
const PREVIEW: f32 = 1.2;

/// The plot inside an easing editor's bounds: inset so the knobs at the
/// edges show whole, over the span `range` shows.
fn ease_graph(bounds: Bounds<Pixels>, range: (f64, f64), t: &pm_design::Tokens) -> ease::Graph {
    let inset = t.metrics.keyframe / 2. + t.metrics.border;
    ease::Graph::new(inside(bounds, inset), range)
}

/// The graph inside a curve editor's bounds: inset so points on its edges
/// show whole.
pub(super) fn graph(bounds: Bounds<Pixels>, t: &pm_design::Tokens) -> Graph {
    Graph::new(inside(bounds, grade::curve_inset(t)))
}
fn inside(bounds: Bounds<Pixels>, inset: f32) -> Bounds<Pixels> {
    let inset = px(inset);
    Bounds::new(
        bounds.origin + point(inset, inset),
        gpui::size(
            (bounds.size.width - inset * 2.).max(px(1.)),
            (bounds.size.height - inset * 2.).max(px(1.)),
        ),
    )
}

/// A grading control's height for the width it has: a wheel is square
/// between `metrics.grade_wheel` and `grade_wheel_max` (centred past the
/// greatest), a curve editor square between `curve_editor` and
/// `curve_editor_max`.
fn side(kit: &Kit, width: f32, t: &pm_design::Tokens) -> f32 {
    let m = &t.metrics;
    match kit {
        Kit::Curve { .. } => width.clamp(m.curve_editor, m.curve_editor_max),
        // A little taller than wide: room above and below the square for
        // handles that overshoot.
        Kit::Ease { .. } => (width * 1.15).clamp(m.curve_editor, m.curve_editor_max),
        _ => width.clamp(m.grade_wheel, m.grade_wheel_max),
    }
}

impl SurfaceView {
    /// The height a drawn grading control takes from the width it was last
    /// laid out at (its least before it has been).
    pub(super) fn grade_height(&self, id: Id, kit: &Kit) -> Option<f32> {
        if !matches!(
            kit,
            Kit::Wheel { .. }
                | Kit::Curve { .. }
                | Kit::Ease {
                    thumbnail: false,
                    ..
                }
        ) {
            return None;
        }
        let width = self
            .grade_drawn
            .borrow()
            .get(&id)
            .map_or(0., |b| f32::from(b.size.width));
        Some(side(kit, width, &self.table.tokens))
    }

    /// The drawing of a grading control, showing a drag ahead of the kernel.
    pub(super) fn grade_canvas(
        &self,
        id: Id,
        kit: &Kit,
        disabled: bool,
    ) -> Option<impl IntoElement> {
        let tokens = self.table.tokens.clone();
        let tokens2 = tokens.clone();
        let drawn = self.grade_drawn.clone();
        let drag = self
            .scrub
            .as_ref()
            .filter(|d| d.id == id)
            .and_then(|d| d.grade.as_ref());
        let look = Look {
            disabled,
            active: drag.is_some(),
        };
        let paint: Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut gpui::App)> = match (kit, drag) {
            (
                Kit::Wheel {
                    rgb,
                    neutral,
                    master,
                    ..
                },
                drag,
            ) => {
                let (rgb, dragged) = match drag {
                    Some(GradeDrag::Wheel { live, .. }) => (*live, true),
                    _ => (*rgb, false),
                };
                let target = grade::puck(rgb, *neutral);
                let master = *master;
                let motion = self.grade_motion.clone();
                Box::new(move |b, window, cx| {
                    // Under the pointer the puck is where the hand puts it;
                    // otherwise (a reset, an undo, another wheel's edit) it
                    // settles there on the layout spring, interruptibly.
                    let puck = settle(&motion, id, target, dragged, &tokens, window);
                    grade::paint_wheel(b, puck, master, look, &tokens, window, cx);
                })
            }
            (Kit::Curve { values, channel }, drag) => {
                let (curve, held, leaving) = match drag {
                    Some(GradeDrag::Curve {
                        live,
                        point,
                        leaving,
                        ..
                    }) => (live.clone(), Some(*point), *leaving),
                    _ => (Curve::from_values(values), None, false),
                };
                let channel = *channel;
                Box::new(move |b, window, _| {
                    let g = graph(b, &tokens);
                    grade::paint_curve(g, &curve, channel, held, leaving, look, &tokens, window);
                })
            }
            (
                Kit::Ease {
                    curve,
                    thumbnail: true,
                },
                _,
            ) => {
                let curve = *curve;
                Box::new(move |b, window, _| ease::paint_thumbnail(b, &curve, &tokens, window))
            }
            (Kit::Ease { curve, .. }, drag) => {
                let published = *curve;
                let (curve, held, range) = match drag {
                    Some(GradeDrag::Ease {
                        live, knob, graph, ..
                    }) => (*live, Some(*knob), Some((graph.min, graph.max))),
                    _ => {
                        // What a drag left, until the panel publishes it (briefly:
                        // the panel may have moved on since).
                        let mut pending = self.ease.pending.borrow_mut();
                        let left = pending
                            .get(&id)
                            .filter(|(left, when)| {
                                *left != published && when.elapsed().as_secs_f32() < 0.5
                            })
                            .map(|(left, _)| *left);
                        if left.is_none() {
                            pending.remove(&id);
                        }
                        let shown = left.unwrap_or(published);
                        (shown, None, None)
                    }
                };
                let seen = self.ease.seen.clone();
                let dragging = held.is_some();
                Box::new(move |b, window, _| {
                    // The preview dot runs the curve once after it changes,
                    // and over and over while a handle is held.
                    let now = std::time::Instant::now();
                    let since = {
                        let mut seen = seen.borrow_mut();
                        let entry = seen.entry(id).or_insert((curve, now));
                        if entry.0 != curve {
                            *entry = (curve, now);
                        }
                        now.duration_since(entry.1).as_secs_f32()
                    };
                    let dot = if tokens.accessibility.reduce_motion {
                        None
                    } else if dragging {
                        Some(((since % (PREVIEW + 0.4)) / PREVIEW).min(1.) as f64)
                    } else if since < PREVIEW + 0.4 {
                        Some((since / PREVIEW).min(1.) as f64)
                    } else {
                        None
                    };
                    if dot.is_some() {
                        window.request_animation_frame();
                    }
                    let g = ease_graph(b, range.unwrap_or_else(|| ease::range(&curve)), &tokens);
                    ease::paint_editor(
                        g,
                        &curve,
                        ease::Look {
                            disabled: look.disabled,
                            held,
                            dot,
                        },
                        &tokens,
                        window,
                    );
                })
            }
            _ => return None,
        };
        let kit = kit.clone();
        Some(
            canvas(
                move |bounds, window, _| {
                    // The height follows the width it was given: when that
                    // width asks for another height, lay out again.
                    // Once per change of width, so a height held elsewhere
                    // never loops.
                    let was = drawn.borrow_mut().insert(id, bounds);
                    let width = |b: Bounds<Pixels>| f32::from(b.size.width);
                    let wants = side(&kit, width(bounds), &tokens2);
                    if was.is_none_or(|w| (width(w) - width(bounds)).abs() > 0.5)
                        && (wants - f32::from(bounds.size.height)).abs() > 0.5
                    {
                        window.refresh();
                    }
                },
                move |bounds, _, window, cx| paint(bounds, window, cx),
            )
            .absolute()
            .inset_0(),
        )
    }

    /// A press on a grading control: a wheel's puck moves from where it is;
    /// a curve's point under the pointer moves, or a press on the graph adds
    /// one there. A double-click returns the control to neutral.
    pub(super) fn grade_press(&mut self, id: Id, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(node) = self.prepared.node(id) else {
            return;
        };
        // An easing curve's handle: the panel owns the curve, so each move
        // goes to it as a change (no document gesture).
        if let Kit::Ease {
            curve,
            thumbnail: false,
        } = node.kit
        {
            if node.disabled {
                return;
            }
            let Some(bounds) = self.grade_drawn.borrow().get(&id).copied() else {
                return;
            };
            let t = &self.table.tokens;
            let shown = self
                .ease
                .pending
                .borrow()
                .get(&id)
                .filter(|(_, when)| when.elapsed().as_secs_f32() < 0.5)
                .map_or(curve, |(left, _)| *left);
            let g = ease_graph(bounds, ease::range(&shown), t);
            let Some(knob) = ease::handle_at(&g, &shown, event.position, t.metrics.hit / 2.) else {
                return;
            };
            self.scrub = Some(ScrubDrag {
                id,
                start: event.position,
                number: Default::default(),
                binding: None,
                active: false,
                grade: Some(GradeDrag::Ease {
                    graph: g,
                    knob,
                    from: (shown[knob * 2], shown[knob * 2 + 1]),
                    at: event.position,
                    live: shown,
                }),
            });
            cx.notify();
            return;
        }
        let Some(binding) = node.binding.clone().filter(|_| !node.local_change) else {
            return;
        };
        if node.disabled {
            return;
        }
        let Some(bounds) = self.grade_drawn.borrow().get(&id).copied() else {
            return;
        };
        let t = self.table.tokens.clone();
        let kit = node.kit.clone();
        if event.click_count == 2 {
            let neutral = match &kit {
                Kit::Wheel { neutral, .. } => json!([neutral, neutral, neutral]),
                Kit::Curve { values, .. } => json!(
                    Curve {
                        points: vec![(0., 0.), (1., 1.)]
                    }
                    .values(values)
                ),
                _ => return,
            };
            self.scrub = None;
            self.scrub_ops.push_back(ScrubOp::Begin);
            self.scrub_ops.push_back(ScrubOp::Value(binding, neutral));
            self.scrub_ops.push_back(ScrubOp::Finish(true));
            self.scrub_pump(cx);
            return;
        }
        let drag = match kit {
            Kit::Wheel {
                rgb,
                neutral,
                range,
                ..
            } => GradeDrag::Wheel {
                from: grade::puck(rgb, neutral),
                reach: grade::wheel_reach(grade::wheel_diameter(bounds), &t),
                keep: rgb,
                neutral,
                range,
                live: rgb,
            },
            Kit::Curve { values, .. } => {
                let g = graph(bounds, &t);
                let original = Curve::from_values(&values);
                let hit = t.metrics.hit / 2.;
                let nearest = original
                    .points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let at = g.to_point((p.0.clamp(0., 1.), p.1.clamp(0., 1.)));
                        (
                            i,
                            f32::from((at - event.position).magnitude() as f32 * px(1.)),
                        )
                    })
                    .filter(|(_, d)| *d <= hit)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i);
                let (mut live, point, added) = match nearest {
                    Some(i) => (original.clone(), i, false),
                    None if original.points.len() < grade::CURVE_POINTS
                        && g.area.contains(&event.position) =>
                    {
                        let (x, y) = g.to_value(event.position);
                        let mut c = original.clone();
                        c.points.push((x.clamp(0., 1.), y.clamp(0., 1.)));
                        let i = c.points.len() - 1;
                        // Never on another point's input (the effect would
                        // keep only the one written last).
                        c.points[i].0 = place(&c, i, x);
                        (c, i, true)
                    }
                    None => return,
                };
                let from = live.points[point];
                if added {
                    live.points[point] = (precise(from.0), precise(from.1));
                }
                let drag = GradeDrag::Curve {
                    graph: g,
                    rest: values.clone(),
                    original: if added { live.clone() } else { original },
                    point,
                    from: live.points[point],
                    at: event.position,
                    live,
                    leaving: false,
                };
                if added {
                    // The new point is the press's own edit: it is made now.
                    let GradeDrag::Curve { live, rest, .. } = &drag else {
                        unreachable!()
                    };
                    self.scrub_ops.push_back(ScrubOp::Begin);
                    self.scrub_ops
                        .push_back(ScrubOp::Value(binding.clone(), json!(live.values(rest))));
                    self.scrub = Some(ScrubDrag {
                        id,
                        start: event.position,
                        number: Default::default(),
                        binding: Some(binding),
                        active: true,
                        grade: Some(drag),
                    });
                    self.scrub_pump(cx);
                    cx.notify();
                    return;
                }
                drag
            }
            _ => return,
        };
        self.scrub = Some(ScrubDrag {
            id,
            start: event.position,
            number: Default::default(),
            binding: Some(binding),
            active: false,
            grade: Some(drag),
        });
    }

    /// The pointer moved during a press on a grading control.
    pub(super) fn grade_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // An easing handle follows the pointer from where it was grabbed
        // (finely with Option, on a twentieth grid with Shift); each move is
        // the panel's change.
        if let Some(ScrubDrag {
            id,
            grade:
                Some(GradeDrag::Ease {
                    graph,
                    knob,
                    from,
                    at,
                    live,
                }),
            ..
        }) = self.scrub.as_mut()
        {
            let here = graph.to_value(event.position);
            let pressed = graph.to_value(*at);
            let fine = if event.modifiers.alt { 0.1 } else { 1. };
            let (x, y) = ease::place(
                (
                    from.0 + (here.0 - pressed.0) * fine,
                    from.1 + (here.1 - pressed.1) * fine,
                ),
                (graph.min, graph.max),
                event.modifiers.shift,
            );
            if (live[*knob * 2], live[*knob * 2 + 1]) == (x, y) {
                return;
            }
            live[*knob * 2] = x;
            live[*knob * 2 + 1] = y;
            let (id, live) = (*id, *live);
            self.ease
                .pending
                .borrow_mut()
                .insert(id, (live, std::time::Instant::now()));
            self.change(id, json!(live), cx);
            cx.notify();
            return;
        }
        let threshold = self.table.tokens.metrics.drag_threshold;
        let off = self.table.tokens.metrics.hit;
        let Some(drag) = &mut self.scrub else {
            return;
        };
        let delta = event.position - drag.start;
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if !drag.active {
            if dx.hypot(dy) < threshold {
                return;
            }
            drag.active = true;
            self.scrub_ops.push_back(ScrubOp::Begin);
        }
        let fine = if event.modifiers.alt { 0.1 } else { 1. };
        let Some(binding) = drag.binding.clone() else {
            return;
        };
        let value = match drag.grade.as_mut() {
            Some(GradeDrag::Wheel {
                from,
                reach,
                keep,
                neutral,
                range,
                live,
            }) => {
                let puck = grade::unit((
                    from.0 + (dx / *reach) as f64 * fine,
                    from.1 - (dy / *reach) as f64 * fine,
                ));
                *live = grade::channels(puck, *keep, *neutral, *range);
                json!(live)
            }
            Some(GradeDrag::Curve {
                graph,
                rest,
                original,
                point,
                from,
                at,
                live,
                leaving,
            }) => {
                let a = graph.area;
                let outside = event.position.x < a.left() - px(off)
                    || event.position.x > a.right() + px(off)
                    || event.position.y < a.top() - px(off)
                    || event.position.y > a.bottom() + px(off);
                *leaving = outside && original.points.len() > 2;
                let mut next = original.clone();
                if *leaving {
                    next.points.remove(*point);
                } else {
                    let here = graph.to_value(event.position);
                    let pressed = graph.to_value(*at);
                    let x = from.0 + (here.0 - pressed.0) * fine;
                    let y = from.1 + (here.1 - pressed.1) * fine;
                    next.points[*point] = (place(original, *point, x), precise(y.clamp(0., 1.)));
                }
                *live = next;
                json!(live.values(rest))
            }
            Some(GradeDrag::Ease { .. }) | None => return,
        };
        self.scrub_ops.push_back(ScrubOp::Value(binding, value));
        self.scrub_pump(cx);
        cx.notify();
    }
}

fn precise(v: f64) -> f64 {
    (v * 1000.).round() / 1000.
}

/// Each wheel's drawn puck, moving toward where its values put it.
pub(super) type Motion =
    Rc<RefCell<HashMap<Id, (pm_design::motion::Animated<2>, std::time::Instant)>>>;

/// Where wheel `id`'s puck is drawn this frame: at `target` while dragged (or
/// under Reduce Motion); else on the layout spring from where it was drawn.
fn settle(
    motion: &Motion,
    id: Id,
    target: (f64, f64),
    dragged: bool,
    t: &pm_design::Tokens,
    window: &mut Window,
) -> (f64, f64) {
    use pm_design::motion::{Animated, POINT_PRECISION, Preset};
    let goal = [target.0 as f32, target.1 as f32];
    let now = std::time::Instant::now();
    let mut all = motion.borrow_mut();
    let (spring, last) = all.entry(id).or_insert_with(|| {
        (
            Animated::new(
                goal,
                Preset::Layout.spring(&t.motion),
                POINT_PRECISION / 100.,
            ),
            now,
        )
    });
    if dragged || t.accessibility.reduce_motion {
        spring.snap(goal);
    } else if spring.target != goal {
        spring.animate_to(goal, Preset::Layout.spring(&t.motion));
    }
    let dt = now.duration_since(*last).as_secs_f32();
    *last = now;
    if spring.step(dt) {
        window.request_animation_frame();
    }
    (spring.value[0] as f64, spring.value[1] as f64)
}

/// The least distance between two points' inputs the editor leaves: the
/// effect reads inputs within 1e-6 as one point, and values keep three places.
const ROOM: f64 = 0.002;

/// Where point `i` of `curve` may stand on the input axis, nearest `x`:
/// between its neighbours in input order (ties by index, as the effect
/// orders them), at least `ROOM` from each, within 0 to 1. With no room
/// between them it keeps its input.
fn place(curve: &Curve, i: usize, x: f64) -> f64 {
    let mut order: Vec<usize> = (0..curve.points.len()).collect();
    order.sort_by(|a, b| {
        curve.points[*a]
            .0
            .total_cmp(&curve.points[*b].0)
            .then(a.cmp(b))
    });
    let at = order.iter().position(|k| *k == i).unwrap_or(0);
    let lo = at
        .checked_sub(1)
        .map_or(0., |k| curve.points[order[k]].0 + ROOM);
    let hi = order.get(at + 1).map_or(1., |k| curve.points[*k].0 - ROOM);
    if hi < lo {
        return curve.points[i].0;
    }
    precise(x.clamp(lo.max(0.), hi.min(1.)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No drag leaves two points on one input: a point stops short of its
    /// neighbours, the ends stay within 0 to 1, and an added point never
    /// lands on an existing input.
    #[::core::prelude::v1::test]
    fn a_point_keeps_its_own_input() {
        let curve = Curve {
            points: vec![(0., 0.), (0.5, 0.6), (1., 1.)],
        };
        assert_eq!(place(&curve, 1, 0.9999), 1. - ROOM);
        assert_eq!(place(&curve, 1, -3.), ROOM);
        assert_eq!(place(&curve, 0, 0.7), 0.5 - ROOM);
        assert_eq!(place(&curve, 0, -1.), 0.);
        assert_eq!(place(&curve, 2, 2.), 1.);
        assert_eq!(place(&curve, 2, 0.1), 0.5 + ROOM);
        // A point added on an existing input moves off it.
        let mut added = curve.clone();
        added.points.push((0.5, 0.2));
        let x = place(&added, 3, 0.5);
        assert!((x - 0.5).abs() >= ROOM - 1e-12, "{x}");
        // Whatever a drag asks, every input stays apart.
        for target in [-1., 0., 0.25, 0.5, 0.75, 1., 2.] {
            for i in 0..3 {
                let mut c = curve.clone();
                c.points[i].0 = place(&curve, i, target);
                let mut xs: Vec<f64> = c.points.iter().map(|p| p.0).collect();
                xs.sort_by(f64::total_cmp);
                assert!(xs.windows(2).all(|w| w[1] - w[0] > 1e-6), "{xs:?}");
            }
        }
    }

    #[::core::prelude::v1::test]
    fn the_graph_is_inset_by_half_a_knob_and_clear_of_the_corners() {
        let t = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
        let b = Bounds::new(point(px(10.), px(20.)), gpui::size(px(200.), px(100.)));
        let g = graph(b, &t);
        let inset = grade::curve_inset(&t);
        assert!(inset > t.metrics.keyframe / 2. + t.metrics.border);
        assert_eq!(
            g.to_point((0., 1.)),
            point(px(10. + inset), px(20. + inset))
        );
        let back = g.to_value(g.to_point((0.25, 0.75)));
        assert!((back.0 - 0.25).abs() < 1e-6 && (back.1 - 0.75).abs() < 1e-6);
    }
}
