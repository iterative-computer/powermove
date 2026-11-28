//! The icon next to a client in Settings › Agents › Other Agent Apps.
use objc2::runtime::Bool;
use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSBezierPath, NSColor, NSImage, NSImageScaling, NSImageView, NSView,
    NSWorkspace,
};
use objc2_foundation::{NSBundle, NSData, NSPoint, NSRect, NSSize, NSString};

/// Every mark sits in one 24 pt squircle (a row is 44 pt, as on every page).
/// The mark's side and the space after it (`Settings.app_icon`, `app_icon_gap`).
pub(super) fn size() -> f64 {
    pm_shell::scale().settings.app_icon as f64
}
fn gap() -> f64 {
    pm_shell::scale().settings.app_icon_gap as f64
}

struct Mark {
    svg: &'static [u8],
    scale: f64,
}

const OPENAI: Mark = Mark {
    svg: include_bytes!("../../../assets/settings-agent-icons/openai-blossom.svg"),
    // The official SVG already contains clear space around the mark.
    scale: 1.,
};
const CLAUDE: Mark = Mark {
    svg: include_bytes!("../../../assets/settings-agent-icons/claude-ai-icon.svg"),
    scale: 2. / 3.,
};
const OPENCODE: Mark = Mark {
    svg: include_bytes!("../../../assets/settings-agent-icons/opencode.svg"),
    scale: 2. / 3.,
};

fn resolve<T>(
    id: &str,
    app: impl FnMut(&str) -> Option<T>,
    mark_image: impl FnOnce(&Mark) -> Option<T>,
    symbol: impl FnOnce(&str) -> Option<T>,
) -> Option<T> {
    // See the asset attribution for the product marks that require prior approval.
    let (apps, mark, fallback): (&[&str], Option<&Mark>, _) = match id {
        // Claude Code is a command-line tool: the Claude logo, not the app's icon.
        "claude-code" => (&[], Some(&CLAUDE), "terminal"),
        "claude-desktop" => (&["Claude"], Some(&CLAUDE), "terminal"),
        "codex" => (&["Codex", "ChatGPT"], Some(&OPENAI), "terminal"),
        "gemini-cli" => (&["Gemini"], None, "terminal"),
        "opencode" => (&["OpenCode"], Some(&OPENCODE), "terminal"),
        "cursor" => (&["Cursor"], None, "desktopcomputer"),
        "vscode" => (&["Visual Studio Code"], None, "desktopcomputer"),
        "windsurf" => (&["Windsurf"], None, "desktopcomputer"),
        "lm-studio" => (&["LM Studio"], None, "desktopcomputer"),
        _ => (&[], None, "puzzlepiece.extension"),
    };
    apps.iter()
        .copied()
        .find_map(app)
        .or_else(|| mark.and_then(mark_image))
        .or_else(|| symbol(fallback))
}

pub(super) type AppIcon = fn(&str) -> Option<Retained<NSImage>>;

pub(super) fn installed_app_icon(name: &str) -> Option<Retained<NSImage>> {
    use std::{cell::RefCell, collections::HashMap};
    thread_local! {
        static CACHE: RefCell<HashMap<String, Option<Retained<NSImage>>>> = RefCell::new(HashMap::new());
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(name).cloned()) {
        return hit;
    }
    let workspace = NSWorkspace::sharedWorkspace();
    // This lookup is by exact app name because several clients can provide
    // an app bundle without sharing a single known bundle identifier.
    #[allow(deprecated)]
    let found = workspace
        .fullPathForApplication(&NSString::from_str(&format!("{name}.app")))
        .and_then(|path| bundle_icon(&path));
    CACHE.with(|c| c.borrow_mut().insert(name.to_owned(), found.clone()));
    found
}

fn bundle_icon(path: &NSString) -> Option<Retained<NSImage>> {
    // Avoid handing a missing bundle to iconForFile:, which returns a generic icon.
    NSBundle::bundleWithPath(path)?.bundleIdentifier()?;
    let icon = NSWorkspace::sharedWorkspace().iconForFile(path);
    icon.isValid().then(|| app_squircle(icon))
}

