//! S011 native setup sheet model. Values are never put in project state.
//! `ValuesService` is the missing env/Keychain service boundary, not an in-memory
//! replacement for persistent storage. Save completion must follow durable write.
use super::{Error, PrivateValue, RequestId, ServiceCode, Target};
use std::collections::{BTreeMap, BTreeSet};

pub const LOCAL_NOTE: &str =
    "Stays on this Mac. Never included when you publish or share this extension.";
pub const REENTER: &str = "Enter this again; the saved value can’t be read.";
pub const DESIGN_TODOS: &[&str] = &[];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Secret,
}
#[derive(Debug)]
pub struct Declaration {
    pub key: String,
    pub label: String,
    pub kind: Kind,
    pub required: bool,
    pub help: Option<String>,
}
#[derive(Debug)]
pub enum SavedValue {
    Missing,
    Plain(String),
    SecretSet,
    Unreadable,
}
#[derive(Debug)]
enum Draft {
    Keep,
    Set(PrivateValue),
}
#[derive(Debug)]
struct Row {
    declaration: Declaration,
    saved: SavedValue,
    draft: Draft,
    visible: bool,
}
impl Row {
    fn present(&self) -> bool {
        match &self.draft {
            Draft::Set(v) => !v.expose().is_empty(),
            Draft::Keep => match &self.saved {
                SavedValue::Plain(v) => !v.is_empty(),
                SavedValue::SecretSet => true,
                _ => false,
            },
        }
    }
    fn already_set(&self) -> bool {
        matches!(self.saved, SavedValue::SecretSet) && matches!(self.draft, Draft::Keep)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Widget {
    NSTextField,
    NSSecureTextField,
    AlreadySet,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowView {
    pub key: String,
    pub label: String,
    pub accessibility_label: String,
    pub widget: Widget,
    pub optional: Option<&'static str>,
    pub help: Option<String>,
    pub warning: Option<&'static str>,
    pub display_marker: Option<&'static str>,
    pub actions: Vec<&'static str>,
    pub eye_symbol: Option<&'static str>,
    pub eye_tooltip: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetView {
    pub target: Target,
    pub title: String,
    pub explanation: Option<String>,
    pub rows: Vec<RowView>,
    pub note: &'static str,
    pub cancel_label: &'static str,
    pub save_label: &'static str,
    pub can_edit: bool,
    pub can_save: bool,
    pub can_cancel: bool,
    pub error: Option<String>,
    pub focus_first_field_and_select: bool,
    pub return_saves: bool,
    pub escape_cancels: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    ExtensionSettings,
    MissingPanel,
    RequestValues,
    AgentSecretQuestion,
}
#[derive(Debug)]
pub enum Write {
    Keep {
        key: String,
    },
    Set {
        key: String,
        kind: Kind,
        value: PrivateValue,
    },
    Remove {
        key: String,
    },
}
#[derive(Debug)]
pub struct Save {
    pub request: RequestId,
    pub target: Target,
    /// Provenance env identity, not project id or extension display name.
    pub env_key: String,
    pub writes: Vec<Write>,
}
pub trait ValuesService {
    /// Nonblocking admission to the env/Keychain worker. Return the exact Save
    /// on queue-full so callers cannot lose typed secrets. Apply atomically or
    /// retain old values on error; no partial-success completion is permitted.
    fn enqueue_save(&mut self, save: Save) -> Result<(), (ServiceCode, Save)>;
    /// Tagged Remove is immediate and remains removed after Cancel. Reload the
    /// extension in every window after the worker commits (N6).
    fn enqueue_remove(
        &mut self,
        target: Target,
        request: RequestId,
        env_key: String,
        key: String,
    ) -> Result<(), ServiceCode>;
}

#[derive(Debug)]
enum SetupFailure {
    Save(ServiceCode),
    Reveal(String, ServiceCode),
    Remove(String, ServiceCode),
}
impl SetupFailure {
    fn copy(&self) -> String {
        let (message, code) = match self {
            Self::Save(c) => ("Couldn’t save these values.".into(), c),
            Self::Reveal(label, c) => (format!("Couldn’t show {label}."), c),
            Self::Remove(label, c) => (format!("Couldn’t remove {label}."), c),
        };
        if code.informative().is_empty() {
            message
        } else {
            format!("{message} {}", code.informative())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealFailure {
    Unreadable,
    Service(ServiceCode),
}
#[derive(Debug)]
pub struct Setup {
    target: Target,
    extension: String,
    env_key: String,
    rows: Vec<Row>,
    saving: Option<RequestId>,
    removing: Option<(RequestId, String)>,
    last_request: RequestId,
    revealing: BTreeMap<String, RequestId>,
    error: Option<SetupFailure>,
    closed: bool,
}
impl Setup {
    pub fn open(
        target: Target,
        extension: String,
        env_key: String,
        declarations: Vec<(Declaration, SavedValue)>,
        origin: Origin,
    ) -> Result<Self, Error> {
        if origin == Origin::AgentSecretQuestion {
            return Err(Error::InvalidAction);
        }
        let mut keys = BTreeSet::new();
        if env_key.is_empty()
            || declarations.iter().any(|(d, v)| {
                d.key.is_empty()
                    || !keys.insert(d.key.clone())
                    || matches!(
                        (d.kind, v),
                        (Kind::Plain, SavedValue::SecretSet) | (Kind::Secret, SavedValue::Plain(_))
                    )
            })
        {
            return Err(Error::InvalidAction);
        }
        Ok(Self {
            target,
            extension,
            env_key,
            rows: declarations
                .into_iter()
                .map(|(declaration, saved)| Row {
                    declaration,
                    saved,
                    draft: Draft::Keep,
                    visible: false,
                })
                .collect(),
            saving: None,
            removing: None,
            last_request: 0,
            revealing: BTreeMap::new(),
            error: None,
            closed: false,
        })
    }
    fn editable(&self) -> Result<(), Error> {
        if self.closed {
            Err(Error::Stale)
        } else if self.saving.is_some() || self.removing.is_some() {
            Err(Error::Busy)
        } else {
            Ok(())
        }
    }
    fn row_mut(&mut self, key: &str) -> Result<&mut Row, Error> {
        self.editable()?;
        self.rows
            .iter_mut()
            .find(|r| r.declaration.key == key)
            .ok_or(Error::Stale)
    }
    pub fn edit(&mut self, key: &str, value: PrivateValue) -> Result<(), Error> {
        let row = self.row_mut(key)?;
        if row.already_set() {
            return Err(Error::InvalidAction);
        }
        row.draft = Draft::Set(value);
        self.error = None;
        Ok(())
    }
    pub fn change(&mut self, key: &str) -> Result<(), Error> {
        let row = self.row_mut(key)?;
        if !row.already_set() {
            return Err(Error::InvalidAction);
        }
        row.draft = Draft::Set(PrivateValue::new(String::new()));
        row.visible = false;
        Ok(())
    }
    /// No confirmation. The row empties after the immediate service deletion;
    /// Cancel does not resurrect it (tagged authored behavior, N6).
    pub fn remove(
        &mut self,
        key: &str,
        request: RequestId,
        service: &mut impl ValuesService,
    ) -> Result<(), Error> {
        self.editable()?;
        if request <= self.last_request || !self.rows.iter().any(|r| r.declaration.key == key) {
            return Err(Error::InvalidAction);
        }
        service
            .enqueue_remove(
                self.target.clone(),
                request,
                self.env_key.clone(),
                key.into(),
            )
            .map_err(|code| {
                let label = self
                    .rows
                    .iter()
                    .find(|r| r.declaration.key == key)
                    .expect("checked")
                    .declaration
                    .label
                    .clone();
                self.error = Some(SetupFailure::Remove(label, code));
                Error::Service(code)
            })?;
        self.error = None;
        self.removing = Some((request, key.into()));
        self.last_request = request;
        Ok(())
    }
    pub fn complete_remove(
        &mut self,
        target: &Target,
        request: RequestId,
        result: Result<(), ServiceCode>,
    ) -> Result<(), Error> {
        if self.closed
            || *target != self.target
            || self.removing.as_ref().map(|r| r.0) != Some(request)
        {
            return Err(Error::Stale);
        }
        let (_, key) = self.removing.take().expect("checked");
        if let Err(code) = result {
            let label = self
                .rows
                .iter()
                .find(|r| r.declaration.key == key)
                .ok_or(Error::Stale)?
                .declaration
                .label
                .clone();
            self.error = Some(SetupFailure::Remove(label, code));
            return Err(Error::Service(code));
        }
        self.error = None;
        let row = self
            .rows
            .iter_mut()
            .find(|r| r.declaration.key == key)
            .ok_or(Error::Stale)?;
        row.saved = SavedValue::Missing;
        row.draft = Draft::Keep;
        row.visible = false;
        Ok(())
    }
    pub fn toggle_visibility(&mut self, key: &str) -> Result<(), Error> {
        let row = self.row_mut(key)?;
        if row.declaration.kind != Kind::Secret || row.already_set() {
            return Err(Error::InvalidAction);
        }
        row.visible = !row.visible;
        Ok(())
    }
    /// Only the native field adapter may call this. Not part of inspection/AX.
    pub fn field_value(&self, key: &str) -> Result<&str, Error> {
        let row = self
            .rows
            .iter()
            .find(|r| r.declaration.key == key)
            .ok_or(Error::Stale)?;
        if self.closed || row.already_set() {
            return Err(Error::InvalidAction);
        }
        Ok(match &row.draft {
            Draft::Set(v) => v.expose(),
            Draft::Keep => match &row.saved {
                SavedValue::Plain(v) => v,
                _ => "",
            },
        })
    }
    pub fn reveal_key(&self, key: &str) -> Result<(&Target, &str, &str, &str), Error> {
        self.editable()?;
        let row = self
            .rows
            .iter()
            .find(|r| r.declaration.key == key && r.already_set())
            .ok_or(Error::InvalidAction)?;
        // Worker resolves the secret only after this explicit action. It must
        // return it to Prompt::Reveal with this generation, not to a log/AX node.
        Ok((
            &self.target,
            &self.env_key,
            &row.declaration.key,
            &row.declaration.label,
        ))
    }
    /// Admit an explicit reveal before enqueueing bridge::Services::reveal. A
    /// delayed callback can never answer another request for the same key.
    pub fn begin_reveal(&mut self, key: &str, request: RequestId) -> Result<(), Error> {
        self.reveal_key(key)?;
        if request <= self.last_request {
            return Err(Error::InvalidAction);
        }
        self.last_request = request;
        self.revealing.insert(key.into(), request);
        Ok(())
    }
    fn finish_reveal(
        &mut self,
        target: &Target,
        key: &str,
        request: RequestId,
    ) -> Result<String, Error> {
        if self.closed || target != &self.target || self.revealing.get(key) != Some(&request) {
            return Err(Error::Stale);
        }
        self.revealing.remove(key);
        let row = self
            .rows
            .iter()
            .find(|r| r.declaration.key == key && r.already_set())
            .ok_or(Error::Stale)?;
        Ok(row.declaration.label.clone())
    }
    /// Success goes only to the explicit native Reveal alert, never SheetView.
    pub fn revealed(
        &mut self,
        target: &Target,
        key: &str,
        request: RequestId,
        value: PrivateValue,
    ) -> Result<super::Prompt, Error> {
        let label = self.finish_reveal(target, key, request)?;
        Ok(super::Prompt::Reveal {
            key: key.into(),
            label,
            value,
        })
    }
    /// Worker result for an explicit Reveal. Never forwards diagnostic text or a
    /// value into this inspectable model. Draft edits survive delayed failures.
    pub fn reveal_failed(
        &mut self,
        target: &Target,
        key: &str,
        request: RequestId,
        failure: RevealFailure,
    ) -> Result<(), Error> {
        self.finish_reveal(target, key, request)?;
        let row = self
            .rows
            .iter_mut()
            .find(|r| r.declaration.key == key)
            .ok_or(Error::Stale)?;
        if !row.already_set() {
            return Err(Error::Stale);
        }
        match failure {
            RevealFailure::Unreadable => {
                row.saved = SavedValue::Unreadable;
                row.visible = false;
                self.error = None;
            }
            RevealFailure::Service(code) => {
                self.error = Some(SetupFailure::Reveal(row.declaration.label.clone(), code))
            }
        }
        Ok(())
    }
    pub fn view(&self) -> SheetView {
        SheetView {
            target: self.target.clone(),
            title: format!("Set Up {}", self.extension),
            explanation: self
                .rows
                .iter()
                .any(|r| r.declaration.required && !r.present())
                .then(|| format!("{} needs these values to work.", self.extension)),
            rows: self
                .rows
                .iter()
                .map(|r| {
                    let secret = r.declaration.kind == Kind::Secret;
                    let already = r.already_set();
                    RowView {
                        key: r.declaration.key.clone(),
                        label: r.declaration.label.clone(),
                        accessibility_label: if already {
                            format!("{} is set", r.declaration.label)
                        } else {
                            r.declaration.label.clone()
                        },
                        widget: if already {
                            Widget::AlreadySet
                        } else if secret && !r.visible {
                            Widget::NSSecureTextField
                        } else {
                            Widget::NSTextField
                        },
                        optional: (!r.declaration.required).then_some("Optional"),
                        help: r.declaration.help.clone(),
                        warning: (matches!(r.saved, SavedValue::Unreadable) && !r.present())
                            .then_some(REENTER),
                        display_marker: already.then_some("••••"),
                        actions: if already {
                            vec!["Reveal", "Change", "Remove"]
                        } else {
                            vec![]
                        },
                        eye_symbol: (secret && !already).then_some("eye"),
                        eye_tooltip: (secret && !already).then(|| {
                            format!(
                                "{} {}",
                                if r.visible { "Hide" } else { "Show" },
                                r.declaration.label
                            )
                        }),
                    }
                })
                .collect(),
            note: LOCAL_NOTE,
            cancel_label: "Cancel",
            save_label: if self.saving.is_some() {
                "Saving…"
            } else {
                "Save"
            },
            can_edit: self.editable().is_ok(),
            can_save: self.editable().is_ok()
                && self
                    .rows
                    .iter()
                    .all(|r| !r.declaration.required || r.present()),
            can_cancel: !self.closed,
            error: self.error.as_ref().map(SetupFailure::copy),
            focus_first_field_and_select: true,
            return_saves: true,
            escape_cancels: true,
        }
    }
    pub fn save(
        &mut self,
        request: RequestId,
        service: &mut impl ValuesService,
    ) -> Result<(), Error> {
        self.editable()?;
        if request <= self.last_request || !self.view().can_save {
            return Err(Error::InvalidAction);
        }
        let writes = self
            .rows
            .iter()
            .map(|r| match &r.draft {
                Draft::Keep => Write::Keep {
                    key: r.declaration.key.clone(),
                },
                Draft::Set(value) => Write::Set {
                    key: r.declaration.key.clone(),
                    kind: r.declaration.kind,
                    // Exact authored bytes, including significant whitespace (N3/N6).
                    value: PrivateValue::new(value.expose().into()),
                },
            })
            .collect();
        let save = Save {
            request,
            target: self.target.clone(),
            env_key: self.env_key.clone(),
            writes,
        };
        if let Err((code, _)) = service.enqueue_save(save) {
            self.error = Some(SetupFailure::Save(code));
            return Err(Error::Service(code));
        }
        self.saving = Some(request);
        self.last_request = request;
        self.error = None;
        Ok(())
    }
    pub fn complete(
        &mut self,
        target: &Target,
        request: RequestId,
        result: Result<(), ServiceCode>,
    ) -> Result<(), Error> {
        if self.closed || *target != self.target || self.saving != Some(request) {
            return Err(Error::Stale);
        }
        self.saving = None;
        self.removing = None;
        match result {
            Ok(()) => {
                self.closed = true;
                self.rows.clear();
            }
            Err(code) => self.error = Some(SetupFailure::Save(code)),
        }
        Ok(())
    }
    pub fn cancel(&mut self) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Stale);
        }
        self.closed = true;
        self.rows.clear();
        self.revealing.clear();
        self.saving = None;
        self.removing = None;
        Ok(())
    }
    /// Call before the service invalidates the generation. The worker must
    /// check target generation at commit, even if a save is already queued.
    pub fn invalidate(&mut self) -> String {
        self.closed = true;
        self.rows.clear();
        self.revealing.clear();
        self.saving = None;
        self.removing = None;
        "Extension was changed, so the dialog closed.".into()
    }
    pub fn closed(&self) -> bool {
        self.closed
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingValues {
    pub title: String,
    pub line: String,
    pub button: &'static str,
    pub component: &'static str,
}
pub fn missing_values(extension: &str) -> MissingValues {
    MissingValues {
        title: format!("Set Up {extension}"),
        line: format!("{extension} needs a few values before it can run."),
        button: "Set Up…",
        component: "EmptyState",
    }
}
