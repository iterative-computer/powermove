//! The icon next to a client in Settings › Agents › Other Agent Apps.
use objc2::runtime::Bool;
use objc2::{AnyThread, MainThreadMarker, Message, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSBezierPath, NSColor, NSImage, NSImageNameActionTemplate, NSImageScaling,
    NSImageView, NSLayoutConstraint, NSLayoutGuide, NSView, NSWorkspace,
};
use objc2_foundation::{NSBundle, NSData, NSPoint, NSRect, NSSize, NSString};

const SIZE: f64 = 24.;
const GAP: f64 = 12.;

struct Mark {
    svg: &'static [u8],
    size: f64,
}

const OPENAI: Mark = Mark {
    svg: include_bytes!("../../../assets/settings-agent-icons/openai-blossom.svg"),
    // The official SVG already contains clear space around the mark.
    size: SIZE,
};
const OPENCODE: Mark = Mark {
    svg: include_bytes!("../../../assets/settings-agent-icons/opencode.svg"),
    size: 16.,
};

fn resolve<T>(
    id: &str,
    app: impl FnMut(&str) -> Option<T>,
    mark_image: impl FnOnce(&Mark) -> Option<T>,
    symbol: impl FnOnce(&str) -> Option<T>,
) -> Option<T> {
    // See the asset attribution for the product marks that require prior approval.
    let (apps, mark, fallback): (&[&str], Option<&Mark>, _) = match id {
        "claude-code" | "claude-desktop" => (&["Claude"], None, "terminal"),
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
    let workspace = NSWorkspace::sharedWorkspace();
    // This lookup is by exact app name because several clients can provide
    // an app bundle without sharing a single known bundle identifier.
    #[allow(deprecated)]
    let path = workspace.fullPathForApplication(&NSString::from_str(&format!("{name}.app")))?;
    bundle_icon(&path)
}

fn bundle_icon(path: &NSString) -> Option<Retained<NSImage>> {
    // Avoid handing a missing bundle to iconForFile:, which returns a generic icon.
    NSBundle::bundleWithPath(path)?.bundleIdentifier()?;
    let icon = NSWorkspace::sharedWorkspace().iconForFile(path);
    icon.isValid().then_some(icon)
}

fn bundled_mark(mark: &Mark) -> Option<Retained<NSImage>> {
    let logo = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(mark.svg))?;
    let size = mark.size;
    let draw = block2::RcBlock::new(move |rect: NSRect| -> Bool {
        // A neutral light tile keeps both original marks legible in either appearance.
        NSColor::colorWithWhite_alpha(0.95, 1.).setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 5., 5.).fill();
        let origin = NSPoint::new(rect.mid().x - size / 2., rect.mid().y - size / 2.);
        logo.drawInRect(NSRect::new(origin, NSSize::new(size, size)));
        Bool::YES
    });
    Some(NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(SIZE, SIZE),
        false,
        &draw,
    ))
}

fn image_view(mtm: MainThreadMarker, id: &str, app: AppIcon) -> Retained<NSImageView> {
    let image = resolve(id, app, bundled_mark, |symbol| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            None,
        )
        .or_else(|| NSImage::imageNamed(unsafe { NSImageNameActionTemplate }))
    });
    let view = NSImageView::new(mtm);
    view.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(SIZE, SIZE)));
    view.setImage(image.as_deref());
    view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    view.setContentTintColor(Some(&NSColor::labelColor()));
    view.setAccessibilityElement(false);
    view
}

pub(super) struct LeadingIcon {
    pub(super) container: Retained<NSView>,
    copy: Retained<NSLayoutGuide>,
    anchors: Option<CopyAnchors>,
}

struct CopyAnchors {
    first: Retained<NSView>,
    last: Retained<NSView>,
    top: Retained<NSLayoutConstraint>,
    bottom: Retained<NSLayoutConstraint>,
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
        let copy = NSLayoutGuide::new();
        container.addLayoutGuide(&copy);
        super::constraints(&[
            icon.leadingAnchor()
                .constraintEqualToAnchor_constant(&container.leadingAnchor(), inset),
            icon.widthAnchor().constraintEqualToConstant(SIZE),
            icon.heightAnchor().constraintEqualToConstant(SIZE),
            icon.centerYAnchor()
                .constraintEqualToAnchor(&copy.centerYAnchor()),
            copy.leadingAnchor()
                .constraintEqualToAnchor(&icon.leadingAnchor()),
            copy.widthAnchor().constraintEqualToConstant(SIZE),
            body.leadingAnchor()
                .constraintEqualToAnchor_constant(&container.leadingAnchor(), SIZE + GAP),
            body.trailingAnchor()
                .constraintEqualToAnchor(&container.trailingAnchor()),
            body.topAnchor()
                .constraintEqualToAnchor(&container.topAnchor()),
            body.bottomAnchor()
                .constraintEqualToAnchor(&container.bottomAnchor()),
        ]);
        Self {
            container,
            copy,
            anchors: None,
        }
    }

    pub(super) fn align_with(&mut self, first: &NSView, last: &NSView) {
        if self.anchors.as_ref().is_some_and(|anchors| {
            std::ptr::eq(&*anchors.first, first) && std::ptr::eq(&*anchors.last, last)
        }) {
            return;
        }
        if let Some(old) = self.anchors.take() {
            old.top.setActive(false);
            old.bottom.setActive(false);
        }
        let top = self
            .copy
            .topAnchor()
            .constraintEqualToAnchor(&first.topAnchor());
        let bottom = self
            .copy
            .bottomAnchor()
            .constraintEqualToAnchor(&last.bottomAnchor());
        top.setActive(true);
        bottom.setActive(true);
        self.anchors = Some(CopyAnchors {
            first: first.retain(),
            last: last.retain(),
            top,
            bottom,
        });
    }
}

#[cfg(debug_assertions)]
pub(super) fn capture_app_icon(name: &str) -> Option<Retained<NSImage>> {
    // Known system bundles provide app-shaped stand-ins for the capture.
    let path = match name {
        "Claude" => "/System/Applications/Books.app",
        "Codex" => "/System/Applications/Shortcuts.app",
        "Gemini" => "/System/Applications/Maps.app",
        "OpenCode" => "/System/Applications/Utilities/Terminal.app",
        _ => return None,
    };
    bundle_icon(&NSString::from_str(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: [(&str, &str); 5] = [
        ("claude-code", "Claude"),
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
        for (id, bundle) in [
            ("claude-code", "Claude"),
            ("claude-desktop", "Claude"),
            ("gemini-cli", "Gemini"),
        ] {
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
