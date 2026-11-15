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
/// The status line shows the kernel's own sentence, not the variant.
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kernel(message) | Self::Rejected(message) => f.write_str(message),
            other => write!(f, "{other:?}"),
        }
    }
}
impl std::error::Error for ClientError {}
impl From<ModelError> for ClientError {
    fn from(e: ModelError) -> Self {
        Self::Model(e)
    }
}

/// An immutable, disposable frame. Only scalar layer starts differ from the
/// authoritative document; keys and every other subtree stay shared.
#[derive(Clone, Debug)]
pub struct LayerPreviewFrame {
    pub starts: std::sync::Arc<[(String, f64)]>,
    pub snapshot: pm_kernel::Snapshot,
}
struct LayerPreview {
    drag: Drag,
    commands: Vec<pm_kernel::Command>,
    frame: Option<LayerPreviewFrame>,
    serial: u64,
    sample: Option<DragSample>,
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
    preview: Option<LayerPreview>,
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
            preview: None,
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
        if self.preview.as_ref().is_some_and(|preview| {
            preview.drag.start.doc_id != p.snapshot.doc_id
                || preview.drag.start.comp_id != p.snapshot.open_comp()
        }) {
            self.preview = None;
        }
        if let Some(sample) = self.preview.as_ref().and_then(|p| p.sample) {
            self.preview_sample(sample)?;
        }
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
        let document = plan.expected_document.unwrap_or(self.model.index.doc_id);
        let mut calls = plan.calls.into_iter().peekable();
        while let Some(call) = calls.next() {
            let call = match call {
                Call::Transact {
                    label,
                    body,
                    expected_revision,
                } => {
                    let mut selection = Vec::new();
                    while matches!(calls.peek(), Some(Call::Selection(_))) {
                        if let Some(Call::Selection(next)) = calls.next() {
                            selection.push(next);
                        }
                    }
                    if selection.is_empty() {
                        Call::Transact {
                            label,
                            body,
                            expected_revision,
                        }
                    } else {
                        Call::TransactWithSelection {
                            label,
                            body,
                            expected_revision,
                            selection: pm_kernel::SelectionCall::Batch(selection),
                        }
                    }
                }
                call => call,
            };
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
            .map_err(|e| ClientError::Kernel(format!("{e:?}")))?;
        self.sync()?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(ClientError::Rejected(format!("{e:?}")));
        }
        Ok(ack)
    }
    pub async fn begin_drag(&mut self, kind: DragKind) -> Result<(), ClientError> {
        if self.active.is_some() || self.preview.is_some() {
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
    /// Native Layers-mode bar moves preview locally. No writer lease, history,
    /// document revision or JS publication changes until mouse-up.
    pub fn begin_preview_drag(&mut self, kind: DragKind) -> Result<(), ClientError> {
        if self.active.is_some() || self.preview.is_some() {
            return Err(ClientError::GestureActive);
        }
        if !matches!(kind, DragKind::Layers { .. }) {
            return Err(ModelError::InvalidGesture.into());
        }
        self.sync()?;
        self.preview = Some(LayerPreview {
            drag: Drag::begin(&self.model, kind)?,
            commands: vec![],
            frame: None,
            serial: 0,
            sample: None,
        });
        Ok(())
    }
    pub fn layer_preview(&self) -> Option<&LayerPreviewFrame> {
        self.preview.as_ref().and_then(|p| p.frame.as_ref())
    }
    fn preview_sample(&mut self, sample: DragSample) -> Result<Ack, ClientError> {
        use pm_model::{Document, Op, Patch};
        let p = self
            .kernel
            .published(&self.project)
            .ok_or(ClientError::Closed)?;
        let preview = self.preview.as_mut().ok_or(ClientError::NoGesture)?;
        if p.snapshot.doc_id != preview.drag.start.doc_id
            || p.snapshot.open_comp() != preview.drag.start.comp_id
        {
            return Err(ClientError::DocumentChanged);
        }
        let commands = preview.drag.commands(sample)?;
        let starts: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                pm_kernel::Command::Layer(pm_kernel::LayerCommand::Set { layer_id, patch }) => {
                    patch.from.map(|from| (layer_id.clone(), from))
                }
                _ => None,
            })
            .collect();
        // Resolve the current stack once, including a concurrent reorder. A
        // foreign edit remains visible and cancellation never rolls it back.
        let root = p.snapshot.root().ok_or(ClientError::Closed)?;
        let comp = pm_kernel::model::comp_node(root, preview.drag.start.comp_id.as_deref())
            .ok_or(ClientError::DocumentChanged)?;
        let locations: std::collections::HashMap<_, _> = pm_kernel::model::layers_of(comp)
            .enumerate()
            .filter_map(|(index, layer)| {
                layer
                    .get("id")
                    .map(pm_model::Node::to_value)
                    .and_then(|id| id.as_str().map(|id| (id.to_owned(), index)))
            })
            .collect();
        let mut ops = Vec::with_capacity(starts.len());
        for (id, from) in &starts {
            let index = *locations
                .get(id)
                .ok_or_else(|| ModelError::UnknownId(id.clone()))?;
            let mut path = pm_kernel::model::LayerLoc {
                comp_id: preview.drag.start.comp_id.clone(),
                index,
            }
            .path();
            path.push("from".into());
            ops.push(Op::Leaf(Patch::set(path, pm_model::Value::Number(*from))));
        }
        preview.serial += 1;
        let revision = p.revision() + preview.serial;
        let mut document = Document::from_root(Some(root.clone()), p.revision());
        document
            .apply_ops_at(&ops, revision)
            .map_err(|e| ClientError::Kernel(e.to_string()))?;
        preview.sample = Some(sample);
        preview.commands = commands;
        preview.frame = Some(LayerPreviewFrame {
            starts: starts.into(),
            snapshot: pm_kernel::Snapshot {
                doc_id: p.snapshot.doc_id,
                revision,
                document: std::sync::Arc::new(document),
            },
        });
        Ok(Ack {
            txid: None,
            revision: p.revision(),
            folded: None,
            notice: None,
            reply: Reply::Applied {
                revision: p.revision(),
                changed: false,
            },
        })
    }
    pub async fn drag(&mut self, sample: DragSample) -> Result<Ack, ClientError> {
        if self.preview.is_some() {
            return self.preview_sample(sample);
        }
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
        if let Some(preview) = self.preview.take() {
            let published = self
                .kernel
                .published(&self.project)
                .ok_or(ClientError::Closed)?;
            if published.snapshot.doc_id != preview.drag.start.doc_id {
                self.sync()?;
                return Err(ClientError::DocumentChanged);
            }
            if commit {
                return self
                    .call(Call::Transact {
                        label: Some(preview.drag.label().into()),
                        body: WriteBody::Commands(preview.commands),
                        expected_revision: None,
                    })
                    .await;
            }
            self.sync()?;
            return Ok(Ack {
                txid: None,
                revision: published.revision(),
                folded: None,
                notice: None,
                reply: Reply::Applied {
                    revision: published.revision(),
                    changed: false,
                },
            });
        }
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
        if self.active.is_some() || self.preview.is_some() {
            self.finish_drag(false).await.map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn active_drag(&self) -> Option<&Drag> {
        self.preview
            .as_ref()
            .map(|p| &p.drag)
            .or_else(|| self.active.as_ref().map(|(_, d)| d))
    }
}
