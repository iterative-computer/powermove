//! Close/quit reducer. Effects execute asynchronously; replies are ticket checked.
//! Call observe with fresh kernel publications after flushing/joining/saving.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Window,
    Quit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Flushing,
    Waiting,
    ReviewQuit,
    Prompt,
    PickingSave,
    ChangedOnDisk,
    Saving,
    Refreshing,
    Barrier,
    Closing,
    PreparingQuit,
    RecordingRecoveries,
    Failed,
    Cancelled,
    Done,
}
#[derive(Debug)]
pub enum Effect {
    FlushEdits {
        ticket: Ticket,
        sessions: Vec<SessionHandle>,
    },
    Present(Request),
    Dismiss(Ticket),
    Select {
        window: WindowId,
        session: SessionHandle,
    },
    /// Join the host's existing save/import; never start another or abort it.
    Join {
        ticket: Ticket,
        session: SessionHandle,
    },
    Refresh {
        ticket: Ticket,
        session: SessionHandle,
    },
    Kernel {
        ticket: Ticket,
        call: ShellCall,
    },
    Notice {
        target: Target,
        notice: Notice,
    },
    /// Retire this session/tab; close its window only when its last tab retires.
    CloseDocument {
        window: WindowId,
        session: SessionHandle,
    },
    /// Resolve each id to its actual originating name/path on a worker and
    /// durably retain its inline library notice before acknowledging this ticket.
    ResolveRecoveryCopies {
        ticket: Ticket,
        ids: Vec<String>,
    },
    TerminateLater,
    Terminate(bool),
}
pub struct CloseFlow {
    pub intent: Intent,
    pub stage: Stage,
    pub documents: Vec<Document>,
    index: usize,
    ticket: Ticket,
    tickets: Tickets,
    started: Duration,
    finishing: bool,
    reviewed: bool,
    discard_all: bool,
    rejected: Option<SessionHandle>,
    retry_only: bool,
}
impl Default for CloseFlow {
    fn default() -> Self {
        Self {
            intent: Intent::Window,
            stage: Stage::Idle,
            documents: vec![],
            index: 0,
            ticket: Ticket(0),
            tickets: Tickets::default(),
            started: Duration::ZERO,
            finishing: false,
            reviewed: false,
            discard_all: false,
            rejected: None,
            retry_only: false,
        }
    }
}
impl CloseFlow {
    pub fn ticket(&self) -> Ticket {
        self.ticket
    }
    fn next(&mut self, stage: Stage) -> Ticket {
        self.stage = stage;
        self.ticket = self.tickets.next_ticket();
        self.ticket
    }
    fn current(&self) -> &Document {
        &self.documents[self.index]
    }
    fn request(&mut self, stage: Stage, target: Target, presentation: Presentation) -> Effect {
        let ticket = self.next(stage);
        Effect::Present(Request {
            ticket,
            target,
            presentation,
        })
    }
    pub fn begin(
        &mut self,
        intent: Intent,
        documents_in_tab_order: Vec<Document>,
        now: Duration,
    ) -> Vec<Effect> {
        if !matches!(
            self.stage,
            Stage::Idle | Stage::Failed | Stage::Cancelled | Stage::Done
        ) {
            return vec![];
        }
        self.intent = intent;
        self.documents = documents_in_tab_order;
        self.index = 0;
        self.reviewed = false;
        self.discard_all = false;
        self.rejected = None;
        self.retry_only = false;
        self.finishing = false;
        self.started = now;
        let ticket = self.next(Stage::Flushing);
        let mut effects = vec![];
        if intent == Intent::Quit {
            effects.push(Effect::TerminateLater);
        }
        effects.push(Effect::FlushEdits {
            ticket,
            sessions: self.documents.iter().map(|d| d.session.clone()).collect(),
        });
        effects
    }
    /// Stale generation updates cannot change the decision being reviewed.
    pub fn observe(&mut self, document: Document) -> bool {
        if let Some(slot) = self
            .documents
            .iter_mut()
            .find(|d| d.session == document.session)
        {
            *slot = document;
            true
        } else {
            false
        }
    }
    pub fn flushed(&mut self, ticket: Ticket, now: Duration) -> Vec<Effect> {
        if ticket != self.ticket || self.stage != Stage::Flushing {
            return vec![];
        }
        self.settle(now)
    }
    fn settle(&mut self, now: Duration) -> Vec<Effect> {
        // Before the multi-document count, all active work must be joined and
        // dirtiness observed again. This also preserves tab-order review.
        if !self.reviewed {
            if let Some(index) = self.documents.iter().position(|d| d.work().is_some()) {
                self.index = index;
                return self.wait(now);
            }
            let count = self.documents.iter().filter(|d| d.dirty).count();
            if self.intent == Intent::Quit && count >= 2 {
                return vec![self.request(
                    Stage::ReviewQuit,
                    Target {
                        owner: Owner::Application,
                        session: None,
                    },
                    Presentation::Alert(review_quit(count)),
                )];
            }
            self.reviewed = true;
            self.index = 0;
        }
        self.advance(now)
    }
    fn advance(&mut self, now: Duration) -> Vec<Effect> {
        if self.index == self.documents.len() {
            if self.intent == Intent::Quit {
                self.started = now;
                let ticket = self.next(Stage::PreparingQuit);
                return vec![Effect::Kernel {
                    ticket,
                    call: ShellCall::PrepareQuit,
                }];
            }
            self.next(Stage::Done);
            return vec![];
        }
        if self.current().work().is_some() {
            return self.wait(now);
        }
        let mut effects = vec![Effect::Select {
            window: self.current().window,
            session: self.current().session.clone(),
        }];
        if self.current().dirty && !self.discard_all {
            effects.push(self.request(
                Stage::Prompt,
                self.current().target(),
                Presentation::Alert(unsaved(&self.current().name)),
            ));
        } else {
            effects.extend(self.barrier(now));
        }
        effects
    }
    fn wait(&mut self, now: Duration) -> Vec<Effect> {
        self.started = now;
        self.finishing = false;
        let ticket = self.next(Stage::Waiting);
        vec![Effect::Join {
            ticket,
            session: self.current().session.clone(),
        }]
    }
    /// Call with a fresh observation even if joining work succeeded. A save
    /// receipt alone cannot establish that a concurrent edit was saved.
    pub fn joined(&mut self, ticket: Ticket, now: Duration) -> Vec<Effect> {
        if ticket != self.ticket || self.stage != Stage::Waiting {
            return vec![];
        }
        if self.current().work().is_some() {
            return vec![];
        }
        let mut effects = self.dismiss_finishing();
        effects.extend(self.settle(now));
        effects
    }
    fn dismiss_finishing(&mut self) -> Vec<Effect> {
        if std::mem::take(&mut self.finishing) {
            vec![Effect::Dismiss(self.ticket)]
        } else {
            vec![]
        }
    }
    pub fn tick(&mut self, now: Duration) -> Vec<Effect> {
        if self.finishing || now.saturating_sub(self.started) < Duration::from_secs(1) {
            return vec![];
        }
        let work = match self.stage {
            Stage::Waiting => self.current().work(),
            Stage::Saving => Some(Work::Saving),
            Stage::Barrier | Stage::PreparingQuit => Some(Work::Durability),
            _ => None,
        };
        let Some(work) = work else { return vec![] };
        // A windowless quit barrier has no owner/name; do not invent copy.
        let Some(doc) = self
            .documents
            .get(self.index.min(self.documents.len().saturating_sub(1)))
        else {
            return vec![];
        };
        self.finishing = true;
        vec![Effect::Present(Request {
            ticket: self.ticket,
            target: doc.target(),
            presentation: Presentation::Finishing(Finishing::new(&doc.name, work)),
        })]
    }
    pub fn cancel(&mut self, ticket: Ticket) -> Vec<Effect> {
        if ticket != self.ticket
            || !matches!(
                self.stage,
                Stage::Waiting
                    | Stage::ReviewQuit
                    | Stage::Prompt
                    | Stage::PickingSave
                    | Stage::ChangedOnDisk
                    | Stage::Saving
                    | Stage::Barrier
                    | Stage::PreparingQuit
                    | Stage::Failed
            )
        {
            return vec![];
        }
        let mut effects = vec![Effect::Dismiss(self.ticket)];
        self.finishing = false;
        self.next(Stage::Cancelled);
        if self.intent == Intent::Quit {
            effects.push(Effect::Terminate(false));
        }
        effects
    }
    pub fn choose(&mut self, ticket: Ticket, action: Option<Action>, now: Duration) -> Vec<Effect> {
        if ticket != self.ticket {
            return vec![];
        }
        // System sheet dismissal/cancel is always non-affirmative.
        if action.is_none() || action == Some(Action::Cancel) {
            return self.cancel(ticket);
        }
        match (self.stage, action) {
            (Stage::ReviewQuit, Some(Action::ReviewChanges | Action::DiscardChanges)) => {
                self.discard_all = action == Some(Action::DiscardChanges);
                self.reviewed = true;
                self.index = 0;
                self.advance(now)
            }
            (Stage::ChangedOnDisk, Some(Action::SaveAnyway)) => {
                self.documents[self.index].disk_changed = false;
                self.save(false, now)
            }
            (Stage::ChangedOnDisk, Some(Action::SaveAs)) => self.save(true, now),
            (Stage::Prompt, Some(Action::Save)) => self.save(false, now),
            (Stage::Prompt, Some(Action::DontSave)) => self.discard_current(now),
            (Stage::Failed, Some(Action::SaveAs))
                if self.retry_only && self.index < self.documents.len() =>
            {
                self.save(true, now)
            }
            (Stage::Failed, Some(Action::Ok)) if !self.retry_only => {
                self.next(Stage::Cancelled);
                vec![]
            }
            _ => vec![],
        }
    }
    fn save(&mut self, force_picker: bool, now: Duration) -> Vec<Effect> {
        match save_preflight(self.current(), force_picker) {
            SavePreflight::Join(_) => self.wait(now),
            SavePreflight::Picker(picker) => vec![self.request(
                Stage::PickingSave,
                self.current().target(),
                Presentation::Picker(picker),
            )],
            SavePreflight::ReviewChanged(alert) => vec![self.request(
                Stage::ChangedOnDisk,
                self.current().target(),
                Presentation::Alert(alert),
            )],
            SavePreflight::Save => {
                self.started = now;
                let ticket = self.next(Stage::Saving);
                vec![Effect::Kernel {
                    ticket,
                    call: ShellCall::Save(self.current().session.clone()),
                }]
            }
        }
    }
    pub fn picked(&mut self, ticket: Ticket, path: Option<PathBuf>, now: Duration) -> Vec<Effect> {
        if ticket != self.ticket || self.stage != Stage::PickingSave {
            return vec![];
        }
        let Some(path) = path else {
            return self.cancel(ticket);
        };
        self.started = now;
        let ticket = self.next(Stage::Saving);
        vec![Effect::Kernel {
            ticket,
            call: ShellCall::SaveAs {
                session: self.current().session.clone(),
                path,
                replace: true,
            },
        }]
    }
    fn barrier(&mut self, now: Duration) -> Vec<Effect> {
        self.started = now;
        let ticket = self.next(Stage::Barrier);
        vec![Effect::Kernel {
            ticket,
            call: ShellCall::DurabilityBarrier,
        }]
    }
    fn discard_current(&mut self, now: Duration) -> Vec<Effect> {
        // This approval is local to this document, never an instruction to save
        // its rejected work during final PrepareQuit. Kernel archives it.
        self.discard_all_for_current();
        self.barrier(now)
    }
    fn discard_all_for_current(&mut self) {
        self.rejected = self.current().session.clone().into();
    }
    pub fn refreshed(&mut self, ticket: Ticket, now: Duration) -> Vec<Effect> {
        if ticket != self.ticket || self.stage != Stage::Refreshing {
            return vec![];
        }
        if self.retry_only {
            self.next(Stage::Cancelled);
            return vec![];
        }
        self.advance(now)
    }
    pub fn recoveries_recorded(&mut self, ticket: Ticket) -> Vec<Effect> {
        if ticket != self.ticket || self.stage != Stage::RecordingRecoveries {
            return vec![];
        }
        self.next(Stage::Done);
        vec![Effect::Terminate(true)]
    }
    /// `code` comes from ServiceErrorCode, not error text. Kernel currently
    /// exposes untyped Storage failures: the host must use "unknown" for those.
    pub fn failed(&mut self, ticket: Ticket, code: &str) -> Vec<Effect> {
        if ticket != self.ticket {
            return vec![];
        }
        let mut effects = self.dismiss_finishing();
        if self.documents.is_empty()
            || matches!(
                self.stage,
                Stage::PreparingQuit | Stage::RecordingRecoveries
            )
        {
            self.retry_only = false;
            let alert = open_error("", code, false);
            effects.push(self.request(
                Stage::Failed,
                Target {
                    owner: Owner::Application,
                    session: None,
                },
                Presentation::Alert(alert),
            ));
            if self.intent == Intent::Quit {
                effects.push(Effect::Terminate(false));
            }
            return effects;
        }
        self.retry_only = true;
        let index = self.index.min(self.documents.len() - 1);
        self.index = index;
        let target = self.current().target();
        let alert = failed_save(&self.current().name, code);
        effects.push(self.request(Stage::Failed, target, Presentation::Alert(alert)));
        if self.intent == Intent::Quit {
            effects.push(Effect::Terminate(false));
        }
        effects
    }
    pub fn reply(&mut self, ticket: Ticket, reply: ShellReply, now: Duration) -> Vec<Effect> {
        if ticket != self.ticket {
            return vec![];
        }
        let valid = match (&self.stage, &reply) {
            (Stage::Saving, ShellReply::Saved { session, .. }) => {
                session == &self.current().session
            }
            (Stage::Barrier, ShellReply::Durable)
            | (Stage::Closing, ShellReply::Closed { .. })
            | (Stage::PreparingQuit, ShellReply::Compacted { .. }) => true,
            _ => false,
        };
        if !valid {
            return self.failed(ticket, "unknown");
        }
        let mut effects = self.dismiss_finishing();
        if let Some(doc) = self
            .documents
            .get(self.index.min(self.documents.len().saturating_sub(1)))
        {
            effects.extend(
                recovery_notices(&doc.name, &reply)
                    .into_iter()
                    .map(|notice| Effect::Notice {
                        target: doc.target(),
                        notice,
                    }),
            );
        }
        match self.stage {
            Stage::Saving => {
                let ticket = self.next(Stage::Refreshing);
                effects.push(Effect::Refresh {
                    ticket,
                    session: self.current().session.clone(),
                });
            }
            Stage::Barrier => {
                let discard = self.current().dirty
                    && (self.discard_all
                        || self.rejected.as_ref() == Some(&self.current().session));
                if self.current().dirty && !discard {
                    effects.extend(self.advance(now));
                    return effects;
                }
                let call = if discard {
                    ShellCall::DiscardClose(self.current().session.clone())
                } else {
                    // Close also saves. A clean release must not rewrite a file
                    // another app changed, or implicitly save a copy's source.
                    ShellCall::DeferClose(self.current().session.clone())
                };
                let ticket = self.next(Stage::Closing);
                effects.push(Effect::Kernel { ticket, call });
            }
            Stage::Closing => {
                effects.push(Effect::CloseDocument {
                    window: self.current().window,
                    session: self.current().session.clone(),
                });
                self.rejected = None;
                self.index += 1;
                effects.extend(self.advance(now));
            }
            Stage::PreparingQuit => {
                if let ShellReply::Compacted {
                    recovery_copies, ..
                } = reply
                {
                    if recovery_copies.is_empty() {
                        self.next(Stage::Done);
                        effects.push(Effect::Terminate(true));
                    } else {
                        let ticket = self.next(Stage::RecordingRecoveries);
                        effects.push(Effect::ResolveRecoveryCopies {
                            ticket,
                            ids: recovery_copies,
                        });
                    }
                }
            }
            _ => unreachable!(),
        }
        effects
    }
}
