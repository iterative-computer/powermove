//! Platform-neutral output for the AppKit adapter; no metrics or drawn UI.
//! Copy is verbatim from design/reviews/na-cloud-storehost.md, items 3–4.
use super::{Decision, Permission, PermissionRequest};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PermissionCopy {
    pub permission: Permission,
    pub symbol: &'static str,
    pub name: &'static str,
    pub detail: &'static str,
    /// Store disclosure stays distinct from the consent sheet's name/detail.
    pub store_label: &'static str,
}

pub const PERMISSION_COPY: [PermissionCopy; 5] = [
    PermissionCopy {
        permission: Permission::FullAccess,
        symbol: "exclamationmark.shield",
        name: "Full Access",
        detail: "Runs with the same access as Powermove.",
        store_label: "Needs full access to Powermove",
    },
    PermissionCopy {
        permission: Permission::ProjectWrite,
        symbol: "pencil",
        name: "Edit Your Project",
        detail: "Can change layers, keyframes and media in open projects.",
        store_label: "Edits your project",
    },
    PermissionCopy {
        permission: Permission::Assets,
        symbol: "photo.on.rectangle",
        name: "Import Files",
        detail: "Can add files you choose to your project.",
        store_label: "Imports files",
    },
    PermissionCopy {
        permission: Permission::Network,
        symbol: "network",
        name: "Use the Network",
        detail: "Can load images, fonts and data from the internet.",
        store_label: "Uses the network",
    },
    PermissionCopy {
        permission: Permission::Clipboard,
        symbol: "doc.on.clipboard",
        name: "Use the Clipboard",
        detail: "Can copy to and read from the clipboard when you ask it to.",
        store_label: "Uses the clipboard",
    },
];

pub const FULL_ACCESS_DETAIL: &str = "It will run with the same access as the app: your projects, files you open, the network, and other extensions’ values. Only trust extensions from people you know. You can revoke this from the Library.";
pub const SETTINGS_ALLOW_LABEL: &str = "Allow";

/// Missing-metadata presentation (design re-review 2026-11-01). Older records
/// can lack a signed display name, host publisher name or icon; these stand in.
/// Never substitute an extension ID, bundle hash or publisher ID.
pub const NAMELESS_SHEET_TITLE: &str = "Allow This Extension To:";
pub const NAMELESS_ALERT_TITLE: &str = "Give this extension full access to Powermove?";
pub const UNIDENTIFIED_PUBLISHER_LINE: &str = "From an unidentified developer";
/// SF Symbol drawn in place of a missing bundle icon (`symbol_panel` size,
/// `secondary` colour, centred in the icon well).
pub const PLACEHOLDER_ICON_SYMBOL: &str = "puzzlepiece.extension";
/// Trailing badge after a verified publisher name (`accent_text` colour).
pub const VERIFIED_PUBLISHER_SYMBOL: &str = "checkmark.seal.fill";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PermissionSurface {
    /// Window-modal, attached below the parent toolbar, never centred.
    WindowModalSheet,
    /// System NSAlert with warning style/symbol, Cancel as default.
    NsAlertWarning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PermissionButtonStyle {
    System,
    /// Adapter resolves the native accent via pm-design / AccentColor.
    Accent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PermissionButton {
    pub label: &'static str,
    pub decision: Decision,
    pub style: PermissionButtonStyle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionPresentation {
    pub surface: PermissionSurface,
    /// Always present: nameless records use `NAMELESS_*_TITLE`.
    pub title: String,
    /// Secondary line under the title: `From Acme` / `From Acme · Unverified`
    /// / `From an unidentified developer`. `muted`, never the publisher ID.
    pub publisher_line: String,
    /// `VERIFIED_PUBLISHER_SYMBOL` only when the host verified a named publisher.
    pub publisher_badge: Option<&'static str>,
    /// `PLACEHOLDER_ICON_SYMBOL` when the manifest has no icon; `None` means
    /// draw the request's bundle `icon` asset.
    pub icon_symbol: Option<&'static str>,
    pub informative_text: Option<&'static str>,
    pub rows: Vec<PermissionCopy>,
    pub buttons: [PermissionButton; 2],
    pub default_button: usize,
    pub cancel_button: usize,
    /// Use this unchanged as the sheet/alert's AX identifier (also for MCP).
    pub accessibility_identifier: String,
}

impl PermissionRequest {
    /// AppKit renders all strings as plain text. Layout/material/type/spacing
    /// come from pm-design; this crate introduces no visual values.
    pub fn presentation(&self) -> PermissionPresentation {
        let rows = PERMISSION_COPY
            .into_iter()
            .filter(|row| self.permissions.contains(&row.permission))
            .collect();
        let name = (!self.extension_name.is_empty()).then_some(&self.extension_name);
        let named_publisher = !self.publisher_name.is_empty();
        let publisher_line = match (named_publisher, self.publisher_verified) {
            (false, _) => UNIDENTIFIED_PUBLISHER_LINE.to_owned(),
            (true, true) => format!("From {}", self.publisher_name),
            (true, false) => format!("From {} · Unverified", self.publisher_name),
        };
        let publisher_badge =
            (named_publisher && self.publisher_verified).then_some(VERIFIED_PUBLISHER_SYMBOL);
        let icon_symbol = self.icon.is_none().then_some(PLACEHOLDER_ICON_SYMBOL);
        if self.requires_trusted_realm {
            PermissionPresentation {
                surface: PermissionSurface::NsAlertWarning,
                title: name.map_or_else(
                    || NAMELESS_ALERT_TITLE.to_owned(),
                    |name| format!("Give {name} full access to Powermove?"),
                ),
                publisher_line,
                publisher_badge,
                icon_symbol,
                informative_text: Some(FULL_ACCESS_DETAIL),
                rows,
                buttons: [
                    PermissionButton {
                        label: "Trust",
                        decision: Decision::Allow,
                        style: PermissionButtonStyle::System,
                    },
                    PermissionButton {
                        label: "Cancel",
                        decision: Decision::Deny,
                        style: PermissionButtonStyle::System,
                    },
                ],
                default_button: 1,
                cancel_button: 1,
                accessibility_identifier: self.id.clone(),
            }
        } else {
            PermissionPresentation {
                surface: PermissionSurface::WindowModalSheet,
                title: name.map_or_else(
                    || NAMELESS_SHEET_TITLE.to_owned(),
                    |name| format!("Allow “{name}” To:"),
                ),
                publisher_line,
                publisher_badge,
                icon_symbol,
                informative_text: None,
                rows,
                buttons: [
                    PermissionButton {
                        label: "Don’t Allow",
                        decision: Decision::Deny,
                        style: PermissionButtonStyle::System,
                    },
                    PermissionButton {
                        label: "Allow",
                        decision: Decision::Allow,
                        style: PermissionButtonStyle::Accent,
                    },
                ],
                default_button: 1,
                cancel_button: 0,
                accessibility_identifier: self.id.clone(),
            }
        }
    }
}
