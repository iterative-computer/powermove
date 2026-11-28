//! Native macOS What's New sheet (`Powermove ▸ What’s New…` and post-update).
//!
//! Two layouts from the same sheet recipe:
//! - **With cover image**: full-bleed hero cover at the top with a smooth
//!   multi-stop gradient melting directly into the sheet surface, a floating
//!   version pill at top-right, and the title block pulled gently into the fade.
//! - **Without cover image**: compact header with the Powermove App Icon tile,
//!   title + subtitle, and a subtle version pill on the right.
//!
//! Both share the grouped highlights card (monochrome SF Symbol tiles, title +
//! subtitle, inset hairline dividers) and the quiet footer (`Release Notes ↗`
//! on the left, accent `Continue` push button on the right).

use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSBackingStoreType,
    NSBezelStyle, NSBezierPath, NSButton, NSColor, NSColorSpace, NSControlSize, NSFont,
    NSFontAttributeName, NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSGradient, NSGraphicsContext, NSImage, NSImageScaling,
    NSImageSymbolConfiguration, NSImageSymbolScale, NSImageView, NSModalResponse,
    NSModalResponseOK, NSStringDrawing, NSStringDrawingOptions, NSStringNSExtendedStringDrawing,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString,
};
use pm_design::{Colors, Ink, Scheme, Tokens};
use pm_services::surface_backing::maintenance::Release;
use std::cell::RefCell;

const SHEET_WIDTH: f64 = 456.;
const HERO_HEIGHT: f64 = 228.;
const HERO_OVERLAP: f64 = 52.;
const SIDE_INSET: f64 = 20.;
const TITLE_INSET: f64 = 10.;
const CARD_RADIUS: f64 = 12.;
const ROW_PAD_X: f64 = 12.;
const ROW_PAD_Y: f64 = 10.;
const ICON_BOX: f64 = 26.;
const ICON_GAP: f64 = 11.;
const APP_TILE: f64 = 34.;
const FOOTER_HEIGHT: f64 = 28.;

const V2_HERO_JPEG: &[u8] = include_bytes!("../assets/whats-new-hero.jpg");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub symbol: String,
    pub title: String,
    pub subtitle: String,
}

#[derive(Clone, Debug)]
pub struct WhatsNewContent {
    pub version_badge: String,
    pub title: String,
    pub summary: String,
    pub cover_image: Option<Vec<u8>>,
    pub highlights: Vec<Highlight>,
    pub releases_url: String,
}

impl WhatsNewContent {
    /// Built-in Powermove 2.0 release log (used when opened from the menu
    /// without a local release feed fixture, or when a v2.0 release has no
    /// structured highlights).
    pub fn v2_default(version: &str, releases_url: &str) -> Self {
        let badge = format_version_badge(version, true);
        Self {
            version_badge: badge,
            title: "Powermove, rewritten in Rust".into(),
            summary: "Rebuilt natively on Rust, Metal, and GPUI — without Electron.".into(),
            cover_image: Some(V2_HERO_JPEG.to_vec()),
            highlights: vec![
                Highlight {
                    symbol: "gauge.with.needle".into(),
                    title: "Significantly better performance".into(),
                    subtitle: "Native Rust & Metal playback and lower memory use".into(),
                },
                Highlight {
                    symbol: "sparkles".into(),
                    title: "Introducing Powermove Aerolite".into(),
                    subtitle: "Built-in agent with inference included".into(),
                },
                Highlight {
                    symbol: "macwindow".into(),
                    title: "Refined user interface".into(),
                    subtitle: "Native macOS controls, typography, and motion".into(),
                },
            ],
            releases_url: releases_url.into(),
        }
    }

