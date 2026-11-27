//! Vendored glyphs (`icons/`, see its README): where one stands for an SF
//! Symbol of the same meaning, the app draws it instead, in the same place,
//! size and tint. Pierre's set (Apache-2.0) covers the general chrome, its
//! filled variant wherever it has one; glyphs it has no counterpart for are
//! drawn on the same grid in the same grammar (`icons/powermove`). SF Symbols
//! stay where AppKit itself draws the glyph inside a control (pop-up chevrons,
//! switches, menu marks, the window buttons) and for names not mapped here.
use resvg::{tiny_skia, usvg};

/// A vendored glyph: its SVG (a 16 px grid; a wider one is fitted by its
/// longer side), how many quarter turns (clockwise) it is drawn at, whether
/// it is mirrored left to right first, and its size against the shared
/// `FILL` (1 for all but a glyph matched by eye to the SF Symbol it replaced),
/// and, for a product mark that keeps its own colour (0xRRGGBB), that colour
/// in place of the surface's tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub svg: &'static str,
    pub turns: u8,
    pub mirror: bool,
    pub size: f32,
    pub color: Option<u32>,
}

macro_rules! pierre {
    ($name:literal) => {
        include_str!(concat!("../icons/pierre/Icon", $name, ".svg"))
    };
}
macro_rules! drawn {
    ($name:literal) => {
        include_str!(concat!("../icons/powermove/", $name, ".svg"))
    };
}

