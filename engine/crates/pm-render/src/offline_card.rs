//! Powermove's Media Offline card (docs/native/design/media-offline.md).
//!
//! Frames that isolate failing layers (the viewer, agent stills) draw it in
//! the layer's own box where the media is offline; exports never do. The card
//! is an SVG document drawn through the SVG image path, so it is resolved at
//! output resolution like any SVG image layer.
//!
//! It is drawn over the image, so it uses the dark-scheme design tokens in
//! both appearances (language §4, "Over the image"):
//! `colors.viewer`, `colors.danger`, `colors.text`, `colors.secondary`,
//! `colors.muted`.

use std::path::PathBuf;

const VIEWER: &str = "#0E0E0E";
const DANGER: &str = "#F36357";
const TEXT: &str = "#F2F2F2";
const SECONDARY: &str = "#B4B4B4";
const MUTED: &str = "#949494";
const WHITE: &str = "#FFFFFF";
/// SF Pro is the system font (`.SF NS`); the rest are fallbacks.
const FONT: &str = "'.SF NS', 'SF Pro', 'Helvetica Neue', Helvetica, Arial, sans-serif";

/// What fits at the box's size on screen (device pixels high).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Hatch, disc and title.
    Compact,
    /// And the file name.
    Named,
    /// And the relink hint.
    Full,
}

