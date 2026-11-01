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
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
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
        let ack = self
            .kernel
            .call(&self.project, Request::new(self.caller, call))
            .await
            .ok_or(ClientError::Closed)?
            .map_err(|e| ClientError::Kernel(format!("{e:?}")))?;
        self.sync()?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(ClientError::Rejected(format!("{e:?}")));
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
        for call in plan.calls {
            self.call(call).await?;
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
            .map_err(|e| ClientError::Kernel(format!("{e:?}")))?;
        self.sync()?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(ClientError::Rejected(format!("{e:?}")));
        }
        Ok(ack)
    }
    pub async fn begin_drag(&mut self, kind: DragKind) -> Result<(), ClientError> {
        if self.active.is_some() {
            return Err(ClientError::GestureActive);
        }
        self.sync()?;
        let mut drag = Drag::begin(&self.model, kind)?;
        if let DragKind::Property { targets, time } = &drag.kind {
            let published = self
                .kernel
                .published(&self.project)
                .ok_or(ClientError::Closed)?;
            let mut mirror = pm_kernel::QueryMirror::new(&published);
            for (layer, channel) in targets {
                let value = mirror
                    .queries()
                    .evaluate(self.model.index.comp_id.as_deref(), layer, channel, *time)
                    .map_err(|e| ClientError::Kernel(format!("{e:?}")))?;
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
                    label: drag.label().into(),
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
        let revision = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?
            .revision();
        if self.active_revision.is_some_and(|r| r != revision)
            && matches!(
                drag.kind,
                DragKind::Keys { .. }
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
        let (id, _) = self.active.as_ref().ok_or(ClientError::NoGesture)?;
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
    pub async fn publish_view(&self) -> Result<(), ClientError> {
        use pm_kernel::state::{StateOp, StateTransaction};
        let state = self.kernel.state();
        let old = state.ui.get(&self.project).cloned().unwrap_or_default();
        self.kernel
            .state_transaction(StateTransaction {
                expected_revision: Some(state.revision),
                ops: vec![StateOp::Ui {
                    project: self.project.clone(),
                    next: self.model.patch_ui(&old),
                }],
            })
            .await
            .ok_or(ClientError::Closed)?
            .map_err(|e| ClientError::Kernel(format!("{e:?}")))?;
        Ok(())
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
