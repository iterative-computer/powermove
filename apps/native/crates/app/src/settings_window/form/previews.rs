//! Settings' visual pickers: each option a rounded tile on a wallpaper in
//! Powermove's dusk, with a crisp mini window inside (traffic lights, clean
//! panes) in the appearance it shows. The selected option is ringed in the
//! accent around the whole tile, concentric with it. Every size and colour is
//! a token (`pm_design::Settings`, its `PickerPalette`, `Colors`).
use objc2::rc::Retained;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSImage};
use objc2_foundation::{NSArray, NSRect, NSSize};
use pm_design::{Colors, Scheme, Settings, css};

fn tokens(scheme: Scheme) -> Settings {
    Settings::of(scheme)
}

/// The room around the tile for its ring.
fn margin(s: &Settings) -> f64 {
    (s.picker_ring_gap + s.picker_ring) as f64
}

pub(super) fn image_size() -> NSSize {
    let s = tokens(Scheme::Dark);
    let m = margin(&s);
    NSSize::new(
        s.picker_width as f64 + 2. * m,
        s.picker_height as f64 + 2. * m,
    )
}

/// One mini window at (x, y) of `w`×`h` in `scheme`'s colours: a title row
/// with traffic lights (and `tabs` project tabs after them), a sidebar pane
/// and two content panes over a wide one.
fn window(s: &Settings, x: f64, y: f64, w: f64, h: f64, scheme: Scheme, tabs: usize) -> String {
    let c = Colors::of(scheme);
    let p = s.picker;
    let r = s.picker_window_radius as f64;
    let ink = css(c.text);
    // Proportions: a title row, then panes with a hairline of window between.
    let top = h * 0.2;
    let pad = (w * 0.035).max(1.5);
    let dot = (h * 0.045).max(1.1);
    let side = w * 0.26;
    let (px, py) = (x + pad, y + top);
    let pw = w - 2. * pad;
    let ph = h - top - pad;
    let cx = px + side + pad;
    let cw = pw - side - pad;
    let upper = ph * 0.58;
    let lower = ph - upper - pad;
    let half = (cw - pad) / 2.;
    let pane = |x: f64, y: f64, w: f64, h: f64, fill: u32| {
        format!(
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{pr}" fill="{f}"/>"#,
            pr = (r - pad).max(1.),
            f = css(fill)
        )
    };
    let mut out = format!(
        r#"<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{win}"/></g>"#,
        win = css(c.window),
    );
    // Panes a clear step off the window: raised in the dark, paper in the
    // light; the wide one a step apart from the small ones.
    let dark = scheme == Scheme::Dark;
    let (small, wide) = if dark {
        (c.floating, c.panel)
    } else {
        (c.panel, c.panel_secondary)
    };
    out += &pane(px, py, side, ph, small);
    out += &pane(cx, py, cw, upper, wide);
    out += &pane(cx, py + upper + pad, half, lower, small);
    out += &pane(cx + half + pad, py + upper + pad, half, lower, small);
    // The traffic lights, then the project tabs after them.
    let cy = y + top / 2.;
    for (i, colour) in [p.close, p.minimize, p.zoom].into_iter().enumerate() {
        out += &format!(
            r#"<circle cx="{}" cy="{cy}" r="{dot}" fill="{}"/>"#,
            x + pad + dot + i as f64 * dot * 2.8,
            css(colour)
        );
    }
    let tab_w = w * 0.2;
    for i in 0..tabs {
        let tx = x + pad + dot * 8.6 + i as f64 * (tab_w + pad);
        let opacity = if i == 0 {
            s.picker.window_edge * 1.6
        } else {
            s.picker.window_edge * 0.8
        };
        out += &format!(
            r#"<rect x="{tx}" y="{ty}" width="{tab_w}" height="{th}" rx="{tr}" fill="{ink}" fill-opacity="{opacity}"/>"#,
            ty = cy - dot * 1.4,
            th = dot * 2.8,
            tr = dot * 1.4,
        );
    }
    out += &format!(
        r#"<rect x="{ex}" y="{ey}" width="{ew}" height="{eh}" rx="{r}" fill="none" stroke="{ink}" stroke-opacity="{eo}" stroke-width="0.5"/>"#,
        ex = x + 0.25,
        ey = y + 0.25,
        ew = w - 0.5,
        eh = h - 0.5,
        eo = p.window_edge,
    );
    out
}