/// The vendored glyph that stands for SF Symbol `name`, if one does.
pub fn glyph(name: &str) -> Option<Glyph> {
    let g = |svg| {
        Some(Glyph {
            svg,
            turns: 0,
            mirror: false,
            size: 1.,
            color: None,
        })
    };
    let turned = |svg, turns| {
        Some(Glyph {
            svg,
            turns,
            mirror: false,
            size: 1.,
            color: None,
        })
    };
    match name {
        // Home stands among SF Symbols in the toolbar: the filled house at
        // the optical size of the SF `house` it replaced (14 pt ink in a
        // 14 pt square, where the shared fill would make it 12.25 pt).
        "house" => Some(Glyph {
            svg: pierre!("HomeFill"),
            turns: 0,
            mirror: false,
            size: 16. / 14.,
            color: None,
        }),
        "align.horizontal.left" => g(pierre!("AlignLeft")),
        "align.horizontal.center" => g(pierre!("AlignCenter")),
        "align.horizontal.right" => g(pierre!("AlignRight")),
        // The same three drawings turned a quarter: one family.
        "align.vertical.top" => turned(pierre!("AlignLeft"), 1),
        "align.vertical.center" => turned(pierre!("AlignCenter"), 1),
        "align.vertical.bottom" => turned(pierre!("AlignRight"), 1),
        "distribute.horizontal.center" => g(drawn!("distribute-horizontal")),
        "distribute.vertical.center" => turned(drawn!("distribute-horizontal"), 1),
        // Chrome.
        // Pierre's plus spans 10 of 16 px in a 1.5 px line: beside the
        // search glass it reads small and thin, so it is drawn full size.
        "plus" => g(drawn!("plus")),
        // The Agent composer's "+": the same plus in Pierre's 1.5 px line,
        // drawn large and light beside the draft.
        "composer.add" => g(drawn!("plus-thin")),
        // The composer's Send and Stop, inside their circle.
        "composer.send" => turned(pierre!("Arrow"), 1),
        "composer.stop" => g(drawn!("stop")),
        // The model chooser's one small chevron.
        "composer.chevron" => g(pierre!("ChevronSm")),
        // Provider marks beside the model's name. OpenAI's blossom
        // identifies ChatGPT and OpenAI, monochrome (as in Settings, see its
        // ATTRIBUTION); the Claude logo identifies Claude, Claude Code and
        // Anthropic keys, in its own colour (SVGL's file, unchanged, same
        // ATTRIBUTION); custom endpoints show Pierre's neutral sparkle.
        "provider.chatgpt" => Some(Glyph {
            svg: include_str!("../../app/assets/settings-agent-icons/openai-blossom.svg"),
            turns: 0,
            mirror: false,
            // The official file keeps clear space round the mark (about half
            // its square): drawn at the sparkle's size.
            size: 2.,
            color: None,
        }),
        "provider.claude" | "provider.anthropic" => Some(Glyph {
            svg: include_str!("../../app/assets/settings-agent-icons/claude-ai-icon.svg"),
            turns: 0,
            mirror: false,
            size: 1.,
            color: Some(0xD97757),
        }),
        "provider.compatible" => g(pierre!("Sparkle")),
        // The Powermove harness: the app's own mark (assets/brand), filled
        // in one ink, a touch smaller than its box as the sparkle is.
        "provider.powermove" => Some(Glyph {
            svg: drawn!("powermove"),
            turns: 0,
            mirror: false,
            size: 0.8,
            color: None,
        }),
        "minus" => g(pierre!("Minus")),
        "xmark" => g(pierre!("X")),
        "ellipsis" => g(pierre!("Ellipsis")),
        "magnifyingglass" => g(pierre!("Search")),
        "gearshape" => g(pierre!("GearFill")),
        "sparkles" => g(pierre!("Sparkles")),
        "square.grid.2x2" => g(drawn!("grid")),
        "trash" => g(pierre!("BinFill")),
        "clock.fill" => g(drawn!("recents")),
        "basket.fill" => g(drawn!("store")),
        "person.crop.circle.fill" => g(drawn!("account")),
        "folder" => g(pierre!("Folder")),
        "list.bullet" => g(pierre!("ListUnordered")),
        "link" => g(pierre!("Link")),
        "icloud.and.arrow.down" => g(pierre!("Download")),
        "exclamationmark.triangle.fill" => g(pierre!("CiWarningFill")),
        "arrow.clockwise" => g(pierre!("Reload")),
        "arrow.counterclockwise" => Some(Glyph {
            svg: pierre!("Reload"),
            turns: 0,
            mirror: true,
            size: 1.,
            color: None,
        }),
        // Pierre's chevron points down; the others are it turned.
        "chevron.down" => g(pierre!("Chevron")),
        "chevron.left" => turned(pierre!("Chevron"), 1),
        "chevron.up" => turned(pierre!("Chevron"), 2),
        "chevron.right" => turned(pierre!("Chevron"), 3),
        // Tools (the toolbar's segments and the Shape tool's kinds).
        "cursorarrow" => g(pierre!("Cursor")),
        "crop.rotate" => g(pierre!("Reload")),
        "scope" => g(drawn!("anchor")),
        "square.on.circle" => g(drawn!("shape")),
        // The Pen tool draws paths: a nib, not Pierre's pencil (rename).
        "pencil.tip" => g(drawn!("pen")),
        "textformat" => g(pierre!("Type")),
        "square" | "square.fill" => g(drawn!("rectangle")),
        "app" => g(pierre!("Squircle")),
        "circle" => g(pierre!("CircleFill")),
        "hexagon" => g(drawn!("hexagon")),
        "star" => g(pierre!("StarFill")),
        // Timeline rows: switches and transport.
        "eye" => g(pierre!("Eye")),
        "eye.slash" => g(pierre!("EyeSlash")),
        "lock" => g(pierre!("Lock")),
        "lock.open" => g(pierre!("LockOpen")),
        "shield.fill" => g(pierre!("ShieldFill")),
        "speaker.wave.2" => g(drawn!("speaker")),
        "speaker.slash" => g(drawn!("speaker-off")),
        // Solo, as After Effects shows it: a dot, small while off (the
        // timeline dims it), larger in the accent while on.
        "s.circle.fill" => g(drawn!("solo")),
        "s.circle" => g(drawn!("solo-off")),
        // Shy: the filled face in both states (its dashed twin reads as a
        // sun at 1x); the timeline dims it while off.
        "face.dashed" | "face.dashed.fill" => g(pierre!("EmojiSmileFill")),
        "play.fill" => g(drawn!("play")),
        "pause.fill" => g(drawn!("pause")),
        "backward.end.fill" => g(drawn!("backward-end")),
        "backward.frame.fill" => g(drawn!("backward-frame")),
        "forward.frame.fill" => g(drawn!("forward-frame")),
        "forward.end.fill" => g(drawn!("forward-end")),
        "diamond" => g(drawn!("diamond-outline")),
        "diamond.fill" => g(drawn!("diamond")),
        "diamond.lefthalf.filled" => g(drawn!("diamond-half")),
        "wand.and.rays" | "wand.and.stars" => g(drawn!("wand")),
        "point.topleft.down.to.point.bottomright.curvepath" => g(drawn!("path")),
        "arrow.right.and.line.vertical.and.arrow.left" => g(drawn!("squeeze")),
        "captions.bubble" => g(pierre!("CommentFill")),
        // Layer kinds (rows, the Project panel).
        "film" => g(drawn!("film")),
        "film.stack" => g(drawn!("film-stack")),
        "photo" | "photo.on.rectangle" | "photo.on.rectangle.angled.fill" => g(pierre!("Image")),
        "photo.stack" => g(drawn!("photo-stack")),
        "waveform" => g(drawn!("waveform")),
        "music.note" => g(drawn!("note")),
        "slider.horizontal.3" => g(drawn!("sliders")),
        "circle.dashed" => g(drawn!("null")),
        "camera" => g(drawn!("camera")),
        "square.stack" => g(drawn!("stack")),
        "rectangle.stack" => g(drawn!("rect-stack")),
        "square.dashed" => g(drawn!("dashed")),
        "lightbulb" => g(pierre!("BulbFill")),
        "square.stack.3d.up" | "square.stack.3d.down.right" => g(pierre!("Layers")),
        // Agent provider marks in the composer model picker.
        "provider.openai" => g(drawn!("provider-openai")),
        "provider.claude" => g(drawn!("provider-claude")),
        _ => None,
    }
}