    /// Builds sheet content from one or more GitHub-style `Release` objects.
    pub fn from_releases(releases: &[Release], current: &str, releases_url: &str) -> Self {
        let Some(latest) = releases.first() else {
            return Self::v2_default(current, releases_url);
        };
        let meta = &latest.metadata;
        let body = meta["body"].as_str().unwrap_or("");
        let cover_image = meta["cover_image"]
            .as_str()
            .and_then(|path| std::fs::read(path).ok())
            .or_else(|| {
                let tag = latest.version.trim_start_matches('v');
                (tag == "2.0" || tag == "2.0.0").then(|| V2_HERO_JPEG.to_vec())
            });
        let has_image = cover_image.is_some();
        let version_badge = format_version_badge(
            if latest.version.is_empty() {
                current
            } else {
                &latest.version
            },
            has_image,
        );
        let title = meta["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "What’s New in Powermove".into());

        let (parsed_summary, parsed_highlights) = parse_release_body(body);
        let summary = meta["summary"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or(parsed_summary)
            .unwrap_or_else(|| "Fixes and performance improvements".into());

        let highlights = meta["highlights"]
            .as_array()
            .and_then(|arr| {
                let list: Vec<Highlight> = arr
                    .iter()
                    .filter_map(|item| {
                        let title = item["title"].as_str()?.trim().to_owned();
                        if title.is_empty() {
                            return None;
                        }
                        let subtitle = item["subtitle"]
                            .as_str()
                            .map(str::trim)
                            .unwrap_or("")
                            .to_owned();
                        let symbol = item["symbol"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .unwrap_or_else(|| infer_symbol(&title, &subtitle));
                        Some(Highlight {
                            symbol,
                            title,
                            subtitle,
                        })
                    })
                    .collect();
                (!list.is_empty()).then_some(list)
            })
            .unwrap_or_else(|| {
                if !parsed_highlights.is_empty() {
                    parsed_highlights
                } else {
                    vec![Highlight {
                        symbol: "checkmark.circle".into(),
                        title: "Fixes and improvements".into(),
                        subtitle: "Updated to the latest release of Powermove".into(),
                    }]
                }
            });

        let url = meta["html_url"]
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .unwrap_or(releases_url)
            .to_owned();

        Self {
            version_badge,
            title,
            summary,
            cover_image,
            highlights,
            releases_url: url,
        }
    }
}

fn format_version_badge(version: &str, long: bool) -> String {
    let trimmed = version.trim().trim_start_matches('v');
    let display = if trimmed.is_empty() {
        "2.0"
    } else if let Some(short) = trimmed.strip_suffix(".0") {
        // Keep "2.0" instead of stripping to "2", strip "2.0.0" -> "2.0".
        if short.contains('.') { short } else { trimmed }
    } else {
        trimmed
    };
    if long {
        format!("Version {display}")
    } else {
        format!("v{display}")
    }
}

/// Parses an optional lead summary paragraph and up to 5 bullet highlights
/// from standard Markdown release notes (`- **Title** — Subtitle`, `- Title:
/// Subtitle`, or plain `- Title`).
pub(crate) fn parse_release_body(body: &str) -> (Option<String>, Vec<Highlight>) {
    let mut summary = None;
    let mut highlights = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("![") {
            continue;
        }
        let bullet = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("• "));
        if let Some(item) = bullet {
            if highlights.len() >= 5 {
                continue;
            }
            let (title, subtitle) = split_highlight(item.trim());
            if !title.is_empty() {
                let symbol = infer_symbol(&title, &subtitle);
                highlights.push(Highlight {
                    symbol,
                    title,
                    subtitle,
                });
            }
        } else if summary.is_none() && highlights.is_empty() {
            summary = Some(strip_markdown_inline(line));
        }
    }
    (summary, highlights)
}

fn split_highlight(item: &str) -> (String, String) {
    // Handle `**Title** — Subtitle` or `**Title**: Subtitle`.
    if let Some(rest) = item.strip_prefix("**")
        && let Some((bold, tail)) = rest.split_once("**")
    {
        let title = bold.trim().to_owned();
        let subtitle = tail
            .trim()
            .trim_start_matches([':', '—', '–', '-'])
            .trim();
        return (title, strip_markdown_inline(subtitle));
    }
    for sep in [" — ", " – ", ": ", " - "] {
        if let Some((left, right)) = item.split_once(sep) {
            return (
                strip_markdown_inline(left.trim()),
                strip_markdown_inline(right.trim()),
            );
        }
    }
    (strip_markdown_inline(item), String::new())
}

fn strip_markdown_inline(s: &str) -> String {
    s.replace("**", "").replace('`', "").trim().to_owned()
}

fn infer_symbol(title: &str, subtitle: &str) -> String {
    let hay = format!("{title} {subtitle}").to_ascii_lowercase();
    if hay.contains("aerolite") || hay.contains("agent") || hay.contains("ai ") {
        "sparkles"
    } else if hay.contains("performance") || hay.contains("playback") || hay.contains("memory") {
        "gauge.with.needle"
    } else if hay.contains("export") || hay.contains("render") {
        "arrow.up.circle"
    } else if hay.contains("shader") || hay.contains("faster") || hay.contains("speed") {
        "bolt"
    } else if hay.contains("3d") || hay.contains("mesh") || hay.contains("camera") {
        "cube"
    } else if hay.contains("interface")
        || hay.contains("macos")
        || hay.contains("window")
        || hay.contains("sidebar")
        || hay.contains("ui")
    {
        "macwindow"
    } else if hay.contains("audio") || hay.contains("speech") || hay.contains("caption") {
        "waveform"
    } else {
        "checkmark.circle"
    }
    .to_owned()
}

fn srgb(rgb: u32, alpha: f64) -> Retained<NSColor> {
    let r = ((rgb >> 16) & 0xff) as f64 / 255.;
    let g = ((rgb >> 8) & 0xff) as f64 / 255.;
    let b = (rgb & 0xff) as f64 / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha)
}

