//! Ghost loading (owner request, 2026-11-14): the moment a project is chosen
//! the window shows the editor with every panel in its place, each filled
//! with quiet shapes where its content will be, in Home's own loading
//! language (text ink at the hover wash, pulsing slowly; still under Reduce
//! Motion). Each panel's real content replaces its ghost when that panel is
//! ready, with a short fade (`motion.menu_fade`).
use gpui::{prelude::*, *};
use pm_design::Tokens;
use pm_shell::{kit::ink, layout::Rect};
use std::time::Duration;

/// How long one slow pulse of the ghost takes (Home's loading cards').
pub const PULSE: Duration = Duration::from_millis(1400);

/// What kind of content a panel shows, so its ghost has that shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The composition, framed at its aspect.
    Viewer,
    /// Ruler and track rows under the transport.
    Timeline,
    /// Label and value rows (Properties, Effect Controls).
    Fields,
    /// A search field over list rows (Effects, the Store's lists).
    Search,
    /// Rows with a thumbnail (Project).
    Items,
    /// Rows of text (any other panel).
    Rows,
}
pub fn shape(id: &str) -> Shape {
    match id {
        "viewer" => Shape::Viewer,
        "timeline" => Shape::Timeline,
        "inspector" | "layer-effects" | "gradient-controls" | "color" => Shape::Fields,
        "fxbrowser" | "captions" | "takes" => Shape::Search,
        "assets" => Shape::Items,
        _ => Shape::Rows,
    }
}

/// Where one open stands, for one panel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Open {
    /// A project was chosen and is opening (or its panels are mounting).
    pub chosen: bool,
    /// The project has been read and this window shows it.
    pub landed: bool,
    /// Its panels haven't all drawn it yet.
    pub settling: bool,
    /// This panel has drawn it.
    pub ready: bool,
    /// The viewer waits for effects an extension is still loading.
    pub waiting: bool,
    /// The panel's body is an extension's own page (drawn below GPUI).
    pub native: bool,
}
/// Whether a panel shows its ghost: from the moment a project is chosen
/// until that panel is ready (or the whole open has settled).
pub fn shows(open: Open) -> bool {
    open.chosen
        && !open.native
        && (!open.landed || (open.settling && !open.ready) || open.waiting)
}
/// The ghost's box: the panel's rect under its header, exactly the box its
/// real body takes, so nothing moves when the content arrives.
pub fn body_size(bounds: Rect, t: &Tokens) -> (f32, f32) {
    (bounds.w.max(0.), (bounds.h - t.metrics.header).max(0.))
}
/// The body under a panel's header in `bounds` (the panel's rect): the same
/// box the real body takes, so nothing moves when it arrives.
/// `poster`: the picture the opening project's viewer last showed (or its
/// Home poster), decoded, shown in the viewer's frame.
pub fn body(
    id: &str,
    bounds: Rect,
    aspect: Option<f32>,
    poster: Option<std::sync::Arc<RenderImage>>,
    t: &Tokens,
) -> AnyElement {
    let m = &t.metrics;
    let (w, h) = body_size(bounds, t);
    let wash = ink(t.colors.text, t.translucency.hover);
    let bar = move |width: f32, height: f32| {
        div()
            .flex_none()
            .w(px(width.max(0.)))
            .h(px(height))
            .rounded(px(height.min(m.control_radius)))
            .bg(wash)
    };
    let text = m.field * 0.45;
    let inset = m.inset;
    let inner = (w - inset * 2.).max(0.);
    // Rows that fit the body, never one cut by its bottom edge.
    let rows = |row: f32, top: f32| ((h - top) / row.max(1.)).floor().max(0.) as usize;
    let content = match shape(id) {
        Shape::Viewer => {
            // The composition's frame at its aspect (16:9 when unknown),
            // inside the body's inset, centred.
            let aspect = aspect.filter(|a| a.is_finite() && *a > 0.).unwrap_or(16. / 9.);
            let (room_w, room_h) = ((w - inset * 2.).max(0.), (h - inset * 2.).max(0.));
            let (fw, fh) = if room_w / aspect <= room_h {
                (room_w, room_w / aspect)
            } else {
                (room_h * aspect, room_h)
            };
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(fw))
                        .h(px(fh))
                        .rounded(px(m.control_radius))
                        .overflow_hidden()
                        .bg(wash)
                        .children(poster.map(|poster| {
                            img(ImageSource::Render(poster))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                        })),
                )
        }
        Shape::Timeline => {
            let name_column = (w * 0.3).clamp(m.value_column, m.value_column * 2.);
            let lanes = rows(m.track_row, m.ruler);
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(m.ruler))
                        .flex_none()
                        .flex()
                        .items_center()
                        .pl(px(name_column + inset))
                        .child(bar((w - name_column - inset * 2.).max(0.), 2.)),
                )
                .children((0..lanes).map(|i| {
                    // Layers start at different times and run different
                    // lengths, as a timeline's bars do.
                    let start = [0.0, 0.08, 0.0, 0.22, 0.04][i % 5];
                    let span = [0.9, 0.55, 0.7, 0.45, 0.8][i % 5];
                    let track = (w - name_column - inset * 2.).max(0.);
                    div()
                        .h(px(m.track_row))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px(px(inset))
                        .child(div().w(px(name_column)).flex_none().child(bar(name_column * 0.55, text)))
                        .child(div().w(px(track * start)).flex_none())
                        .child(bar(track * span, m.track_row * 0.6))
                }))
        }
        Shape::Fields => div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(inset))
            .pt(px(m.gap))
            .child(div().h(px(m.row)).flex_none().flex().items_center().child(bar(inner * 0.35, text)))
            .children((0..rows(m.row, m.gap + m.row)).map(|i| {
                let label = [0.42, 0.3, 0.36, 0.26][i % 4];
                div()
                    .h(px(m.row))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pl(px(m.inset))
                    .child(bar((inner - m.value_column - m.inset) * label, text))
                    .child(bar(m.value_column, m.field))
            })),
        Shape::Search => div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(inset))
            .pt(px(m.gap))
            .child(div().h(px(m.control)).flex_none().child(bar(inner, m.control)))
            .children((0..rows(m.row, m.gap + m.control)).map(|i| {
                let label = [0.5, 0.38, 0.62, 0.44, 0.3][i % 5];
                div()
                    .h(px(m.row))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(bar(inner * label, text))
            })),
        Shape::Items => div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(inset))
            .pt(px(m.gap))
            .children((0..rows(m.row, m.gap)).map(|i| {
                let label = [0.55, 0.4, 0.48, 0.34][i % 4];
                div()
                    .h(px(m.row))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(m.gap))
                    .child(bar(m.kind_badge, m.kind_badge))
                    .child(bar((inner - m.kind_badge - m.gap) * label, text))
            })),
        Shape::Rows => div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(inset))
            .pt(px(m.gap))
            .children((0..rows(m.row, m.gap)).map(|i| {
                let label = [0.6, 0.45, 0.52, 0.38][i % 4];
                div()
                    .h(px(m.row))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(bar(inner * label, text))
            })),
    };
    let ghost = div()
        .id(SharedString::from(format!("ghost-{id}")))
        .role(Role::Status)
        .aria_label("Opening")
        .w(px(w))
        .h(px(h))
        .flex_none()
        .overflow_hidden()
        .child(content);
    if t.accessibility.reduce_motion {
        // Still shapes, no pulse.
        return ghost.opacity(0.75).into_any_element();
    }
    ghost
        .with_animation(
            SharedString::from(format!("ghost-pulse-{id}")),
            Animation::new(PULSE)
                .repeat()
                .with_easing(pulsating_between(0.45, 1.)),
            |this, delta| this.opacity(delta),
        )
        .into_any_element()
}

