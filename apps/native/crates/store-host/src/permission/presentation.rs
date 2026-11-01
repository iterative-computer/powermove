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
    /// None is a neutral placeholder for an older nameless manifest.
    pub title: Option<String>,
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
        // DESIGN-TODO(opus): presentation for old records without a signed
        // display name. Leave the title absent; never show an ID/hash instead.
        let name = (!self.extension_name.is_empty()).then_some(&self.extension_name);
        if self.requires_trusted_realm {
            PermissionPresentation {
                surface: PermissionSurface::NsAlertWarning,
                title: name.map(|name| format!("Give {name} full access to Powermove?")),
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
                title: name.map(|name| format!("Allow “{name}” To:")),
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