fn attributes(
    font: &NSFont,
    ink: &NSColor,
) -> Retained<NSDictionary<NSString, AnyObject>> {
    NSDictionary::<NSString, AnyObject>::from_slices(
        &[unsafe { NSFontAttributeName }, unsafe {
            NSForegroundColorAttributeName
        }],
        &[font.as_ref(), ink.as_ref()],
    )
}

fn wrapped_height(text: &str, font: &NSFont, width: f64) -> f64 {
    if text.is_empty() {
        return 0.;
    }
    let attrs = attributes(font, &NSColor::labelColor());
    let rect = unsafe {
        NSString::from_str(text).boundingRectWithSize_options_attributes_context(
            NSSize::new(width.max(1.), f64::MAX),
            NSStringDrawingOptions::UsesLineFragmentOrigin
                | NSStringDrawingOptions::UsesFontLeading,
            Some(&attrs),
            None,
        )
    };
    rect.size.height.ceil()
}

fn line_width(text: &str, font: &NSFont) -> f64 {
    let attrs = attributes(font, &NSColor::labelColor());
    unsafe { NSString::from_str(text).sizeWithAttributes(Some(&attrs)) }
        .width
        .ceil()
}

fn line_height(font: &NSFont) -> f64 {
    let attrs = attributes(font, &NSColor::labelColor());
    unsafe { NSString::from_str("Ag").sizeWithAttributes(Some(&attrs)) }
        .height
        .ceil()
}

fn draw_text(text: &str, font: &NSFont, ink: &NSColor, rect: NSRect) {
    if text.is_empty() {
        return;
    }
    let attrs = attributes(font, ink);
    unsafe {
        NSString::from_str(text).drawWithRect_options_attributes_context(
            rect,
            NSStringDrawingOptions::UsesLineFragmentOrigin
                | NSStringDrawingOptions::UsesFontLeading
                | NSStringDrawingOptions::TruncatesLastVisibleLine,
            Some(&attrs),
            None,
        );
    }
}

struct RowMetrics {
    y: f64,
    height: f64,
    title_h: f64,
    sub_h: f64,
}

struct SheetLayout {
    has_image: bool,
    header_y: f64,
    title_h: f64,
    summary_h: f64,
    card_y: f64,
    card_h: f64,
    rows: Vec<RowMetrics>,
    footer_y: f64,
    total_h: f64,
}

struct Fonts {
    kicker: Retained<NSFont>,
    title_hero: Retained<NSFont>,
    title_compact: Retained<NSFont>,
    summary: Retained<NSFont>,
    badge: Retained<NSFont>,
    row_title: Retained<NSFont>,
    row_sub: Retained<NSFont>,
}