/// Aerolite's monthly credits against Aero's (1,000 of 2,500, as
/// `apps/www`'s Pricing and the billing contract have them).
const LITE_SHARE: f64 = 1000. / 2500.;

/// A plan's picture: its monthly credits as a ring on the wallpaper, full
/// for the bigger plan, drawn clockwise from the top like a gauge.
fn credits_ring(w: f64, h: f64, share: f64, c: &Colors) -> String {
    let r = h * 0.28;
    let stroke = h * 0.09;
    let (cx, cy) = (w / 2., h / 2.);
    let ink = css(c.white);
    let track = pm_design::Ink::new().tint;
    let circumference = std::f64::consts::TAU * r;
    format!(
        r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{ink}" stroke-opacity="{track}" stroke-width="{stroke}"/>
<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{ink}" stroke-width="{stroke}" stroke-linecap="round" stroke-dasharray="{on} {circumference}" transform="rotate(-90 {cx} {cy})" filter="url(#shadow)"/>"#,
        on = circumference * share,
    )
}

/// The wallpaper: a dusk sky, a dark fold across it and a warm glow low down.
fn wallpaper(s: &Settings, w: f64, h: f64) -> String {
    let p = s.picker;
    format!(
        r#"<rect width="{w}" height="{h}" fill="url(#sky)"/>
<path d="M{a} {h} C{b} {c} {d} {e} {w} {f} L{w} {h} Z" fill="{fold}" fill-opacity="{fo}" filter="url(#soft)"/>
<path d="M0 {g} C{i} {j} {k} {l} {w} {m} L{w} {h} L0 {h} Z" fill="{fold}" fill-opacity="{fo2}" filter="url(#soft)"/>
<path d="M{sa} {h} C{sb} {sc} {sd} {se} {w} {sf}" fill="none" stroke="{sheen}" stroke-opacity="{so}" stroke-width="{sw}" filter="url(#soft)"/>
<ellipse cx="{gx}" cy="{gy}" rx="{grx}" ry="{gry}" fill="{glow}" fill-opacity="{go}" filter="url(#soft)"/>"#,
        sa = w * 0.05,
        sb = w * 0.35,
        sc = h * 0.45,
        sd = w * 0.65,
        se = h * 0.3,
        sf = h * 0.12,
        sheen = css(p.sheen),
        so = p.sheen_opacity,
        sw = h * 0.08,
        a = -w * 0.1,
        b = w * 0.3,
        c = h * 0.35,
        d = w * 0.6,
        e = h * 0.15,
        f = h * 0.05,
        g = h * 0.7,
        i = w * 0.35,
        j = h * 0.55,
        k = w * 0.7,
        l = h * 0.95,
        m = h * 0.75,
        fold = css(p.fold),
        fo = p.fold_opacity,
        fo2 = p.fold_opacity * 0.8,
        gx = w * 0.7,
        gy = h * 1.05,
        grx = w * 0.45,
        gry = h * 0.25,
        glow = css(p.glow),
        go = p.glow_opacity,
    )
}

/// The picture of `option` in a picker of `kind` ("appearance": system,
/// light, dark; "projects": tabs, windows), for an appearance that is
/// `dark`; ringed when `selected`.
pub(super) fn svg(kind: &str, option: &str, selected: bool, dark: bool) -> String {
    let here = if dark { Scheme::Dark } else { Scheme::Light };
    let s = tokens(here);
    let ours = Colors::of(here);
    let p = s.picker;
    let (w, h) = (s.picker_width as f64, s.picker_height as f64);
    let radius = s.picker_radius as f64;
    // The mini window: centred, most of the tile.
    let (ww, wh) = (w * 0.64, h * 0.66);
    let (wx, wy) = ((w - ww) / 2., (h - wh) / 2. + h * 0.04);
    let body = match (kind, option) {
        ("appearance", "light") => window(&s, wx, wy, ww, wh, Scheme::Light, 0),
        ("appearance", "dark") => window(&s, wx, wy, ww, wh, Scheme::Dark, 0),
        // Match System: light on the left, dark on the right, split on a
        // diagonal through the window.
        ("appearance", _) => format!(
            r#"{dark}<g clip-path="url(#half)">{light}</g>"#,
            dark = window(&s, wx, wy, ww, wh, Scheme::Dark, 0),
            light = window(&s, wx, wy, ww, wh, Scheme::Light, 0),
        ),
        ("plans", plan) => credits_ring(w, h, if plan == "aero" { 1. } else { LITE_SHARE }, &ours),
        ("projects", "tabs") => window(&s, wx, wy, ww, wh, here, 2),
        // Separate windows: one behind another.
        ("projects", _) => {
            let (bw, bh) = (ww * 0.86, wh * 0.86);
            format!(
                r#"{back}{front}"#,
                back = window(&s, wx - w * 0.06, wy - h * 0.08, bw, bh, here, 0),
                front = window(&s, wx + w * 0.08, wy + h * 0.06, bw, bh, here, 0),
            )
        }
        _ => String::new(),
    };
    let gap = s.picker_ring_gap as f64;
    let ring_w = s.picker_ring as f64;
    let ring = if selected {
        // Concentric with the tile: its radius plus the gap to its centre line.
        let o = gap + ring_w / 2.;
        format!(
            r#"<rect x="{x}" y="{x}" width="{rw}" height="{rh}" rx="{rr}" fill="none" stroke="{accent}" stroke-width="{ring_w}"/>"#,
            x = -o,
            rw = w + 2. * o,
            rh = h + 2. * o,
            rr = radius + o,
            accent = css(ours.accent),
        )
    } else {
        String::new()
    };
    let m = margin(&s);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{iw}" height="{ih}" viewBox="{nm} {nm} {iw} {ih}">
<defs>
<linearGradient id="sky" x1="0" y1="0" x2="0.35" y2="1"><stop offset="0" stop-color="{s0}"/><stop offset="0.55" stop-color="{s1}"/><stop offset="1" stop-color="{s2}"/></linearGradient>
<filter id="soft" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="{blur}"/></filter>
<filter id="shadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="{sd}" stdDeviation="{sb}" flood-color="{shade}" flood-opacity="{so}"/></filter>
<clipPath id="tile"><rect width="{w}" height="{h}" rx="{radius}"/></clipPath>
<clipPath id="half"><path d="M0 0H{ha}L{hb} {h}H0Z"/></clipPath>
</defs>
<g clip-path="url(#tile)">{paper}{body}</g>
<rect x="0.25" y="0.25" width="{ew}" height="{eh}" rx="{radius}" fill="none" stroke="{ink}" stroke-opacity="{eo}" stroke-width="0.5"/>
{ring}</svg>"#,
        iw = w + 2. * m,
        ih = h + 2. * m,
        nm = -m,
        s0 = css(p.sky_top),
        s1 = css(p.sky_mid),
        s2 = css(p.sky_bottom),
        blur = h * 0.06,
        sd = h * 0.03,
        sb = h * 0.05,
        shade = css(s.tile.shadow),
        so = p.window_shadow,
        ha = w * 0.58,
        hb = w * 0.42,
        paper = wallpaper(&s, w, h),
        ew = w - 0.5,
        eh = h - 0.5,
        ink = css(ours.text),
        eo = p.tile_edge,
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

#[cfg(test)]
mod tests {
    /// Every picture is well-formed SVG; with `PM_PREVIEW_DIR` set, each is
    /// written there to look at.
    #[test]
    fn every_picture_is_an_svg() {
        let out = std::env::var_os("PM_PREVIEW_DIR");
        for (kind, option) in [
            ("appearance", "system"),
            ("appearance", "light"),
            ("appearance", "dark"),
            ("projects", "tabs"),
            ("projects", "windows"),
            ("plans", "aerolite"),
            ("plans", "aero"),
        ] {
            for dark in [true, false] {
                for selected in [true, false] {
                    let svg = super::svg(kind, option, selected, dark);
                    assert!(
                        svg.starts_with("<svg") && svg.ends_with("</svg>"),
                        "{kind} {option}"
                    );
                    if let Some(dir) = &out {
                        let name = format!(
                            "{kind}-{option}-{}-{}.svg",
                            if dark { "dark" } else { "light" },
                            selected as u8
                        );
                        std::fs::write(std::path::Path::new(dir).join(name), &svg).unwrap();
                    }
                }
            }
        }
    }
}
