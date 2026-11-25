//! `command_invoked`: every command a person starts, by its fixed registry
//! id and how they started it. Commands outside the shipped tables (an
//! extension's own) are counted as "other", never by their name.
use pm_telemetry::{CommandInvoked, Event, Via};
use std::collections::HashSet;
use std::sync::OnceLock;

/// The shipped command ids: the keymap, the menu bar and the tools.
fn registry_id(name: &str) -> Option<&'static str> {
    static IDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| {
        let keymap = crate::keyboard::shipped().into_iter().map(|b| b.command);
        let menu = crate::menu::equivalents().into_iter().map(|e| e.command);
        let fixed = [
            "toolSelect",
            "toolHand",
            "toolZoom",
            "toolRotate",
            "toolAnchor",
            "toolShape",
            "toolPen",
            "toolText",
            "agent.undoTurn",
            "agent.pause",
            "agent.stop",
            "agent.steer",
        ]
        .map(str::to_owned);
        keymap
            .chain(menu)
            .chain(fixed)
            .map(|c| &*Box::leak(c.into_boxed_str()))
            .collect()
    })
    .get(name)
    .copied()
}

/// A command a person started (call where it is sent; free when telemetry
/// is off).
pub fn command(name: &str, via: Via) {
    if !pm_telemetry::enabled() {
        return;
    }
    pm_telemetry::capture(Event::CommandInvoked(CommandInvoked {
        command_id: registry_id(name).unwrap_or("other"),
        via,
        count: 1,
    }));
}

/// Panels a person asked for (the Window menu, the panel library, a
/// toggle) and not yet shown: only these count as `panel_opened`, never a
/// restore or an extension's panel arriving at launch.
static REQUESTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn panel_requested(id: &str) {
    if !pm_telemetry::enabled() {
        return;
    }
    if let Ok(mut requested) = REQUESTED.lock() {
        requested.retain(|r| r != id);
        requested.push(id.to_owned());
        if requested.len() > 16 {
            requested.remove(0);
        }
    }
}

/// Whether `id` was asked for (and forgets it).
pub fn take_requested(id: &str) -> bool {
    REQUESTED.lock().is_ok_and(|mut requested| {
        let before = requested.len();
        requested.retain(|r| r != id);
        requested.len() != before
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn shipped_commands_are_known_and_others_are_not() {
        assert_eq!(super::registry_id("zoomIn"), Some("zoomIn"));
        assert_eq!(super::registry_id("palette"), Some("palette"));
        assert_eq!(super::registry_id("toolPen"), Some("toolPen"));
        assert_eq!(super::registry_id("my-extension.secretThing"), None);
    }
}
