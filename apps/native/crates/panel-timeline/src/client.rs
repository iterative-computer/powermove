use crate::{Drag, DragKind, DragSample, HostEffect, ModelError, Plan, TimelineModel};
use pm_kernel::{Ack, Call, Caller, GestureCall, KernelHandle, Reply, Request, WriteBody};

#[derive(Debug)]
pub enum ClientError {
    Closed,
    Kernel(String),
    Rejected(String),
    Model(ModelError),
    GestureActive,
    NoGesture,
    DocumentChanged,
    OriginMismatch,
    ConcurrentRelativeGesture,
}
/// The status line shows a sentence, never a variant or an internal id.
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kernel(message) | Self::Rejected(message) => f.write_str(message),
            Self::Model(e) => write!(f, "{e}"),
            Self::Closed => f.write_str("The project is closed."),
            Self::GestureActive => f.write_str("Another timeline edit is still under way."),
            Self::NoGesture => f.write_str("That timeline edit has already ended."),
            Self::DocumentChanged => {
                f.write_str("The project changed meanwhile, so this timeline edit was not made.")
            }
            Self::OriginMismatch => f.write_str("That timeline edit belongs to another window."),
            Self::ConcurrentRelativeGesture => {
                f.write_str("Another edit is moving the same items at the same time.")
            }
        }
    }
}
impl std::error::Error for ClientError {}
impl From<ModelError> for ClientError {
    fn from(e: ModelError) -> Self {
        Self::Model(e)
    }
}