/// Whether Home tells the window about its open now: when one begins, ends
/// or gives way to another (a ghost then shows, goes, or changes name).
pub fn announce(before: &Option<String>, now: &Option<String>) -> bool {
    before != now
}
/// How far into its fade a panel's real content is, `elapsed` after it
/// replaced its ghost: 0 at the swap, 1 once `motion.menu_fade` has passed.
pub fn reveal(elapsed: Duration, t: &Tokens) -> f32 {
    let fade = t.motion.menu_fade;
    if t.accessibility.reduce_motion || fade <= 0. {
        return 1.;
    }
    (elapsed.as_secs_f64() / fade).clamp(0., 1.) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_design::{Accessibility, Scheme};

    /// Shown the moment a project is chosen, per panel until that panel is
    /// ready, gone when the open settles or fails (no project landed and
    /// nothing chosen any more).
    #[::core::prelude::v1::test]
    fn a_ghost_shows_from_the_choice_until_its_panel_is_ready() {
        let chosen = Open {
            chosen: true,
            ..Default::default()
        };
        assert!(shows(chosen), "at once, before the project is read");
        let landed = Open {
            landed: true,
            settling: true,
            ..chosen
        };
        assert!(shows(landed), "read, but this panel hasn't drawn it");
        assert!(!shows(Open { ready: true, ..landed }), "this panel is ready");
        assert!(!shows(Open { settling: false, ..landed }), "the open settled");
        assert!(
            shows(Open {
                settling: false,
                waiting: true,
                ..landed
            }),
            "the viewer waits for an extension's effects"
        );
        assert!(!shows(Open { native: true, ..chosen }), "an extension's own page");
        assert!(!shows(Open::default()), "failed or cancelled: nothing chosen");
    }

    /// The ghost takes exactly the box the panel's body takes (within 1 pt).
    #[::core::prelude::v1::test]
    fn a_ghost_is_the_size_of_the_body_it_stands_for() {
        let t = Tokens::new(Scheme::Dark, Accessibility::default());
        let panel = Rect {
            x: 8.,
            y: 52.,
            w: 301.5,
            h: 467.25,
        };
        let (w, h) = body_size(panel, &t);
        assert!((w - panel.w).abs() <= 1. && (h - (panel.h - t.metrics.header)).abs() <= 1.);
        // A panel folded to its header has no body to ghost.
        let folded = Rect {
            h: t.metrics.header,
            ..panel
        };
        assert_eq!(body_size(folded, &t).1, 0.);
    }

    #[::core::prelude::v1::test]
    fn every_panel_has_a_ghost_of_its_kind() {
        assert_eq!(shape("viewer"), Shape::Viewer);
        assert_eq!(shape("timeline"), Shape::Timeline);
        assert_eq!(shape("inspector"), Shape::Fields);
        assert_eq!(shape("fxbrowser"), Shape::Search);
        assert_eq!(shape("assets"), Shape::Items);
        assert_eq!(shape("notes"), Shape::Rows);
    }

    #[::core::prelude::v1::test]
    fn real_content_fades_in_over_the_menu_fade_and_at_once_under_reduce_motion() {
        let t = Tokens::new(Scheme::Dark, Accessibility::default());
        assert_eq!(reveal(Duration::ZERO, &t), 0.);
        let half = Duration::from_secs_f64(t.motion.menu_fade / 2.);
        assert!((reveal(half, &t) - 0.5).abs() < 0.01);
        assert_eq!(reveal(Duration::from_secs(1), &t), 1.);
        let still = Tokens::new(
            Scheme::Dark,
            Accessibility {
                reduce_motion: true,
                ..Default::default()
            },
        );
        assert_eq!(reveal(Duration::ZERO, &still), 1.);
    }
}
