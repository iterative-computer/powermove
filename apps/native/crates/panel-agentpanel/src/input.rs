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
    pub claude: bool,
}
pub fn slash_choices(draft: &str, catalog: &Catalog, working: bool, blocked: bool) -> Vec<Choice> {
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
                    .efforts
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
        id.starts_with(&lower) && (*id != "new" || !blocked) && (*id != "stop" || working)
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