impl Fonts {
    fn new(t: &Tokens) -> Self {
        let ts = t.text_size;
        Self {
            kicker: NSFont::systemFontOfSize_weight(ts.xs as f64, unsafe { NSFontWeightMedium }),
            title_hero: NSFont::systemFontOfSize_weight((ts.x2l - 1.) as f64, unsafe {
                NSFontWeightSemibold
            }),
            title_compact: NSFont::systemFontOfSize_weight((ts.xl - 1.) as f64, unsafe {
                NSFontWeightSemibold
            }),
            summary: NSFont::systemFontOfSize_weight(ts.sm as f64, unsafe { NSFontWeightRegular }),
            badge: NSFont::monospacedDigitSystemFontOfSize_weight(ts.xs as f64, unsafe {
                NSFontWeightMedium
            }),
            row_title: NSFont::systemFontOfSize_weight(ts.sm as f64, unsafe {
                NSFontWeightMedium
            }),
            row_sub: NSFont::systemFontOfSize_weight(ts.xs as f64, unsafe { NSFontWeightRegular }),
        }
    }
}

fn measure(content: &WhatsNewContent, fonts: &Fonts, has_image: bool) -> SheetLayout {
    let card_w = SHEET_WIDTH - 2. * SIDE_INSET;
    let copy_w = card_w - 2. * ROW_PAD_X - ICON_BOX - ICON_GAP;
    let y;
    let header_y;
    let title_h;
    let summary_h;
    if has_image {
        header_y = HERO_HEIGHT - HERO_OVERLAP;
        let text_w = card_w - 2. * TITLE_INSET;
        let kicker_h = line_height(&fonts.kicker);
        title_h = wrapped_height(&content.title, &fonts.title_hero, text_w);
        summary_h = wrapped_height(&content.summary, &fonts.summary, text_w);
        y = header_y + kicker_h + 4. + title_h + 6. + summary_h + 14.;
    } else {
        header_y = 22.;
        let badge_w = line_width(&content.version_badge, &fonts.badge) + 16.;
        let text_w = card_w - 2. * TITLE_INSET - APP_TILE - 10. - badge_w - 12.;
        title_h = wrapped_height(&content.title, &fonts.title_compact, text_w);
        summary_h = wrapped_height(&content.summary, &fonts.kicker, text_w);
        let head_h = APP_TILE.max(title_h + 2. + summary_h);
        y = header_y + head_h + 14.;
    }
    let card_y = y;
    let mut rows = Vec::with_capacity(content.highlights.len());
    let mut row_y = 0.;
    for item in &content.highlights {
        let th = wrapped_height(&item.title, &fonts.row_title, copy_w);
        let sh = if item.subtitle.is_empty() {
            0.
        } else {
            wrapped_height(&item.subtitle, &fonts.row_sub, copy_w)
        };
        let copy_h = th + if sh > 0. { 2. + sh } else { 0. };
        let row_h = (copy_h.max(ICON_BOX) + 2. * ROW_PAD_Y).ceil();
        rows.push(RowMetrics {
            y: row_y,
            height: row_h,
            title_h: th,
            sub_h: sh,
        });
        row_y += row_h;
    }
    let card_h = row_y;
    let footer_y = card_y + card_h + 16.;
    let total_h = (footer_y + FOOTER_HEIGHT + SIDE_INSET).ceil();
    SheetLayout {
        has_image,
        header_y,
        title_h,
        summary_h,
        card_y,
        card_h,
        rows,
        footer_y,
        total_h,
    }
}

struct CanvasState {
    content: WhatsNewContent,
    tokens: Tokens,
    cover: Option<Retained<NSImage>>,
    app_icon: Option<Retained<NSImage>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMWhatsNewCanvas"]
    #[ivars = CanvasState]
    struct Canvas;

    impl Canvas {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let state = self.ivars();
            let t = &state.tokens;
            let dark = t.scheme == Scheme::Dark;
            let c = &t.colors;
            let ink = Ink::new();
            let bounds = self.bounds();
            let fonts = Fonts::new(t);
            let has_image = state.cover.is_some();
            let layout = measure(&state.content, &fonts, has_image);

            // Sheet ground.
            let sheet_bg = srgb(c.window, 1.);
            sheet_bg.setFill();
            NSBezierPath::fillRect(bounds);