/// How much of a symbol's square a vendored glyph fills: the 16 px grid
/// drawn edge to edge reads larger than an SF Symbol of the same point size,
/// so it stands a little inside (matched by eye against SF neighbours).
pub const FILL: f32 = 14. / 16.;

/// `glyph`'s coverage (0–255 per pixel) on a `side` × `side` canvas, centred,
/// its longer side filling `FILL` × `glyph.size` of it.
pub fn coverage(glyph: Glyph, side: u32) -> Option<Vec<u8>> {
    turned(glyph, side, 0.)
}

/// `glyph`'s coverage turned `degrees` clockwise about its centre (a
/// chevron springing round to point up while its menu is open).
fn turned(glyph: Glyph, side: u32, degrees: f32) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_str(glyph.svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = side as f32 * FILL * glyph.size / w.max(h);
    let (cx, cy) = (w / 2., h / 2.);
    let half = side as f32 / 2.;
    let mut transform = tiny_skia::Transform::from_translate(half, half)
        .pre_rotate(degrees)
        .pre_scale(scale, scale)
        .pre_rotate(90. * glyph.turns as f32);
    if glyph.mirror {
        transform = transform.pre_scale(-1., 1.);
    }
    let transform = transform.pre_translate(-cx, -cy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap.pixels().iter().map(|p| p.alpha()).collect())
}