impl Tier {
    pub fn for_height(device_height: f64) -> Tier {
        if device_height < 120.0 {
            Tier::Compact
        } else if device_height < 260.0 {
            Tier::Named
        } else {
            Tier::Full
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `text` cut with an ellipsis to about `chars` characters (SVG text cannot
/// truncate itself; the estimate uses SF Pro's average advance).
fn fit(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let keep: String = text.chars().take(chars.saturating_sub(1)).collect();
    format!("{}…", keep.trim_end())
}

/// The card for `file` in a `w` × `h` layer box.
pub fn svg(file: &str, w: f64, h: f64, tier: Tier) -> String {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let s = w.min(h);
    // Title ≈ 8 % of the width, at most 14 % of the height.
    let title = (w * 0.08).min(h * 0.14).max(1.0);
    let name = title * 0.5;
    let hint = title * 0.42;
    let disc = title * 1.1;
    let gap = title * 0.32;
    // Average SF Pro advance ≈ 0.56 em.
    let name_chars = ((w * 0.78) / (name * 0.56)).floor().max(4.0) as usize;
    let mut lines: Vec<(f64, String)> = vec![(title, "Media Offline".into())];
    if tier != Tier::Compact {
        lines.push((name, fit(file, name_chars)));
    }
    if tier == Tier::Full {
        lines.push((hint, "Locate File… in the Project panel".into()));
    }
    let text_height: f64 = lines.iter().map(|(size, _)| size * 1.25).sum();
    let content = disc + gap + text_height;
    let pad = title * 0.7;
    let plate_w = (title * 7.6 + pad * 2.0)
        .max(name_chars.min(file.chars().count()) as f64 * name * 0.56 + pad * 2.0)
        .min(w * 0.9);
    let plate_h = (content + pad * 2.0).min(h * 0.9);
    let (px, py) = ((w - plate_w) / 2.0, (h - plate_h) / 2.0);
    let radius = (plate_h * 0.08).min(s * 0.04);
    let stripe = s * 0.035;
    let period = s * 0.09;
    let hairline = (s * 0.0015).max(0.5);
    let cx = w / 2.0;
    let mut y = py + pad;
    let mut body = String::new();
    // The disc: `danger` with a white exclamation mark.
    let r = disc / 2.0;
    let dy = y + r;
    body.push_str(&format!(
        r#"<circle cx="{cx:.3}" cy="{dy:.3}" r="{r:.3}" fill="{DANGER}"/><rect x="{:.3}" y="{:.3}" width="{:.3}" height="{:.3}" rx="{:.3}" fill="{WHITE}"/><circle cx="{cx:.3}" cy="{:.3}" r="{:.3}" fill="{WHITE}"/>"#,
        cx - r * 0.09,
        dy - r * 0.55,
        r * 0.18,
        r * 0.7,
        r * 0.09,
        dy + r * 0.45,
        r * 0.11,
    ));
    y += disc + gap;
    for (i, (size, text)) in lines.iter().enumerate() {
        let (weight, fill) = match i {
            0 => (600, TEXT),
            1 => (400, SECONDARY),
            _ => (400, MUTED),
        };
        y += size * 1.25;
        body.push_str(&format!(
            r#"<text x="{cx:.3}" y="{:.3}" text-anchor="middle" font-family="{FONT}" font-size="{size:.3}" font-weight="{weight}" fill="{fill}">{}</text>"#,
            y - size * 0.3,
            escape(text),
        ));
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.3}" height="{h:.3}" viewBox="0 0 {w:.3} {h:.3}"><defs><pattern id="hatch" width="{period:.3}" height="{period:.3}" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="{stripe:.3}" height="{period:.3}" fill="{DANGER}" fill-opacity="0.18"/></pattern></defs><rect width="{w:.3}" height="{h:.3}" fill="{VIEWER}"/><rect width="{w:.3}" height="{h:.3}" fill="url(#hatch)"/><rect x="{px:.3}" y="{py:.3}" width="{plate_w:.3}" height="{plate_h:.3}" rx="{radius:.3}" fill="{VIEWER}" fill-opacity="0.94" stroke="{WHITE}" stroke-opacity="0.08" stroke-width="{hairline:.3}"/>{body}</svg>"#
    )
}

/// Cards kept on disk and in memory by this process. A card for the same
/// file, box and tier is reused without building or hashing its SVG again;
/// the least recently used one beyond this is deleted.
const MAX_CARDS: usize = 32;
/// Files left by earlier runs are removed once they are this old.
const STALE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

type Key = (String, u64, u64, Tier);
struct Cards {
    dir: PathBuf,
    next: u64,
    /// Most recently used last.
    kept: Vec<(Key, PathBuf)>,
}
static CARDS: std::sync::Mutex<Option<Cards>> = std::sync::Mutex::new(None);

/// This process's card directory (other processes, such as an export job,
/// keep their own); stale directories of earlier runs are pruned once.
fn open_cards() -> Result<Cards, String> {
    let root = std::env::temp_dir().join("powermove-offline-cards");
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE);
            if old {
                let _ = std::fs::remove_dir_all(entry.path());
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let dir = root.join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(Cards {
        dir,
        next: 0,
        kept: Vec::new(),
    })
}

/// The card's file, written once per file name, box and tier, so the SVG
/// image path can draw and cache it like any SVG file.
pub fn path(file: &str, w: f64, h: f64, tier: Tier) -> Result<PathBuf, String> {
    let key: Key = (file.to_owned(), w.to_bits(), h.to_bits(), tier);
    let mut guard = CARDS.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(open_cards()?);
    }
    let cards = guard.as_mut().expect("opened above");
    if let Some(i) = cards.kept.iter().position(|(k, p)| *k == key && p.exists()) {
        let kept = cards.kept.remove(i);
        let path = kept.1.clone();
        cards.kept.push(kept);
        return Ok(path);
    }
    let svg = svg(file, w, h, tier);
    let path = cards.dir.join(format!("{}.svg", cards.next));
    let partial = cards.dir.join(format!("{}.partial", cards.next));
    cards.next += 1;
    // Write then rename: a reader never sees a partial card.
    std::fs::write(&partial, svg.as_bytes()).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    cards.kept.retain(|(k, _)| *k != key);
    cards.kept.push((key, path.clone()));
    while cards.kept.len() > MAX_CARDS {
        let (_, old) = cards.kept.remove(0);
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// Cards this process keeps (for tests).
#[cfg(test)]
fn kept() -> usize {
    CARDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(0, |c| c.kept.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_follow_the_size_on_screen() {
        assert_eq!(Tier::for_height(60.0), Tier::Compact);
        assert_eq!(Tier::for_height(200.0), Tier::Named);
        assert_eq!(Tier::for_height(900.0), Tier::Full);
    }

    #[test]
    fn the_card_names_the_file_and_parses() {
        for tier in [Tier::Compact, Tier::Named, Tier::Full] {
            let doc = svg("cursor <rectangle> & scale up.mp4", 1920.0, 1080.0, tier);
            assert!(doc.contains("Media Offline"));
            assert_eq!(
                doc.contains("cursor &lt;rectangle&gt; &amp; scale up.mp4"),
                tier != Tier::Compact
            );
            assert_eq!(doc.contains("Locate File"), tier == Tier::Full);
            pm_vector::render::svg::SvgDocument::parse(doc.as_bytes()).expect("valid SVG");
        }
    }

    #[test]
    fn long_names_end_in_an_ellipsis() {
        let name = "LastClip-1E9EA2ED-EDDD-4115-8C8E-3FBCF23AB2EA-and-more.mp4";
        let doc = svg(name, 160.0, 120.0, Tier::Named);
        assert!(doc.contains('…'));
        assert!(!doc.contains(name));
    }

    #[test]
    fn cards_are_reused_and_bounded() {
        let a = path("a.mp4", 640.0, 360.0, Tier::Named).unwrap();
        let stamp = std::fs::metadata(&a).unwrap().modified().unwrap();
        let b = path("a.mp4", 640.0, 360.0, Tier::Named).unwrap();
        assert_eq!(a, b, "the same card is the same file");
        assert_eq!(
            std::fs::metadata(&b).unwrap().modified().unwrap(),
            stamp,
            "not rewritten"
        );
        // A scrub through many sizes keeps at most MAX_CARDS files.
        let mut paths = vec![];
        for i in 0..(MAX_CARDS + 8) {
            paths.push(path("b.mp4", 100.0 + i as f64, 60.0, Tier::Compact).unwrap());
        }
        assert!(kept() <= MAX_CARDS);
        assert!(!paths[0].exists(), "the oldest card is deleted");
        assert!(paths.last().unwrap().exists());
        let on_disk = std::fs::read_dir(paths.last().unwrap().parent().unwrap())
            .unwrap()
            .count();
        assert!(on_disk <= MAX_CARDS, "{on_disk} files");
    }

    /// Pictures of the card at thumbnail, small-viewer and full-screen sizes:
    /// `PM_CARD_PREVIEW=/tmp/cards cargo test -p pm-render offline_card -- --ignored`.
    #[test]
    #[ignore = "writes PNGs; set PM_CARD_PREVIEW"]
    fn preview_sizes() {
        let dir = std::path::PathBuf::from(std::env::var("PM_CARD_PREVIEW").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let file = "cursor rectangle scale up.mp4";
        for (name, w, h) in [
            ("thumb", 160u32, 90u32),
            ("small", 480, 270),
            ("full", 1920, 1080),
            ("square", 400, 400),
        ] {
            let tier = Tier::for_height(f64::from(h));
            let doc = svg(file, 1920.0, 1920.0 * f64::from(h) / f64::from(w), tier);
            let mut options = usvg::Options::default();
            options.fontdb_mut().load_system_fonts();
            let tree = usvg::Tree::from_str(&doc, &options).unwrap();
            let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
            let size = tree.size();
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height()),
                &mut pixmap.as_mut(),
            );
            pixmap
                .save_png(dir.join(format!("card-{name}.png")))
                .unwrap();
        }
    }
}
