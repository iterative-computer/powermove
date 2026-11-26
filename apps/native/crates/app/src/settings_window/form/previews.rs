//! Settings' visual pickers: each option a small picture of Powermove's own
//! editor (its panels, viewer and timeline in the design tokens' colours of
//! the appearance it shows), never a drawing of the system's window chrome.
//! The selected option is ringed in the accent, concentric with the picture.
use objc2::rc::Retained;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSImage};
use objc2_foundation::{NSArray, NSRect, NSSize};
use pm_design::{Colors, Scheme};

/// The picture's size in points, and the ring's room around it.
pub(super) const SIZE: (f64, f64) = (120., 75.);
const RING: f64 = 4.;
const RADIUS: f64 = 7.;

pub(super) fn image_size() -> NSSize {
    NSSize::new(SIZE.0 + 2. * RING, SIZE.1 + 2. * RING)
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xff_ffff)
}

/// One editor window at (x, y) of `w`×`h`, in `scheme`'s colours; `tabs`
/// project tabs in its title row (the first selected).
fn editor(x: f64, y: f64, w: f64, h: f64, scheme: Scheme, tabs: usize) -> String {
    let c = Colors::of(scheme);
    let ink = hex(c.text);
    let line = |x: f64, y: f64, w: f64, o: f64| {
        format!(
            r#"<rect x="{x}" y="{y}" width="{w}" height="1.6" rx="0.8" fill="{ink}" fill-opacity="{o}"/>"#
        )
    };
    // Proportions of the editor: a title row, project | viewer | inspector
    // over the timeline.
    let top = 9.;
    let pad = 3.;
    let col = w * 0.22;
    let insp = w * 0.2;
    let mid = h * 0.58;
    let (px, py) = (x + pad, y + top);
    let vx = px + col + 2.;
    let vw = w - 2. * pad - col - insp - 4.;
    let ix = vx + vw + 2.;
    let ty = py + mid + 2.;
    let th = h - top - mid - 2. - pad;
    let tw = w - 2. * pad - col - 2.;
    let labels = c.labels;
    let mut s = format!(
        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{RADIUS}" fill="{window}"/>
<rect x="{px}" y="{py}" width="{col}" height="{ph}" rx="2.5" fill="{panel}"/>
<rect x="{vx}" y="{py}" width="{vw}" height="{mid}" rx="2.5" fill="{viewer}"/>
<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" rx="1" fill="{frame}"/>
<rect x="{ix}" y="{py}" width="{insp}" height="{mid}" rx="2.5" fill="{panel}"/>
<rect x="{vx}" y="{ty}" width="{tw}" height="{th}" rx="2.5" fill="{panel}"/>
<rect x="{c1x}" y="{r1}" width="{c1w}" height="3" rx="1" fill="{l1}"/>
<rect x="{c2x}" y="{r2}" width="{c2w}" height="3" rx="1" fill="{l2}"/>
<rect x="{c3x}" y="{r3}" width="{c3w}" height="3" rx="1" fill="{l3}"/>
<rect x="{phx}" y="{ty}" width="0.8" height="{th}" fill="{accent}"/>"#,
        window = hex(c.window),
        panel = hex(c.panel),
        viewer = hex(c.viewer),
        ph = h - top - pad,
        frame = hex(if scheme == Scheme::Dark {
            c.panel_secondary
        } else {
            c.floating
        }),
        fw = vw * 0.7,
        fh = vw * 0.7 * 9. / 16.,
        fx = vx + vw * 0.15,
        fy = py + (mid - vw * 0.7 * 9. / 16.) / 2.,
        c1x = vx + 3.,
        c1w = tw * 0.45,
        r1 = ty + 3.,
        l1 = hex(labels.orange),
        c2x = vx + 3. + tw * 0.2,
        c2w = tw * 0.5,
        r2 = ty + 7.5,
        l2 = hex(labels.blue),
        c3x = vx + 3. + tw * 0.08,
        c3w = tw * 0.3,
        r3 = ty + 12.,
        l3 = hex(labels.violet),
        phx = vx + tw * 0.38,
        accent = hex(c.accent),
    );
    for i in 0..3 {
        s += &line(
            px + 2.5,
            py + 4. + 4. * i as f64,
            col * [0.7, 0.5, 0.6][i],
            0.28,
        );
    }
    for i in 0..2 {
        s += &line(
            ix + 2.5,
            py + 4. + 4. * i as f64,
            insp * [0.6, 0.45][i],
            0.28,
        );
    }
    for i in 0..tabs {
        let tx = x + 6. + i as f64 * 19.;
        let fill = if i == 0 {
            format!(r#"fill="{ink}" fill-opacity="0.16""#)
        } else {
            String::new()
        };
        s += &format!(
            r#"<rect x="{tx}" y="{ty}" width="17" height="5" rx="2.5" {fill}/>"#,
            ty = y + 2.
        );
        s += &line(tx + 3.5, y + 3.7, 10., if i == 0 { 0.6 } else { 0.32 });
    }
    s
}

/// The picture of `option` in a picker of `kind` ("appearance": system,
/// light, dark; "projects": tabs, windows), for an appearance that is
/// `dark`; ringed when `selected`.
pub(super) fn svg(kind: &str, option: &str, selected: bool, dark: bool) -> String {
    let (w, h) = SIZE;
    let here = if dark { Scheme::Dark } else { Scheme::Light };
    let ours = Colors::of(here);
    let body = match (kind, option) {
        ("appearance", "light") => editor(0., 0., w, h, Scheme::Light, 0),
        ("appearance", "dark") => editor(0., 0., w, h, Scheme::Dark, 0),
        ("appearance", _) => format!(
            r#"<clipPath id="half"><path d="M0 0H{a}L{b} {h}H0Z"/></clipPath>{dark}<g clip-path="url(#half)">{light}</g>"#,
            a = w * 0.62,
            b = w * 0.38,
            dark = editor(0., 0., w, h, Scheme::Dark, 0),
            light = editor(0., 0., w, h, Scheme::Light, 0),
        ),
        ("projects", "tabs") => editor(0., 0., w, h, here, 2),
        ("projects", _) => format!(
            r#"<g opacity="0.55">{back}</g>{front}"#,
            back = editor(0., 0., w * 0.78, h * 0.78, here, 0),
            front = editor(w * 0.22, h * 0.22, w * 0.78, h * 0.78, here, 0),
        ),
        _ => String::new(),
    };
    // The ring is concentric with the picture: its radius is the picture's
    // plus the gap between them.
    let ring = if selected {
        format!(
            r#"<rect x="-3" y="-3" width="{rw}" height="{rh}" rx="{rr}" fill="none" stroke="{accent}" stroke-width="2"/>"#,
            rw = w + 6.,
            rh = h + 6.,
            rr = RADIUS + 3.,
            accent = hex(ours.accent),
        )
    } else {
        format!(
            // A 1 pt edge at 10 % ink, so a light picture holds its shape on
            // a light card.
            r#"<rect x="0.5" y="0.5" width="{rw}" height="{rh}" rx="{RADIUS}" fill="none" stroke="{ink}" stroke-opacity="0.1" stroke-width="1"/>"#,
            rw = w - 1.,
            rh = h - 1.,
            ink = hex(ours.text),
        )
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{iw}" height="{ih}" viewBox="{m} {m} {iw} {ih}"><clipPath id="pic"><rect width="{w}" height="{h}" rx="{RADIUS}"/></clipPath><g clip-path="url(#pic)">{body}</g>{ring}</svg>"#,
        iw = w + 2. * RING,
        ih = h + 2. * RING,
        m = -RING,
    )
}

/// The option's picture for whichever appearance draws it: AppKit asks at
/// draw time, so a change of appearance redraws it right.
pub(super) fn image(kind: &str, option: &str, selected: bool) -> Option<Retained<NSImage>> {
    let size = image_size();
    let light = pm_shell::settings_window::svg_image(&svg(kind, option, selected, false), size)?;
    let dark = pm_shell::settings_window::svg_image(&svg(kind, option, selected, true), size)?;
    let draw = block2::RcBlock::new(move |rect: NSRect| -> objc2::runtime::Bool {
        let names = unsafe { [NSAppearanceNameAqua, NSAppearanceNameDarkAqua] };
        let dark_now = NSAppearance::currentDrawingAppearance()
            .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&names))
            .is_some_and(|name| &*name == names[1]);
        (if dark_now { &dark } else { &light }).drawInRect(rect);
        objc2::runtime::Bool::YES
    });
    Some(NSImage::imageWithSize_flipped_drawingHandler(
        size, false, &draw,
    ))
}