/// A macOS app icon drawn so its squircle (824 of its 1024-point canvas)
/// fills the slot, the same size as the other marks' frames.
fn app_squircle(icon: Retained<NSImage>) -> Retained<NSImage> {
    let draw = block2::RcBlock::new(move |rect: NSRect| -> Bool {
        let grow = 1024. / 824.;
        let size = NSSize::new(rect.size.width * grow, rect.size.height * grow);
        let origin = NSPoint::new(
            rect.mid().x - size.width / 2.,
            rect.mid().y - size.height / 2.,
        );
        icon.drawInRect(NSRect::new(origin, size));
        Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(size(), size()), false, &draw)
}

/// An app with no mark of its own: Settings' lit tile with a glyph standing
/// for its kind, in the appearance it is drawn in.
fn lit_tile(symbol: &str) -> Option<Retained<NSImage>> {
    let glyph = match symbol {
        "terminal" => "terminal",
        "desktopcomputer" => "window",
        _ => "extensions",
    };
    let light = pm_shell::settings_window::tile_image(glyph, false, size())?;
    let dark = pm_shell::settings_window::tile_image(glyph, true, size())?;
    let draw = block2::RcBlock::new(move |rect: NSRect| -> Bool {
        let names = unsafe {
            [
                objc2_app_kit::NSAppearanceNameAqua,
                objc2_app_kit::NSAppearanceNameDarkAqua,
            ]
        };
        let dark_now = objc2_app_kit::NSAppearance::currentDrawingAppearance()
            .bestMatchFromAppearancesWithNames(&objc2_foundation::NSArray::from_slice(&names))
            .is_some_and(|name| &*name == names[1]);
        let image = if dark_now { &dark } else { &light };
        // The tile image carries room for its shadow around the tile.
        let size = image.size();
        let origin = NSPoint::new(
            rect.mid().x - size.width / 2.,
            rect.mid().y - size.height / 2.,
        );
        image.drawInRect(NSRect::new(origin, size));
        Bool::YES
    });
    Some(NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(size(), size()),
        false,
        &draw,
    ))
}

fn bundled_mark(mark: &Mark) -> Option<Retained<NSImage>> {
    let logo = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(mark.svg))?;
    let scale = mark.scale;
    let draw = block2::RcBlock::new(move |rect: NSRect| -> Bool {
        // A neutral light tile keeps both original marks legible in either
        // appearance; its corner is the lit tiles' (6 of 22 points).
        NSColor::colorWithWhite_alpha(0.95, 1.).setFill();
        let tile_size = rect.size.width.min(rect.size.height);
        let radius = tile_size * (6. / 22.);
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
        let size = tile_size * scale;
        let origin = NSPoint::new(rect.mid().x - size / 2., rect.mid().y - size / 2.);
        logo.drawInRect(NSRect::new(origin, NSSize::new(size, size)));
        Bool::YES
    });
    Some(NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(size(), size()),
        false,
        &draw,
    ))
}

fn image_view(mtm: MainThreadMarker, id: &str, app: AppIcon) -> Retained<NSImageView> {
    let image = resolve(id, app, bundled_mark, lit_tile);
    let view = NSImageView::new(mtm);
    view.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(size(), size())));
    view.setImage(image.as_deref());
    view.setImageScaling(NSImageScaling::ScaleNone);
    view.setAccessibilityElement(false);
    view
}

pub(super) struct LeadingIcon {
    pub(super) container: Retained<NSView>,
}

impl LeadingIcon {
    pub(super) fn new(
        mtm: MainThreadMarker,
        id: &str,
        app: AppIcon,
        body: &NSView,
        inset: f64,
    ) -> Self {
        let container = NSView::new(mtm);
        container.setTranslatesAutoresizingMaskIntoConstraints(false);
        let icon = NSView::new(mtm);
        icon.setTranslatesAutoresizingMaskIntoConstraints(false);
        // SF Symbol alignment insets vary. The containing view fixes the slot.
        icon.addSubview(&image_view(mtm, id, app));
        container.addSubview(&icon);
        container.addSubview(body);
        super::constraints(&[
            icon.leadingAnchor()
                .constraintEqualToAnchor_constant(&container.leadingAnchor(), inset),
            icon.widthAnchor().constraintEqualToConstant(size()),
            icon.heightAnchor().constraintEqualToConstant(size()),
            icon.centerYAnchor()
                .constraintEqualToAnchor(&container.centerYAnchor()),
            body.leadingAnchor()
                .constraintEqualToAnchor_constant(&container.leadingAnchor(), size() + gap()),
            body.trailingAnchor()
                .constraintEqualToAnchor(&container.trailingAnchor()),
            body.topAnchor()
                .constraintEqualToAnchor(&container.topAnchor()),
            body.bottomAnchor()
                .constraintEqualToAnchor(&container.bottomAnchor()),
        ]);
        Self { container }
    }
}

