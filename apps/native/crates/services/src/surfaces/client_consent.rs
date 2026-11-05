//! S070 consent spec at 78294c4c9: one native-profile question, recorded before
//! any external settings mutation. Gate facts come from the real app host.
use super::{
    Flight, Ticket,
    agent_clients::{Client, Consent},
};
use crate::{Result, ServiceErrorCode};
use serde::{Deserialize, Serialize};

pub const STORE_KEY: &str = "externalClients.claudeDesktop";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    Added,
    Declined,
    Skipped,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Absent,
    OriginalApp,
    ThisApp,
    ReceiptButRemoved,
    Customized,
    Unreadable,
}
#[derive(Debug, Default)]
pub struct Ready {
    pub installed: bool,
    pub editor_key: bool,
    pub hidden_mcp: bool,
    pub first_launch_finished: bool,
    pub onboarding_finished: bool,
    pub transient_open: bool,
    pub playing: bool,
    pub text_editing: bool,
}
impl Ready {
    fn permits_prompt(&self) -> bool {
        self.installed
            && self.editor_key
            && !self.hidden_mcp
            && self.first_launch_finished
            && self.onboarding_finished
            && !self.transient_open
            && !self.playing
            && !self.text_editing
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Add,
    SwitchOriginal,
}
pub enum Effect {
    Prompt { ticket: Ticket, kind: PromptKind },
    Record(RecordWork),
    None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Prompt(PromptKind),
    Recording,
}
#[derive(Debug, Default)]
pub struct Gate {
    pub answer: Option<Answer>,
    pub error: Option<ServiceErrorCode>,
    flight: Flight,
    stage: Option<Stage>,
}
impl Gate {
    pub fn from_profile(answer: Option<Answer>) -> Self {
        Self {
            answer,
            ..Self::default()
        }
    }
    pub fn consider(&mut self, ready: &Ready, found: Found) -> Effect {
        if self.answer.is_some()
            || self.error.is_some()
            || self.flight.current().is_some()
            || !ready.permits_prompt()
        {
            return Effect::None;
        }
        match found {
            Found::Unreadable => Effect::None,
            Found::ThisApp | Found::ReceiptButRemoved | Found::Customized => {
                self.stage = Some(Stage::Recording);
                Effect::Record(RecordWork {
                    ticket: self.flight.start(),
                    answer: Answer::Skipped,
                })
            }
            Found::Absent | Found::OriginalApp => {
                let kind = if found == Found::OriginalApp {
                    PromptKind::SwitchOriginal
                } else {
                    PromptKind::Add
                };
                self.stage = Some(Stage::Prompt(kind));
                Effect::Prompt {
                    ticket: self.flight.start(),
                    kind,
                }
            }
        }
    }
    pub fn respond(&mut self, ticket: Ticket, accepted: bool) -> Option<RecordWork> {
        if !self.flight.accepts(ticket) || !matches!(self.stage, Some(Stage::Prompt(_))) {
            return None;
        }
        self.stage = Some(Stage::Recording);
        Some(RecordWork {
            ticket,
            answer: if accepted {
                Answer::Added
            } else {
                Answer::Declined
            },
        })
    }
    pub fn receive(&mut self, reply: RecordReply) -> (bool, Option<Consent>) {
        if self.stage != Some(Stage::Recording) || !self.flight.finish(reply.ticket) {
            return (false, None);
        }
        self.stage = None;
        match reply.result {
            Ok(record) => {
                self.answer = Some(record.answer);
                (
                    true,
                    if record.fresh && record.answer == Answer::Added {
                        Consent::from_native_response(Client::ClaudeDesktop, true)
                    } else {
                        None
                    },
                )
            }
            Err(error) => {
                self.error = Some(error.code);
                (true, None)
            }
        }
    }
    /// Closing/quitting while the alert is open consumes no profile answer.
    /// Once a record is queued, retain the worker until its real outcome arrives.
    pub fn close_prompt(&mut self) {
        if matches!(self.stage, Some(Stage::Prompt(_))) {
            self.flight.cancel();
            self.stage = None;
        }
    }
}
pub struct RecordWork {
    pub ticket: Ticket,
    pub answer: Answer,
}
pub struct Recorded {
    pub answer: Answer,
    pub fresh: bool,
}
pub struct RecordReply {
    pub ticket: Ticket,
    pub result: Result<Recorded>,
}
pub trait ConsentStore: Send + 'static {
    /// Kernel-owned durable compare-and-set from absent. `fresh=false` if another
    /// window already answered; return that stored answer without changing it.
    fn record_once(&mut self, key: &str, answer: Answer) -> Result<Recorded>;
}
impl RecordWork {
    pub fn run(self, store: &mut impl ConsentStore) -> RecordReply {
        RecordReply {
            ticket: self.ticket,
            result: store.record_once(STORE_KEY, self.answer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> Ready {
        Ready {
            installed: true,
            editor_key: true,
            first_launch_finished: true,
            onboarding_finished: true,
            ..Ready::default()
        }
    }
    #[derive(Default)]
    struct Store {
        answer: Option<Answer>,
        fail: bool,
    }
    impl ConsentStore for Store {
        fn record_once(&mut self, key: &str, answer: Answer) -> Result<Recorded> {
            assert_eq!(key, STORE_KEY);
            if self.fail {
                return Err(crate::ServiceError::message(
                    ServiceErrorCode::DiskFull,
                    "fixture record failed",
                ));
            }
            if let Some(answer) = self.answer {
                Ok(Recorded {
                    answer,
                    fresh: false,
                })
            } else {
                self.answer = Some(answer);
                Ok(Recorded {
                    answer,
                    fresh: true,
                })
            }
        }
    }
    fn prompt(gate: &mut Gate, found: Found) -> Ticket {
        let Effect::Prompt { ticket, .. } = gate.consider(&ready(), found) else {
            panic!("prompt expected");
        };
        ticket
    }
    #[test]
    fn consent_waits_for_safe_editor_context_and_never_consumes_missing_install() {
        let mut gate = Gate::default();
        for field in [
            "installed",
            "key",
            "launch",
            "onboarding",
            "hidden",
            "transient",
            "playback",
            "text",
        ] {
            let mut r = ready();
            match field {
                "installed" => r.installed = false,
                "key" => r.editor_key = false,
                "launch" => r.first_launch_finished = false,
                "onboarding" => r.onboarding_finished = false,
                "hidden" => r.hidden_mcp = true,
                "transient" => r.transient_open = true,
                "playback" => r.playing = true,
                _ => r.text_editing = true,
            };
            assert!(matches!(gate.consider(&r, Found::Absent), Effect::None));
            assert!(gate.answer.is_none());
        }
        assert!(matches!(
            gate.consider(&ready(), Found::Absent),
            Effect::Prompt {
                kind: PromptKind::Add,
                ..
            }
        ));
    }
    #[test]
    fn durable_answer_precedes_registration_permission_and_is_once_per_profile() {
        let mut gate = Gate::default();
        let t = prompt(&mut gate, Found::OriginalApp);
        let work = gate.respond(t, true).unwrap();
        assert!(gate.answer.is_none());
        assert!(gate.respond(t, true).is_none());
        let mut store = Store::default();
        let (accepted, permit) = gate.receive(work.run(&mut store));
        assert!(accepted);
        assert!(permit.is_some());
        assert_eq!(store.answer, Some(Answer::Added));
        assert!(matches!(
            gate.consider(&ready(), Found::Absent),
            Effect::None
        ));
        assert!(matches!(
            Gate::from_profile(store.answer).consider(&ready(), Found::Absent),
            Effect::None
        ));
    }
    #[test]
    fn decline_and_existing_native_custom_or_removed_entries_never_authorize_writes() {
        let mut gate = Gate::default();
        let t = prompt(&mut gate, Found::Absent);
        let work = gate.respond(t, false).unwrap();
        assert!(gate.receive(work.run(&mut Store::default())).1.is_none());
        assert_eq!(gate.answer, Some(Answer::Declined));
        for found in [Found::ThisApp, Found::Customized, Found::ReceiptButRemoved] {
            let mut gate = Gate::default();
            let Effect::Record(work) = gate.consider(&ready(), found) else {
                panic!("skip record");
            };
            assert!(gate.receive(work.run(&mut Store::default())).1.is_none());
            assert_eq!(gate.answer, Some(Answer::Skipped));
        }
    }
    #[test]
    fn unreadable_and_quit_leave_the_question_unconsumed() {
        let mut gate = Gate::default();
        assert!(matches!(
            gate.consider(&ready(), Found::Unreadable),
            Effect::None
        ));
        let t = prompt(&mut gate, Found::Absent);
        gate.close_prompt();
        assert!(gate.respond(t, true).is_none());
        assert!(gate.answer.is_none());
        assert!(matches!(
            gate.consider(&ready(), Found::Absent),
            Effect::Prompt { .. }
        ));
    }
    #[test]
    fn failed_record_and_cross_window_race_do_not_grant_a_second_mutation() {
        let mut gate = Gate::default();
        let t = prompt(&mut gate, Found::Absent);
        let reply = gate.respond(t, true).unwrap().run(&mut Store {
            answer: None,
            fail: true,
        });
        assert!(gate.receive(reply).1.is_none());
        assert_eq!(gate.error, Some(ServiceErrorCode::DiskFull));
        let mut gate = Gate::default();
        let t = prompt(&mut gate, Found::Absent);
        let reply = gate.respond(t, true).unwrap().run(&mut Store {
            answer: Some(Answer::Declined),
            fail: false,
        });
        assert!(gate.receive(reply).1.is_none());
        assert_eq!(gate.answer, Some(Answer::Declined));
    }
}
