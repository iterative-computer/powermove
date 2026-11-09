//! S002/S003/S004/S014/S016/S020/S055, design/surfaces/documents.md.
//!
//! N1/N3: presentation descriptions contain only Opus-specified copy/semantics.
//! N4: discard uses the kernel archive operation; no source is removed.
//! N7: requests carry stable window/session IDs and monotonically unique tickets.
//! N8: hosts enqueue work; only workers may execute or wait for kernel/file IO.
//! `lifecycle.rs` owns the close and quit sequence; alerts and notices come from here.

#[cfg(test)]
#[path = "documents_tests.rs"]
mod tests;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Save,
    Cancel,
    DontSave,
    ReviewChanges,
    DiscardChanges,
    ShowInFinder,
    Ok,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Escape,
    CommandD,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub action: Action,
    pub label: &'static str,
    pub key: Option<Key>,
    pub destructive: bool,
}
impl Button {
    pub fn new(action: Action, label: &'static str, key: Option<Key>) -> Self {
        Self {
            action,
            label,
            key,
            destructive: false,
        }
    }
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub message: String,
    pub informative: String,
    pub buttons: Vec<Button>,
}
impl Alert {
    /// Aborted/closed alerts never choose a destructive or affirmative action.
    pub fn action_at(&self, index: Option<usize>) -> Option<Action> {
        index
            .and_then(|i| self.buttons.get(i))
            .map(|b| b.action)
            .or_else(|| {
                self.buttons
                    .iter()
                    .find(|b| b.action == Action::Cancel)
                    .map(|b| b.action)
            })
    }
}
fn cancel() -> Button {
    Button::new(Action::Cancel, "Cancel", Some(Key::Escape))
}
fn default(action: Action, label: &'static str) -> Button {
    Button::new(action, label, Some(Key::Return))
}
pub fn unsaved(name: &str) -> Alert {
    Alert {
        message: format!("Save changes to “{name}” before closing?"),
        informative: "Your project file hasn’t been updated. Local recovery is kept separately."
            .into(),
        buttons: vec![
            default(Action::Save, "Save"),
            cancel(),
            Button::new(Action::DontSave, "Don’t Save", Some(Key::CommandD)).destructive(),
        ],
    }
}
pub fn review_quit(count: usize) -> Alert {
    assert!(count >= 2);
    Alert { message: format!("You have {count} projects with unsaved changes. Do you want to review these changes before quitting?"), informative: "If you discard them, their project files aren’t updated. Local recovery is kept separately.".into(), buttons: vec![default(Action::ReviewChanges,"Review Changes…"), cancel(), Button::new(Action::DiscardChanges,"Discard Changes",None).destructive()] }
}
/// Wire spelling is ServiceErrorCode's serde snake_case, never Display/diagnostics.
/// Unknown future codes intentionally use the reviewed generic error.
pub fn service_error(code: &str, name: &str) -> (String, &'static str) {
    match code {
        "offline" => (
            "Powermove can’t connect.".into(),
            "Check your internet connection and try again.",
        ),
        "not_found" => (
            format!("“{name}” can’t be found."),
            "It may have been moved or deleted.",
        ),
        "too_large" => (format!("“{name}” is too large to open."), ""),
        "invalid_project" => (
            format!("“{name}” can’t be opened because it isn’t a Powermove project."),
            "",
        ),
        "permission_denied" => (
            format!("You don’t have permission to open “{name}”."),
            "Check its sharing and permissions in Finder.",
        ),
        "disk_full" => (
            "There isn’t enough disk space.".into(),
            "Free some space and try again.",
        ),
        "journal" => (
            format!("“{name}” couldn’t be opened."),
            "Powermove couldn’t start the recovery journal that protects unsaved work. Check free space and permissions in your library, then try again.",
        ),
        "invalid_input" | "invalid_data" => (
            format!("“{name}” couldn’t be read."),
            "The file may be damaged.",
        ),
        _ => (
            "The operation couldn’t be completed.".into(),
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    }
}
pub fn open_error(name: &str, code: &str, path_exists: bool) -> Alert {
    let (message, informative) = service_error(code, name);
    // README §1 first is the default. S004 explicitly makes OK default even
    // though Show in Finder is written first in prose: retain its explicit key.
    let mut buttons = vec![default(Action::Ok, "OK")];
    if path_exists {
        buttons.push(Button::new(Action::ShowInFinder, "Show in Finder", None));
    }
    Alert {
        message,
        informative: informative.into(),
        buttons,
    }
}