/// Awaitable kernel adapter. No wait(), call_blocking(), platform or JS calls.
/// Caller identity is injected so agent/MCP lock protections are retained.
pub struct TimelineClient {
    pub kernel: KernelHandle,
    pub project: String,
    pub caller: Caller,
    pub model: TimelineModel,
    active: Option<(u64, Drag)>,
    active_revision: Option<u64>,
    adopted: Option<pm_model::Value>,
}
impl TimelineClient {
    pub fn new(
        kernel: KernelHandle,
        project: impl Into<String>,
        caller: Caller,
    ) -> Result<Self, ClientError> {
        let mut this = Self {
            kernel,
            project: project.into(),
            caller,
            model: TimelineModel::default(),
            active: None,
            active_revision: None,
            adopted: None,
        };
        this.sync()?;
        Ok(this)
    }
    pub fn sync(&mut self) -> Result<(), ClientError> {
        let p = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?;
        self.model.sync(&p);
        Ok(())
    }
    pub async fn call(&mut self, call: Call) -> Result<Ack, ClientError> {
        self.call_document(call, self.model.index.doc_id).await
    }
    async fn call_document(&mut self, call: Call, document: u64) -> Result<Ack, ClientError> {
        let ack = self
            .kernel
            .call(
                &self.project,
                Request::new(self.caller, call).with_document(document),
            )
            .await
            .ok_or(ClientError::Closed)?
            .map_err(|e| ClientError::Kernel(e.to_string()))?;
        self.sync()?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(ClientError::Rejected(e.to_string()));
        }
        if let Reply::Edit(e) = &ack.reply
            && !e.ok
        {
            return Err(ClientError::Rejected(e.message.clone().unwrap_or_default()));
        }
        Ok(ack)
    }
    /// Stops on the first rejection: dependent selection/host effects never run.
    pub async fn dispatch(&mut self, plan: Plan) -> Result<Vec<HostEffect>, ClientError> {
        let published = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?;
        if plan
            .expected_document
            .is_some_and(|doc| doc != published.snapshot.doc_id)
        {
            return Err(ClientError::DocumentChanged);
        }
        if plan
            .required_origin
            .is_some_and(|origin| origin != self.caller.origin)
        {
            return Err(ClientError::OriginMismatch);
        }
        let document = plan.expected_document.unwrap_or(self.model.index.doc_id);
        for call in plan.calls {
            self.call_document(call, document).await?;
        }
        Ok(plan.effects)
    }
    /// Explicit idempotency supplied by an agent's per-writer sequence.
    pub async fn transact_idempotent(
        &mut self,
        commands: Vec<pm_kernel::Command>,
        txid: u64,
    ) -> Result<Ack, ClientError> {
        let request = self.model.transact(self.caller, commands, txid);
        let ack = self
            .kernel
            .call(&self.project, request)
            .await
            .ok_or(ClientError::Closed)?
            .map_err(|e| ClientError::Kernel(e.to_string()))?;
        self.sync()?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(ClientError::Rejected(e.to_string()));
        }
        Ok(ack)
    }
    pub async fn begin_drag(&mut self, kind: DragKind) -> Result<(), ClientError> {
        if self.active.is_some() {
            return Err(ClientError::GestureActive);
        }
        self.sync()?;
        let mut drag = Drag::begin(&self.model, kind)?;
        if let DragKind::ProEdit(command) = &drag.kind {
            let source = self
                .model
                .index
                .source
                .as_ref()
                .ok_or(ModelError::InvalidGesture)?;
            let command: pm_kernel::Command = command.clone().into();
            command
                .plan(
                    source.root().ok_or(ModelError::InvalidGesture)?,
                    self.caller.origin,
                )
                .map_err(|e| ClientError::Rejected(e.to_string()))?;
        }
        if let DragKind::Property { targets, time } = &drag.kind {
            let published = self
                .kernel
                .published(&self.project)
                .ok_or(ClientError::Closed)?;
            let mut mirror = pm_kernel::QueryMirror::new(&published);
            let mut evaluated = targets.clone();
            for (layer, channel) in targets {
                let paired = match channel.as_str() {
                    "scale.x" => Some("scale.y"),
                    "scale.y" => Some("scale.x"),
                    _ => None,
                };
                if let Some(paired) = paired {
                    if !evaluated.contains(&(layer.clone(), paired.into())) {
                        evaluated.push((layer.clone(), paired.into()));
                    }
                }
            }
            for (layer, channel) in &evaluated {
                let value = mirror
                    .queries()
                    .evaluate(self.model.index.comp_id.as_deref(), layer, channel, *time)
                    .map_err(|e| ClientError::Kernel(e.to_string()))?;
                let number = value
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or(ModelError::InvalidNumber)?;
                drag.property_values
                    .insert((layer.clone(), channel.clone()), number);
            }
        }
        let ack = self
            .call(
                GestureCall::Begin {
                    label: drag.label(),
                }
                .into(),
            )
            .await?;
        let Reply::Gesture(id) = ack.reply else {
            return Err(ClientError::Rejected("gesture acknowledgement".into()));
        };
        self.active_revision = Some(ack.revision);
        self.active = Some((id, drag));
        Ok(())
    }
    pub async fn drag(&mut self, sample: DragSample) -> Result<Ack, ClientError> {
        let (id, drag) = self.active.as_mut().ok_or(ClientError::NoGesture)?;
        let published = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?;
        if published.snapshot.doc_id != drag.start.doc_id {
            return Err(ClientError::DocumentChanged);
        }
        let revision = published.revision();
        if self.active_revision.is_some_and(|r| r != revision)
            && matches!(
                drag.kind,
                DragKind::ProEdit(_)
                    | DragKind::Timing(_)
                    | DragKind::Keys { .. }
                    | DragKind::GraphKeys { .. }
                    | DragKind::StretchKeys { .. }
                    | DragKind::TransformKeys { .. }
            )
        {
            return Err(ClientError::ConcurrentRelativeGesture);
        }
        let commands = drag.commands(sample)?;
        let id = *id;
        let ack = self
            .call(
                GestureCall::Step {
                    gesture: id,
                    body: WriteBody::Commands(commands),
                    replace: true,
                }
                .into(),
            )
            .await?;
        self.active_revision = Some(ack.revision);
        Ok(ack)
    }
    /// Escape uses the kernel's selective cancellation; foreign edits survive.
    /// A conflict notice is returned intact so the host can report a kept edit.
    pub async fn finish_drag(&mut self, commit: bool) -> Result<Ack, ClientError> {
        let (id, drag) = self.active.as_ref().ok_or(ClientError::NoGesture)?;
        let published = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?;
        if published.snapshot.doc_id != drag.start.doc_id {
            self.active = None;
            self.active_revision = None;
            return Err(ClientError::DocumentChanged);
        }
        let call = if commit {
            GestureCall::Commit { gesture: *id }
        } else {
            GestureCall::Cancel { gesture: *id }
        };
        let ack = self.call(call.into()).await?;
        self.active = None;
        self.active_revision = None;
        Ok(ack)
    }
    /// Adopt view facts another client of this project persisted. Rows depend
    /// on them, so a native surface and the panel worker must agree.
    pub fn adopt_view(&mut self) -> Result<bool, ClientError> {
        let p = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?;
        let view = p.ui_state.timeline.clone();
        if self.adopted.as_ref() == Some(&view) {
            return Ok(false);
        }
        self.adopted = Some(view);
        self.model.restore_ui(&p.ui_state);
        Ok(true)
    }
    /// [`publish_view`](Self::publish_view) when the view differs from the
    /// one published: every view the panel draws from (the mode, the graph,
    /// reveals) follows whatever changed the model, menu, key or control.
    pub async fn publish_view_if_changed(&self) -> Result<(), ClientError> {
        let state = self.kernel.state();
        let old = state.ui.get(&self.project).cloned().unwrap_or_default();
        if self.model.patch_ui(&old) == old {
            return Ok(());
        }
        self.publish_view().await
    }
    pub async fn publish_view(&self) -> Result<(), ClientError> {
        self.publish_view_from(self.kernel.state()).await
    }
    /// [`Self::publish_view`], patching `state` first. The view facts are a
    /// patch on the project's UI entry as read: when another write moved the
    /// state on meanwhile (the editor writes it on most changes), the patch
    /// is made again on the state as it is now, so nothing either wrote is
    /// lost (PM-476: a fast run of text boxes left the stale write's raw
    /// error in the status line).
    pub async fn publish_view_from(
        &self,
        mut state: std::sync::Arc<pm_kernel::state::StateSnapshot>,
    ) -> Result<(), ClientError> {
        use pm_kernel::state::{STATE_CHANGED, StateOp, StateTransaction};
        const TRIES: usize = 8;
        for attempt in 1..=TRIES {
            let old = state.ui.get(&self.project).cloned().unwrap_or_default();
            let written = self
                .kernel
                .state_transaction(StateTransaction {
                    expected_revision: Some(state.revision),
                    ops: vec![StateOp::Ui {
                        project: self.project.clone(),
                        next: self.model.patch_ui(&old),
                    }],
                })
                .await
                .ok_or(ClientError::Closed)?;
            match written {
                Ok(_) => return Ok(()),
                Err(pm_kernel::KernelFailure::Storage(m))
                    if m == STATE_CHANGED && attempt < TRIES =>
                {
                    state = self.kernel.state();
                }
                Err(e) => return Err(ClientError::Kernel(e.to_string())),
            }
        }
        unreachable!("the last try returns")
    }
    /// The host awaits disposal before retiring/replacing a panel. Drop never
    /// blocks the main thread and cannot silently cancel a kernel lease.
    pub async fn dispose(mut self) -> Result<Option<Ack>, ClientError> {
        if self.active.is_some() {
            self.finish_drag(false).await.map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn active_drag(&self) -> Option<&Drag> {
        self.active.as_ref().map(|(_, d)| d)
    }
}