            let card_x = SIDE_INSET;
            let card_w = bounds.size.width - 2. * SIDE_INSET;

            // Variant A: Full-bleed hero image + smooth multi-stop fade + floating version pill.
            if let Some(cover) = &state.cover {
                let hero_rect = NSRect::new(NSPoint::ZERO, NSSize::new(bounds.size.width, HERO_HEIGHT));
                NSGraphicsContext::saveGraphicsState_class();
                NSBezierPath::clipRect(hero_rect);
                cover.drawInRect(hero_rect);
                NSGraphicsContext::restoreGraphicsState_class();

                let stops = NSArray::from_retained_slice(&[
                    srgb(c.window, 0.00),
                    srgb(c.window, 0.04),
                    srgb(c.window, 0.36),
                    srgb(c.window, 0.82),
                    srgb(c.window, 1.00),
                ]);
                let locations = [0.0_f64, 0.36, 0.58, 0.80, 1.0];
                if let Some(grad) = unsafe {
                    NSGradient::initWithColors_atLocations_colorSpace(
                        NSGradient::alloc(),
                        &stops,
                        locations.as_ptr(),
                        &NSColorSpace::sRGBColorSpace(),
                    )
                } {
                    grad.drawInRect_angle(hero_rect, 90.);
                }

                // Floating version pill at top-right.
                let badge = &state.content.version_badge;
                let bw = line_width(badge, &fonts.badge) + 16.;
                let bh = line_height(&fonts.badge) + 6.;
                let pill_rect = NSRect::new(
                    NSPoint::new(bounds.size.width - 14. - bw, 14.),
                    NSSize::new(bw, bh),
                );
                let pill = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill_rect, 6., 6.);
                let dark_c = Colors::of(Scheme::Dark);
                srgb(dark_c.window, 0.62).setFill();
                pill.fill();
                srgb(dark_c.text, 0.16).setStroke();
                pill.setLineWidth(0.5);
                pill.stroke();
                draw_text(
                    badge,
                    &fonts.badge,
                    &srgb(dark_c.text, 0.90),
                    NSRect::new(
                        NSPoint::new(pill_rect.origin.x + 8., pill_rect.origin.y + 3.),
                        NSSize::new(bw - 16., bh - 6.),
                    ),
                );

                // Title block pulled gently into the fade zone.
                let tx = card_x + TITLE_INSET;
                let tw = card_w - 2. * TITLE_INSET;
                let mut y = layout.header_y;
                let kicker_h = line_height(&fonts.kicker);
                draw_text(
                    "What’s New",
                    &fonts.kicker,
                    &srgb(c.muted, 1.),
                    NSRect::new(NSPoint::new(tx, y), NSSize::new(tw, kicker_h)),
                );
                y += kicker_h + 4.;
                draw_text(
                    &state.content.title,
                    &fonts.title_hero,
                    &srgb(c.text, 1.),
                    NSRect::new(NSPoint::new(tx, y), NSSize::new(tw, layout.title_h)),
                );
                y += layout.title_h + 6.;
                draw_text(
                    &state.content.summary,
                    &fonts.summary,
                    &srgb(c.secondary, 1.),
                    NSRect::new(NSPoint::new(tx, y), NSSize::new(tw, layout.summary_h)),
                );
            } else {
                // Variant B: Compact header with Powermove App Icon tile + version pill.
                let hx = card_x + TITLE_INSET;
                let hy = layout.header_y;
                let icon_rect = NSRect::new(NSPoint::new(hx, hy), NSSize::new(APP_TILE, APP_TILE));
                if let Some(icon) = &state.app_icon {
                    icon.drawInRect(icon_rect);
                }
                let badge = &state.content.version_badge;
                let bw = line_width(badge, &fonts.badge) + 16.;
                let bh = line_height(&fonts.badge) + 6.;
                let pill_rect = NSRect::new(
                    NSPoint::new(card_x + card_w - TITLE_INSET - bw, hy + (APP_TILE - bh) / 2.),
                    NSSize::new(bw, bh),
                );
                let pill = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill_rect, 6., 6.);
                srgb(c.panel, 1.).setFill();
                pill.fill();
                srgb(c.text, ink.edge as f64).setStroke();
                pill.setLineWidth(0.5);
                pill.stroke();
                draw_text(
                    badge,
                    &fonts.badge,
                    &srgb(c.muted, 1.),
                    NSRect::new(
                        NSPoint::new(pill_rect.origin.x + 8., pill_rect.origin.y + 3.),
                        NSSize::new(bw - 16., bh - 6.),
                    ),
                );

