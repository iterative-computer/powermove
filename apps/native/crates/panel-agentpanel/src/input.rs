//! Focus-local keyboard routing. Source: Composer.svelte:106–142,
//! ThreadPicker.svelte:206–215, QuestionCard.svelte:44–45 (v1.2.0).
#[derive(Clone, Debug, Default)]
pub struct Key {
    pub name: String,
    pub meta: bool,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub repeat: bool,
    pub composing: bool,
    pub code_229: bool,
}
impl Key {
    pub fn primary(&self) -> bool {
        self.meta || self.control
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    NativeText,
    NativePaste,
    Ignore,
    Submit,
    Newline,
    SelectAll,
    Undo,
    Redo,
    Delete { backward: bool, repeat: bool },
    MenuStep(i32),
    MenuChoose,
    DismissMenu,
    ClosePopover,
    BlurComposer,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub intent: Intent,
    pub stop_editor: bool,
    pub prevent_default: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ComposerContext {
    pub disabled: bool,
    pub menu_open: bool,
    pub popover_open: bool,
}
pub fn composer(key: &Key, context: ComposerContext) -> Route {
    let route = |intent, prevent_default| Route {
        intent,
        stop_editor: true,
        prevent_default,
    };
    if context.disabled {
        return route(Intent::Ignore, true);
    }
    if key.primary() && !key.alt && !key.shift && key.name.eq_ignore_ascii_case("v") {
        return Route {
            intent: Intent::NativePaste,
            stop_editor: false,
            prevent_default: false,
        };
    }
    if key.composing || key.code_229 {
        return route(Intent::NativeText, false);
    }
    if key.primary() && key.name.eq_ignore_ascii_case("z") {
        return route(
            if key.shift {
                Intent::Redo
            } else {
                Intent::Undo
            },
            true,
        );
    }
    if key.name == "Backspace" || key.name == "Delete" {
        return route(
            Intent::Delete {
                backward: key.name == "Backspace",
                repeat: key.repeat,
            },
            false,
        );
    }
    if key.name == "Enter" && key.repeat {
        return route(Intent::Ignore, true);
    }
    if context.menu_open {
        match key.name.as_str() {
            "Escape" => return route(Intent::DismissMenu, true),
            "ArrowDown" => return route(Intent::MenuStep(1), true),
            "ArrowUp" => return route(Intent::MenuStep(-1), true),
            "Enter" | "Tab" if !key.shift => return route(Intent::MenuChoose, true),
            _ => {}
        }
    }
    if key.name == "Escape" {
        return route(
            if context.popover_open {
                Intent::ClosePopover
            } else {
                Intent::BlurComposer
            },
            true,
        );
    }
    if key.primary() && !key.alt && key.name.eq_ignore_ascii_case("a") {
        return route(Intent::SelectAll, true);
    }
    if key.name == "Enter" {
        return route(
            if key.shift {
                Intent::Newline
            } else {
                Intent::Submit
            },
            true,
        );
    }
    route(Intent::NativeText, false)
}
pub fn question_submits(key: &Key) -> bool {
    key.name == "Enter" && !key.shift && !key.composing && !key.code_229
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashAction {
    New,
    Model,
    Provider,
    Effort,
    Attach,
    Stop,
    Settings,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub action: SlashAction,
    pub value: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// Labels are supplied by the provider; the model invents no display copy.
    pub models: Vec<(String, String)>,
    pub providers: Vec<(String, String)>,
    pub efforts: Vec<String>,
    /// Each listed model's own reasoning efforts (main's per-model choices).
    pub model_efforts: Vec<(String, Vec<String>)>,
    pub claude: bool,
    /// What a saved choice that is not a listed model stands for: `""` is the
    /// model the provider runs when none is chosen; an alias (Claude's
    /// resolved `claude-opus-5-5`) is its listed model.
    pub resolves: Vec<(String, String)>,
    /// The effort each listed model runs at when none is chosen, where the
    /// provider says (Codex's `defaultReasoningEffort`).
    pub default_efforts: Vec<(String, String)>,
}
/// The effort a model runs at when its provider names none: Claude Code's
/// documented default for its current models.
const USUAL_EFFORT: &str = "medium";
impl Catalog {
    /// The listed model a saved choice runs. No choice (`""`, `null` or an
    /// older `"default"`) is the provider's default model, so a picker always
    /// names a real model; the saved value itself is left as it was, and the
    /// provider still picks its own default for it. Unknown values stay as saved.
    pub fn resolve<'a>(&'a self, model: &'a str) -> &'a str {
        let key = if model == "default" { "" } else { model };
        if !key.is_empty() && self.models.iter().any(|(id, _)| id == key) {
            return model;
        }
        self.resolves
            .iter()
            .find(|(from, _)| from == key)
            .map(|(_, to)| to.as_str())
            .or_else(|| {
                key.is_empty()
                    .then(|| self.models.first().map(|(id, _)| id.as_str()))
                    .flatten()
            })
            .unwrap_or(model)
    }
    /// A choice a thread may save: a listed model, one it stands for, or none.
    pub fn accepts(&self, model: &str) -> bool {
        model.is_empty()
            || self.models.iter().any(|(m, _)| m == model)
            || self.resolves.iter().any(|(from, _)| from == model)
    }
    /// The efforts `model` takes: its own when the provider listed them, else
    /// every listed effort. `""` here is the effort's Default.
    pub fn efforts_for(&self, model: &str) -> Vec<String> {
        let model = self.resolve(model);
        match self.model_efforts.iter().find(|(id, _)| id == model) {
            Some((_, own)) => std::iter::once(String::new())
                .chain(own.iter().cloned())
                .collect(),
            None => self.efforts.clone(),
        }
    }
    /// The efforts the effort picker offers for `model`: the ones that model
    /// takes, never an empty "Default". None: the model has no effort levels
    /// and the picker is hidden.
    pub fn effort_choices(&self, model: &str) -> Vec<String> {
        self.efforts_for(model)
            .into_iter()
            .filter(|e| !e.is_empty() && e != "default")
            .collect()
    }
    /// The effort `model` runs at for a saved choice: the choice when the
    /// model takes it; else (none saved: `""`, `null`, an older `"default"`,
    /// or one this model lacks) the model's own default, which the provider
    /// names, else the usual default, else its first level. `""` only when
    /// the model has no effort levels.
    pub fn resolve_effort(&self, model: &str, effort: &str) -> String {
        let choices = self.effort_choices(model);
        if choices.iter().any(|e| e == effort) {
            return effort.to_owned();
        }
        let model = self.resolve(model);
        self.default_efforts
            .iter()
            .find(|(m, _)| m == model)
            .map(|(_, e)| e.as_str())
            .filter(|e| choices.iter().any(|c| c == e))
            .or_else(|| {
                choices
                    .iter()
                    .find(|c| *c == USUAL_EFFORT)
                    .map(String::as_str)
            })
            .or_else(|| choices.first().map(String::as_str))
            .unwrap_or_default()
            .to_owned()
    }
}
pub fn slash_choices(draft: &str, catalog: &Catalog, working: bool, blocked: bool) -> Vec<Choice> {
    slash_choices_for(draft, catalog, "", working, blocked)
}
/// The shortcut menu's rows for a thread on `model`: `/effort` lists the
/// efforts that model takes.
pub fn slash_choices_for(
    draft: &str,
    catalog: &Catalog,
    model: &str,
    working: bool,
    blocked: bool,
) -> Vec<Choice> {
    if !draft.starts_with('/') || draft.contains(['\r', '\n']) {
        return vec![];
    }
    let lower = draft[1..].to_lowercase();
    let command = lower.split(char::is_whitespace).next().unwrap_or_default();
    if let Some(at) = lower.find(char::is_whitespace) {
        let query = lower[at..].split_whitespace().collect::<Vec<_>>().join(" ");
        let (action, options) = match command {
            "model" => (SlashAction::Model, catalog.models.clone()),
            "provider" => (SlashAction::Provider, catalog.providers.clone()),
            "effort" => (
                SlashAction::Effort,
                catalog
                    .effort_choices(model)
                    .iter()
                    .map(|id| {
                        (
                            id.clone(),
                            if id == "xhigh" {
                                "Extra high".into()
                            } else {
                                id.clone()
                            },
                        )
                    })
                    .collect(),
            ),
            _ => return vec![],
        };
        let mut choices: Vec<_> = options
            .iter()
            .filter(|(id, label)| format!("{id} {label}").to_lowercase().contains(&query))
            .map(|(id, _)| Choice {
                id: format!("{command}-{id}"),
                action: action.clone(),
                value: Some(id.clone()),
            })
            .collect();
        if command == "model"
            && catalog.claude
            && valid_claude_id(&query)
            && !options.iter().any(|(id, _)| id == &query)
        {
            choices.push(Choice {
                id: format!("model-{query}"),
                action,
                value: Some(query),
            });
        }
        return choices;
    }
    [
        ("new", SlashAction::New),
        ("model", SlashAction::Model),
        ("effort", SlashAction::Effort),
        ("provider", SlashAction::Provider),
        ("attach", SlashAction::Attach),
        ("settings", SlashAction::Settings),
        ("stop", SlashAction::Stop),
    ]
    .into_iter()
    .filter(|(id, _)| {
        id.starts_with(&lower)
            && (*id != "new" || !blocked)
            && (*id != "stop" || working)
            // A model with no effort levels has nothing to set.
            && (*id != "effort" || !catalog.effort_choices(model).is_empty())
    })
    .map(|(id, action)| Choice {
        id: id.into(),
        action,
        value: None,
    })
    .collect()
}
pub fn valid_claude_id(id: &str) -> bool {
    id.strip_prefix("claude-").is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    })
}
#[derive(Clone, Debug, Default)]
pub struct SlashMenu {
    pub focused: bool,
    dismissed: Option<String>,
    pub index: usize,
}
impl SlashMenu {
    pub fn visible(&self, draft: &str, disabled: bool, choices: &[Choice]) -> bool {
        self.focused && !disabled && self.dismissed.as_deref() != Some(draft) && !choices.is_empty()
    }
    pub fn changed(&mut self) {
        self.index = 0;
        self.dismissed = None;
    }
    pub fn dismiss(&mut self, draft: String) {
        self.dismissed = Some(draft);
    }
    pub fn step(&mut self, delta: i32, count: usize) {
        self.index = if count == 0 {
            0
        } else {
            (self.index.min(count - 1) as i64 + i64::from(delta)).rem_euclid(count as i64) as usize
        };
    }
}

/// Tagged K003/K004 are loose-modifier chords: added Alt/Cmd/Ctrl still match;
/// fields and autorepeat do not. Registry arbitration stays with the host.
pub fn agent_shortcut(key: &Key, in_field: bool) -> bool {
    !in_field
        && !key.repeat
        && !key.composing
        && key.primary()
        && key.shift
        && key.name.eq_ignore_ascii_case("k")
}
