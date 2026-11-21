//! The icon next to a client in Settings › Agents › Other Agent Apps.
use objc2::{MainThreadMarker, Message, rc::Retained};
use objc2_app_kit::{
    NSAccessibility, NSColor, NSImage, NSImageNameActionTemplate, NSImageScaling, NSImageView,
    NSLayoutConstraint, NSLayoutGuide, NSView, NSWorkspace,
};
use objc2_foundation::{NSBundle, NSPoint, NSRect, NSSize, NSString};

const SIZE: f64 = 24.;
const GAP: f64 = 12.;

fn sources(id: &str) -> (Option<&'static str>, &'static str) {
    match id {
        "claude-code" => (Some("Claude"), "terminal"),
        "codex" => (Some("Codex"), "terminal"),
        "claude-desktop" => (Some("Claude"), "app"),
        "gemini-cli" => (Some("Gemini"), "terminal"),
        "opencode" => (Some("OpenCode"), "terminal"),
        "cursor" => (Some("Cursor"), "app"),
        "vscode" => (Some("Visual Studio Code"), "app"),
        "windsurf" => (Some("Windsurf"), "app"),
        "lm-studio" => (Some("LM Studio"), "app"),
        _ => (None, "puzzlepiece.extension"),
    }
}

// There are no bundled provider marks. Look up the app before using an SF Symbol.
fn resolve<T>(
    id: &str,
    app: impl FnOnce(&str) -> Option<T>,
    symbol: impl FnOnce(&str) -> Option<T>,
) -> Option<T> {
    let (bundle, fallback) = sources(id);
    bundle.and_then(app).or_else(|| symbol(fallback))
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

fn image_view(mtm: MainThreadMarker, id: &str, app: AppIcon) -> Retained<NSImageView> {
    let image = resolve(id, app, |symbol| {
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

    const CLIENTS: [(&str, &str, &str); 5] = [
        ("claude-code", "Claude", "terminal"),
        ("codex", "Codex", "terminal"),
        ("claude-desktop", "Claude", "app"),
        ("gemini-cli", "Gemini", "terminal"),
        ("opencode", "OpenCode", "terminal"),
    ];

    #[test]
    fn installed_app_icon_wins_for_each_known_client() {
        for (id, bundle, _) in CLIENTS {
            let image = resolve(
                id,
                |name| {
                    assert_eq!(name, bundle);
                    Some("installed")
                },
                |_| panic!("an installed app already supplied its icon"),
            );
            assert_eq!(image, Some("installed"), "{id}");
        }
    }

    #[test]
    fn missing_apps_use_their_fallback_symbols() {
        for (id, bundle, fallback) in CLIENTS {
            let image = resolve(
                id,
                |name| {
                    assert_eq!(name, bundle);
                    None
                },
                |symbol| {
                    assert_eq!(symbol, fallback);
                    Some("fallback")
                },
            );
            assert_eq!(image, Some("fallback"), "{id}");
        }
    }

    #[test]
    fn any_other_agent_always_uses_the_puzzle_piece() {
        let image = resolve(
            "other",
            |_| panic!("the generic row has no app"),
            |symbol| {
                assert_eq!(symbol, "puzzlepiece.extension");
                Some("puzzle piece")
            },
        );
        assert_eq!(image, Some("puzzle piece"));
    }
}