                let tx = hx + APP_TILE + 10.;
                let tw = pill_rect.origin.x - 12. - tx;
                let text_block_h = layout.title_h + 2. + layout.summary_h;
                let ty = hy + ((APP_TILE - text_block_h) / 2.).max(0.);
                draw_text(
                    &state.content.title,
                    &fonts.title_compact,
                    &srgb(c.text, 1.),
                    NSRect::new(NSPoint::new(tx, ty), NSSize::new(tw, layout.title_h)),
                );
                draw_text(
                    &state.content.summary,
                    &fonts.kicker,
                    &srgb(c.muted, 1.),
                    NSRect::new(
                        NSPoint::new(tx, ty + layout.title_h + 2.),
                        NSSize::new(tw, layout.summary_h),
                    ),
                );
            }

            // Grouped highlights card (`FormGroup`).
            let card_rect = NSRect::new(
                NSPoint::new(card_x, layout.card_y),
                NSSize::new(card_w, layout.card_h),
            );
            let card_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                card_rect,
                CARD_RADIUS,
                CARD_RADIUS,
            );
            srgb(c.panel, 1.).setFill();
            card_path.fill();
            srgb(c.text, if dark { 0.08 } else { 0.09 }).setStroke();
            card_path.setLineWidth(0.5);
            card_path.stroke();

            let copy_x = card_x + ROW_PAD_X + ICON_BOX + ICON_GAP;
            let copy_w = card_w - 2. * ROW_PAD_X - ICON_BOX - ICON_GAP;

            for (idx, (item, rm)) in state.content.highlights.iter().zip(&layout.rows).enumerate() {
                let ry = layout.card_y + rm.y;
                if idx > 0 {
                    srgb(c.text, ink.well as f64).setFill();
                    NSBezierPath::fillRect(NSRect::new(
                        NSPoint::new(copy_x, ry),
                        NSSize::new(card_x + card_w - ROW_PAD_X - copy_x, 0.5),
                    ));
                }

                // Icon tile well.
                let icon_y = ry + (rm.height - ICON_BOX) / 2.;
                let well_rect = NSRect::new(
                    NSPoint::new(card_x + ROW_PAD_X, icon_y),
                    NSSize::new(ICON_BOX, ICON_BOX),
                );
                let well = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(well_rect, 7., 7.);
                srgb(c.text, ink.well as f64).setFill();
                well.fill();
                srgb(c.text, ink.edge as f64).setStroke();
                well.setLineWidth(0.5);
                well.stroke();

                // Row copy (vertically centered in row).
                let block_h = rm.title_h + if rm.sub_h > 0. { 2. + rm.sub_h } else { 0. };
                let cy = ry + (rm.height - block_h) / 2.;
                draw_text(
                    &item.title,
                    &fonts.row_title,
                    &srgb(c.text, 1.),
                    NSRect::new(NSPoint::new(copy_x, cy), NSSize::new(copy_w, rm.title_h)),
                );
                if rm.sub_h > 0. {
                    draw_text(
                        &item.subtitle,
                        &fonts.row_sub,
                        &srgb(c.muted, 1.),
                        NSRect::new(
                            NSPoint::new(copy_x, cy + rm.title_h + 2.),
                            NSSize::new(copy_w, rm.sub_h),
                        ),
                    );
                }
            }
        }
    }
);

struct TargetState {
    sheet: Retained<NSWindow>,
    releases_url: String,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMWhatsNewTarget"]
    #[ivars = TargetState]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(dismiss:))]
        fn dismiss(&self, _: &AnyObject) {
            let sheet = &self.ivars().sheet;
            if let Some(parent) = sheet.sheetParent() {
                parent.endSheet_returnCode(sheet, NSModalResponseOK);
            } else {
                sheet.close();
            }
        }

