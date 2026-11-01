//! Keyboard and pointer actions use the same commands available to automation.
use crate::{
    Error, Result,
    input::{self, Choice, ComposerContext, Intent, Key, Route, SlashAction},
    model::{Effect, EffectKind, Panel, ShellAction},
};

pub struct Submission<'a> {
    pub run: &'a str,
    pub request: &'a str,
}
pub struct KeyOutcome {
    pub route: Route,
    pub effect: Option<Effect>,
}
impl Panel {
    pub fn focus_composer(&mut self, thread: &str, focused: bool) -> Result<()> {
        self.session_mut(thread)?.slash.focused = focused;
        Ok(())
    }
    pub fn slash_choices(&self, thread: &str) -> Result<Vec<Choice>> {
        let s = self.session(thread)?;
        Ok(input::slash_choices(
            &s.draft.legacy().0,
            &s.catalog,
            s.run.is_some(),
            s.applying,
        ))
    }
    /// The host supplies the id of a fresh run, or the current id for steering.
    /// NativeText/Paste routes are fulfilled by the platform text/IME adapter.
    pub fn composer_key(
        &mut self,
        thread: &str,
        key: &Key,
        popover_open: bool,
        ids: Submission<'_>,
        now: u64,
    ) -> Result<KeyOutcome> {
        let choices = self.slash_choices(thread)?;
        let s = self.session(thread)?;
        let text = s.draft.legacy().0;
        let menu_open = s.slash.visible(&text, s.applying, &choices);
        let mut route = input::composer(
            key,
            ComposerContext {
                disabled: s.applying,
                menu_open,
                popover_open,
            },
        );
        let effect = match &route.intent {
            Intent::Submit if s.draft.sendable() => {
                Some(self.submit(thread, ids.run, ids.request, now)?)
            }
            Intent::Newline => {
                self.draft_mut(thread)?.insert_text("\n", now, false);
                None
            }
            Intent::SelectAll => {
                self.session_mut(thread)?.draft.select_all();
                None
            }
            Intent::Undo => {
                self.session_mut(thread)?.draft.undo(false);
                None
            }
            Intent::Redo => {
                self.session_mut(thread)?.draft.undo(true);
                None
            }
            Intent::Delete { backward, repeat } => {
                let handled = self
                    .draft_mut(thread)?
                    .delete_token(*backward, *repeat, now);
                route.prevent_default = handled;
                if !handled {
                    route.intent = Intent::NativeText;
                }
                None
            }
            Intent::MenuStep(delta) => {
                self.session_mut(thread)?.slash.step(*delta, choices.len());
                None
            }
            Intent::DismissMenu => {
                self.session_mut(thread)?.slash.dismiss(text);
                None
            }
            Intent::MenuChoose => {
                let index = self
                    .session(thread)?
                    .slash
                    .index
                    .min(choices.len().saturating_sub(1));
                let choice = choices.get(index).ok_or(Error::Stale)?.id.clone();
                self.choose_slash(thread, &choice, now)?
            }
            Intent::ClosePopover => Some(self.host_action(thread, ShellAction::Close)?),
            Intent::BlurComposer => {
                self.focus_composer(thread, false)?;
                None
            }
            Intent::NativeText => {
                self.session_mut(thread)?.draft.disarm();
                None
            }
            _ => None,
        };
        Ok(KeyOutcome { route, effect })
    }
    /// Pointer activation and Enter/Tab selection share this exact handler.
    pub fn choose_slash(&mut self, thread: &str, id: &str, now: u64) -> Result<Option<Effect>> {
        let choice = self
            .slash_choices(thread)?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::UnknownId(id.into()))?;
        if choice.value.is_none() {
            let prefix = match choice.action {
                SlashAction::Model => Some("/model "),
                SlashAction::Provider => Some("/provider "),
                SlashAction::Effort => Some("/effort "),
                _ => None,
            };
            if let Some(prefix) = prefix {
                self.draft_mut(thread)?.set_text(prefix, now)?;
                return Ok(None);
            }
        }
        // Clear only the slash text, preserving attached files like setText in
        // the shipped InlinePrompt. External effects retain the captured thread.
        self.draft_mut(thread)?.set_text("", now)?;
        let target = self.target(thread)?;
        match choice.action {
            SlashAction::Model => {
                let effort = self.session(thread)?.settings.effort.clone();
                self.set_model(thread, choice.value.ok_or(Error::Stale)?, effort)?;
                Ok(None)
            }
            SlashAction::Effort => {
                let model = self.session(thread)?.settings.model.clone();
                self.set_model(thread, model, choice.value.ok_or(Error::Stale)?)?;
                Ok(None)
            }
            SlashAction::Provider => Ok(Some(self.effect(
                target,
                EffectKind::SelectProvider(choice.value.ok_or(Error::Stale)?),
            ))),
            SlashAction::Attach => Ok(Some(self.effect(target, EffectKind::ChooseFiles))),
            SlashAction::Settings => Ok(Some(self.effect(target, EffectKind::OpenSettings))),
            SlashAction::New => Ok(Some(self.effect(target, EffectKind::NewThread))),
            SlashAction::Stop => self.stop(thread).map(Some),
        }
    }
}
/// Source: Composer.svelte:185–192. A canvas gesture may cancel platform focus;
/// release it explicitly for primary/auxiliary clicks outside the composer.
pub fn outside_composer_blurs(button: u8, focused: bool, inside: bool) -> bool {
    matches!(button, 0 | 1) && focused && !inside
}