#[cfg(debug_assertions)]
pub(super) fn capture_app_icon(name: &str) -> Option<Retained<NSImage>> {
    // Known system bundles provide app-shaped stand-ins for the capture.
    let path = match name {
        "Claude" => "/System/Applications/Books.app",
        "Codex" => "/System/Applications/Calculator.app",
        "Gemini" => "/System/Applications/Maps.app",
        "OpenCode" => "/System/Applications/Utilities/Terminal.app",
        _ => return None,
    };
    bundle_icon(&NSString::from_str(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: [(&str, &str); 4] = [
        ("codex", "Codex"),
        ("claude-desktop", "Claude"),
        ("gemini-cli", "Gemini"),
        ("opencode", "OpenCode"),
    ];

    #[test]
    fn installed_app_icon_wins_for_each_known_client() {
        for (id, bundle) in CLIENTS {
            let image = resolve(
                id,
                |name| {
                    assert_eq!(name, bundle);
                    Some("installed")
                },
                |_| panic!("an installed app already supplied its icon"),
                |_| panic!("an installed app already supplied its icon"),
            );
            assert_eq!(image, Some("installed"), "{id}");
        }
    }

    #[test]
    fn codex_uses_chatgpt_when_codex_is_missing() {
        let mut lookups = Vec::new();
        let image = resolve(
            "codex",
            |name| {
                lookups.push(name.to_owned());
                (name == "ChatGPT").then_some("ChatGPT")
            },
            |_| panic!("ChatGPT already supplied its icon"),
            |_| panic!("ChatGPT already supplied its icon"),
        );
        assert_eq!(image, Some("ChatGPT"));
        assert_eq!(lookups, ["Codex", "ChatGPT"]);
    }

    #[test]
    fn missing_apps_prefer_their_official_mark_when_available() {
        for (id, apps, expected) in [
            ("codex", &["Codex", "ChatGPT"][..], OPENAI.svg),
            ("opencode", &["OpenCode"][..], OPENCODE.svg),
            ("claude-code", &[][..], CLAUDE.svg),
            ("claude-desktop", &["Claude"][..], CLAUDE.svg),
        ] {
            let mut lookups = Vec::new();
            let image = resolve(
                id,
                |name| {
                    lookups.push(name.to_owned());
                    None
                },
                |mark| Some(mark.svg),
                |_| panic!("the official mark should come before the symbol"),
            );
            assert!(image.is_some_and(|image| image == expected), "{id}");
            assert_eq!(lookups, apps, "{id}");
        }
    }

    #[test]
    fn clients_without_a_bundled_mark_use_a_neutral_symbol() {
        for (id, bundle) in [("gemini-cli", "Gemini")] {
            let image = resolve(
                id,
                |name| {
                    assert_eq!(name, bundle);
                    None
                },
                |_| panic!("no official mark is bundled"),
                |symbol| {
                    assert_eq!(symbol, "terminal");
                    Some(symbol.to_owned())
                },
            );
            assert_eq!(image.as_deref(), Some("terminal"), "{id}");
        }
    }

    #[test]
    fn an_unreadable_mark_uses_the_neutral_symbol() {
        let image = resolve(
            "opencode",
            |_| None,
            |_| None,
            |symbol| Some(symbol.to_owned()),
        );
        assert_eq!(image.as_deref(), Some("terminal"));
    }

    #[test]
    fn any_other_agent_always_uses_the_puzzle_piece() {
        let image = resolve(
            "other",
            |_| panic!("the generic row has no app"),
            |_| panic!("the generic row has no vendor mark"),
            |symbol| {
                assert_eq!(symbol, "puzzlepiece.extension");
                Some("puzzle piece")
            },
        );
        assert_eq!(image, Some("puzzle piece"));
    }
}