        #[unsafe(method(openNotes:))]
        fn open_notes(&self, _: &AnyObject) {
            crate::platform::open_url(&self.ivars().releases_url);
        }
    }
);

struct ActiveSheet {
    _window: Retained<NSWindow>,
    _target: Retained<Target>,
}

thread_local! {
    static ACTIVE: RefCell<Option<ActiveSheet>> = const { RefCell::new(None) };
}

/// Presents the native What's New sheet attached to `owner`.
pub fn present(owner: &NSWindow, content: WhatsNewContent, tokens: &Tokens) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if owner.attachedSheet().is_some() || gpui_macos::child_window::sheet_pending(owner) {
        return;
    }

    let dark = tokens.scheme == Scheme::Dark;
    let cover = content.cover_image.as_deref().and_then(|bytes| {
        let data = NSData::with_bytes(bytes);
        NSImage::initWithData(NSImage::alloc(), &data)
    });
    let has_image = cover.is_some();
    let app_icon = (!has_image)
        .then(|| pm_shell::settings_window::tile_image("powermove", dark, APP_TILE))
        .flatten();

    let fonts = Fonts::new(tokens);
    let layout = measure(&content, &fonts, has_image);

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(SHEET_WIDTH, layout.total_h)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let appearance_name = unsafe {
        if dark {
            NSAppearanceNameDarkAqua
        } else {
            NSAppearanceNameAqua
        }
    };
    window.setAppearance(NSAppearance::appearanceNamed(appearance_name).as_deref());
    window.setBackgroundColor(Some(&srgb(tokens.colors.window, 1.)));
    window.setTitle(&NSString::from_str("What’s New"));

    let canvas: Retained<Canvas> = unsafe {
        msg_send![
            super(Canvas::alloc(mtm).set_ivars(CanvasState {
                content: content.clone(),
                tokens: tokens.clone(),
                cover,
                app_icon,
            })),
            initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::new(SHEET_WIDTH, layout.total_h))
        ]
    };

    // Place crisp tinted SF Symbols inside each highlight row's icon tile.
    let sym_cfg = NSImageSymbolConfiguration::configurationWithPointSize_weight_scale(
        13.5,
        unsafe { NSFontWeightRegular },
        NSImageSymbolScale::Medium,
    );
    let sym_tint = srgb(tokens.colors.secondary, 1.);
    for (item, rm) in content.highlights.iter().zip(&layout.rows) {
        let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(&item.symbol),
            None,
        )
        .or_else(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str("checkmark.circle"),
                None,
            )
        })
        .and_then(|img| img.imageWithSymbolConfiguration(&sym_cfg));
        if let Some(img) = symbol {
            let iv = NSImageView::imageViewWithImage(&img, mtm);
            iv.setContentTintColor(Some(&sym_tint));
            iv.setImageScaling(NSImageScaling::ScaleProportionallyDown);
            let icon_y = layout.card_y + rm.y + (rm.height - ICON_BOX) / 2.;
            iv.setFrame(NSRect::new(
                NSPoint::new(SIDE_INSET + ROW_PAD_X + 4., icon_y + 4.),
                NSSize::new(ICON_BOX - 8., ICON_BOX - 8.),
            ));
            canvas.addSubview(&iv);
        }
    }

    let target: Retained<Target> = unsafe {
        msg_send![
            super(Target::alloc(mtm).set_ivars(TargetState {
                sheet: window.clone(),
                releases_url: content.releases_url.clone(),
            })),
            init
        ]
    };

    // Footer left: quiet "Release Notes ↗" action.
    let notes_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Release Notes ↗"),
            Some(&target),
            Some(objc2::sel!(openNotes:)),
            mtm,
        )
    };
    notes_btn.setBordered(false);
    notes_btn.setFont(Some(&NSFont::systemFontOfSize_weight(
        tokens.text_size.sm as f64,
        unsafe { NSFontWeightRegular },
    )));
    notes_btn.setContentTintColor(Some(&srgb(tokens.colors.muted, 1.)));
    let notes_w = line_width("Release Notes ↗", &fonts.summary) + 8.;
    notes_btn.setFrame(NSRect::new(
        NSPoint::new(SIDE_INSET + TITLE_INSET - 2., layout.footer_y + 2.),
        NSSize::new(notes_w, FOOTER_HEIGHT - 4.),
    ));
    canvas.addSubview(&notes_btn);

    // Footer right: accent "Continue" default push button (Return + Escape).
    let cont_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Continue"),
            Some(&target),
            Some(objc2::sel!(dismiss:)),
            mtm,
        )
    };
    cont_btn.setBezelStyle(NSBezelStyle::Push);
    cont_btn.setControlSize(NSControlSize::Regular);
    cont_btn.setBezelColor(Some(&srgb(tokens.colors.accent, 1.)));
    cont_btn.setKeyEquivalent(&NSString::from_str("\r"));
    let cont_w = (line_width("Continue", &fonts.row_title) + 32.).max(88.);
    cont_btn.setFrame(NSRect::new(
        NSPoint::new(SHEET_WIDTH - SIDE_INSET - cont_w, layout.footer_y),
        NSSize::new(cont_w, FOOTER_HEIGHT),
    ));
    canvas.addSubview(&cont_btn);

    // Hidden Escape button so Esc closes the sheet like every macOS sheet.
    let esc_btn = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(""),
            Some(&target),
            Some(objc2::sel!(dismiss:)),
            mtm,
        )
    };
    esc_btn.setKeyEquivalent(&NSString::from_str("\u{1b}"));
    esc_btn.setTransparent(true);
    esc_btn.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(1., 1.)));
    canvas.addSubview(&esc_btn);

    window.setContentView(Some(&canvas));

    let handler = block2::RcBlock::new(move |_: NSModalResponse| {
        ACTIVE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    });

    let on: Retained<NSWindow> = objc2::Message::retain(owner);
    let shown = window.clone();
    ACTIVE.with(|slot| {
        *slot.borrow_mut() = Some(ActiveSheet {
            _window: window.clone(),
            _target: target,
        });
    });
    gpui_macos::child_window::sheet(
        owner,
        &window,
        move || on.beginSheet_completionHandler(&shown, Some(&handler)),
        || {
            ACTIVE.with(|slot| {
                *slot.borrow_mut() = None;
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{WhatsNewContent, format_version_badge, parse_release_body};
    use pm_services::surface_backing::maintenance::Release;
    use serde_json::json;

    #[test]
    fn parses_markdown_bullets_and_summary_into_highlights() {
        let body = "Rebuilt natively on Rust, Metal, and GPUI.\n\n\
            - **Significantly better performance** — Native Rust & Metal playback\n\
            - **Introducing Powermove Aerolite**: Aero, our built-in agent, with inference included\n\
            - Live export progress - Toolbar ring while rendering";
        let (summary, highlights) = parse_release_body(body);
        assert_eq!(
            summary.as_deref(),
            Some("Rebuilt natively on Rust, Metal, and GPUI.")
        );
        assert_eq!(highlights.len(), 3);
        assert_eq!(highlights[0].title, "Significantly better performance");
        assert_eq!(highlights[0].subtitle, "Native Rust & Metal playback");
        assert_eq!(highlights[0].symbol, "gauge.with.needle");
        assert_eq!(highlights[1].symbol, "sparkles");
        assert_eq!(highlights[2].symbol, "arrow.up.circle");
    }

    #[test]
    fn builds_text_only_variant_for_minor_releases_without_image() {
        let release = Release {
            version: "v2.1.0".into(),
            draft: false,
            prerelease: false,
            metadata: json!({
                "tag_name": "v2.1.0",
                "name": "What’s New in Powermove",
                "body": "Fixes and performance improvements\n- **Live export progress** — Toolbar ring and percentage"
            }),
        };
        let content = WhatsNewContent::from_releases(&[release], "2.1.0", "https://example.com");
        assert!(content.cover_image.is_none());
        assert_eq!(content.version_badge, "v2.1");
        assert_eq!(content.highlights.len(), 1);
        assert_eq!(format_version_badge("2.0.0", true), "Version 2.0");
    }
}