/// `glyph` tinted `color` (0xRRGGBB), or in its own colour, turned
/// `degrees` clockwise, as straight BGRA, GPUI's sprite format.
pub fn tinted_bgra(glyph: Glyph, side: u32, color: u32, degrees: f32) -> Option<Vec<u8>> {
    let color = glyph.color.unwrap_or(color);
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    Some(
        turned(glyph, side, degrees)?
            .into_iter()
            .flat_map(|a| [b, g, r, a])
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mapped symbol the main surfaces draw.
    const MAPPED: &[&str] = &[
        "house",
        "align.horizontal.left",
        "align.horizontal.center",
        "align.horizontal.right",
        "align.vertical.top",
        "align.vertical.center",
        "align.vertical.bottom",
        "distribute.horizontal.center",
        "distribute.vertical.center",
        "plus",
        "composer.add",
        "composer.send",
        "composer.stop",
        "composer.chevron",
        "provider.chatgpt",
        "provider.claude",
        "provider.anthropic",
        "provider.compatible",
        "provider.powermove",
        "minus",
        "xmark",
        "ellipsis",
        "magnifyingglass",
        "gearshape",
        "sparkles",
        "square.grid.2x2",
        "trash",
        "folder",
        "list.bullet",
        "link",
        "icloud.and.arrow.down",
        "exclamationmark.triangle.fill",
        "arrow.clockwise",
        "arrow.counterclockwise",
        "chevron.down",
        "chevron.left",
        "chevron.up",
        "chevron.right",
        "cursorarrow",
        "crop.rotate",
        "scope",
        "square.on.circle",
        "pencil.tip",
        "textformat",
        "square",
        "square.fill",
        "app",
        "circle",
        "hexagon",
        "star",
        "eye",
        "eye.slash",
        "lock",
        "lock.open",
        "shield.fill",
        "speaker.wave.2",
        "speaker.slash",
        "s.circle.fill",
        "s.circle",
        "face.dashed",
        "face.dashed.fill",
        "play.fill",
        "pause.fill",
        "backward.end.fill",
        "backward.frame.fill",
        "forward.frame.fill",
        "forward.end.fill",
        "diamond",
        "diamond.fill",
        "diamond.lefthalf.filled",
        "wand.and.rays",
        "wand.and.stars",
        "point.topleft.down.to.point.bottomright.curvepath",
        "arrow.right.and.line.vertical.and.arrow.left",
        "captions.bubble",
        "film",
        "film.stack",
        "photo",
        "photo.on.rectangle",
        "photo.on.rectangle.angled.fill",
        "photo.stack",
        "waveform",
        "music.note",
        "slider.horizontal.3",
        "circle.dashed",
        "camera",
        "square.stack",
        "rectangle.stack",
        "square.dashed",
        "lightbulb",
        "square.stack.3d.up",
        "square.stack.3d.down.right",
        "provider.openai",
        "provider.claude",
    ];

    /// Every mapped glyph parses, draws inside its square (nothing clipped
    /// at the canvas edge) and covers a fair part of it.
    #[test]
    fn every_glyph_draws_inside_its_square() {
        let side = 64usize;
        for name in MAPPED {
            let c = coverage(
                glyph(name).unwrap_or_else(|| panic!("{name} unmapped")),
                side as u32,
            )
            .unwrap();
            let inked = c.iter().filter(|a| **a > 128).count();
            assert!(inked > side * side / 40, "{name} draws too little: {inked}");
            let edge =
                (0..side).flat_map(|i| [i, (side - 1) * side + i, i * side, i * side + side - 1]);
            // Home alone fills its square (matched to the SF house's size).
            if *name != "house" {
                assert!(
                    edge.map(|i| c[i]).all(|a| a < 128),
                    "{name} touches its square's edge"
                );
            }
        }
    }

    /// A quarter turn moves an align-left glyph's edge line to the top; a
    /// mirror moves Reload's arrowhead to the other side; a 16 × 18 grid is
    /// fitted by its longer side.
    #[test]
    fn turns_mirrors_and_wide_grids() {
        let side = 32usize;
        let at = |c: &[u8], x: usize, y: usize| c[y * side + x];
        let left = coverage(glyph("align.horizontal.left").unwrap(), 32).unwrap();
        let top = coverage(glyph("align.vertical.top").unwrap(), 32).unwrap();
        let edge = (32. - 32. * FILL) as usize / 2 + 1;
        assert!(at(&left, edge, 16) > 200 && at(&left, 16, edge) < 50);
        assert!(at(&top, 16, edge) > 200 && at(&top, edge, 16) < 50);
        let cw = coverage(glyph("arrow.clockwise").unwrap(), 32).unwrap();
        let ccw = coverage(glyph("arrow.counterclockwise").unwrap(), 32).unwrap();
        // The ink of each half moves to the other one.
        let half = |c: &[u8], left: bool| -> u32 {
            (0..side)
                .flat_map(|y| (0..side / 2).map(move |x| (x, y)))
                .map(|(x, y)| at(c, if left { x } else { side - 1 - x }, y) as u32)
                .sum()
        };
        let (l, r) = (half(&cw, true), half(&ccw, false));
        assert!(l.abs_diff(r) * 50 < l, "{l} vs {r}");
        assert!(
            half(&cw, true).abs_diff(half(&cw, false)) * 10 > half(&cw, true),
            "Reload is not symmetric"
        );
        let folder = coverage(glyph("folder").unwrap(), 32).unwrap();
        let columns = (0..side)
            .filter(|x| (0..side).any(|y| at(&folder, *x, y) > 128))
            .count();
        assert!(columns as f32 <= 32. * FILL + 1., "{columns} columns");
    }

    /// Home stands at the optical size of the SF `house` it replaced.
    #[test]
    fn home_is_drawn_at_the_sf_house_size() {
        let c = coverage(glyph("house").unwrap(), 140).unwrap();
        let rows = (0..140)
            .filter(|y| (0..140).any(|x| c[y * 140 + x] > 128))
            .count();
        assert!((138..=140).contains(&rows), "{rows} of 140 rows");
    }
}
