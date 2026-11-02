//! Real single-writer boundary. Missing command implementations stay errors;
//! this adapter never writes an optimistic copy of the document.
use super::animators::{Animator, Animators};
use pm_kernel::commands::{MaskCommand, PropertyCommand, TextCommand, properties::SetProperty};
use pm_kernel::kernel::KernelFailure;
use pm_kernel::{
    Ack, Call, Caller, Command, GestureCall, KernelHandle, Pending, Published, Request, WriteBody,
};
use pm_model::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub project: String,
    pub document: u64,
    pub revision: u64,
    pub layer: String,
}
pub struct NewVertex<'a> {
    pub path: &'a str,
    pub vertex: &'a str,
    pub mask: Option<&'a str>,
    pub new_path: bool,
    pub x: f64,
    pub y: f64,
}
impl Target {
    pub fn from_published(p: &Published, layer: &str) -> Option<Self> {
        let root = p.snapshot.root()?;
        pm_kernel::model::find_layer(root, layer)?;
        Some(Self {
            project: p.project.clone(),
            document: p.snapshot.doc_id,
            revision: p.revision(),
            layer: layer.into(),
        })
    }
    pub fn request(&self, commands: Vec<Command>, label: Option<String>) -> Request {
        Request::new(
            Caller::native(),
            Call::Transact {
                label,
                body: WriteBody::Commands(commands),
                expected_revision: Some(self.revision),
            },
        )
    }
    pub fn set_text(&self, text: String, time: f64) -> Request {
        self.request(
            vec![
                TextCommand::SetText {
                    layer_id: self.layer.clone(),
                    text,
                    time,
                }
                .into(),
            ],
            None,
        )
    }
    pub fn paragraph(&self, width: f64, height: Option<f64>, time: f64) -> Request {
        self.request(
            vec![
                TextCommand::SetBox {
                    layer_id: self.layer.clone(),
                    width: Some(width),
                    height,
                    time,
                }
                .into(),
            ],
            None,
        )
    }
    pub fn add_path_vertex(&self, vertex: NewVertex<'_>) -> Request {
        let mut commands = vec![];
        if vertex.new_path {
            commands.push(
                MaskCommand::AddPath {
                    layer_id: self.layer.clone(),
                    path_id: vertex.path.into(),
                    mask_id: vertex.mask.map(str::to_owned),
                }
                .into(),
            );
        }
        commands.push(
            MaskCommand::AddVertex {
                layer_id: self.layer.clone(),
                path_id: vertex.path.into(),
                vertex_id: vertex.vertex.into(),
                x: vertex.x,
                y: vertex.y,
            }
            .into(),
        );
        self.request(commands, None)
    }
    pub fn close_path(&self, path: &str) -> Request {
        self.request(
            vec![
                MaskCommand::ClosePath {
                    layer_id: self.layer.clone(),
                    path_id: path.into(),
                }
                .into(),
            ],
            None,
        )
    }
    pub fn delete_vertex(&self, path: &str, vertex: &str) -> Request {
        self.request(
            vec![
                MaskCommand::DeleteVertex {
                    layer_id: self.layer.clone(),
                    path_id: path.into(),
                    vertex_id: vertex.into(),
                }
                .into(),
            ],
            None,
        )
    }
    pub fn property(&self, channel: &str, value: Value, time: f64) -> Request {
        let mut property = SetProperty::new(&self.layer, channel, value, time);
        property.preserve_hand_edits = false; // trusted human edit, as tagged inspector
        self.request(vec![PropertyCommand::Set(property).into()], None)
    }
    pub fn add_animator(&self, id: &str, animator: Value) -> Request {
        self.request(
            vec![
                TextCommand::AddAnimator {
                    layer_id: self.layer.clone(),
                    animator_id: id.into(),
                    animator,
                }
                .into(),
            ],
            None,
        )
    }
    pub fn remove_animator(&self, id: &str) -> Request {
        self.request(
            vec![
                TextCommand::RemoveAnimator {
                    layer_id: self.layer.clone(),
                    animator_id: id.into(),
                }
                .into(),
            ],
            None,
        )
    }
    pub fn style_text(&self, id: &str, style: Value) -> Request {
        self.request(
            vec![
                TextCommand::SetStyleRange {
                    layer_id: self.layer.clone(),
                    style_id: id.into(),
                    style: Some(style),
                }
                .into(),
            ],
            None,
        )
    }
    pub fn begin_preview(&self, property: &str) -> Request {
        Request::new(
            Caller::native(),
            GestureCall::Begin {
                label: format!("Change {property}"),
            },
        )
    }
}

/// Created only from an acknowledged Reply::Gesture. The actor owns the lease
/// and selective rollback; no view replays a stale original value on Escape.
pub struct Preview {
    pub gesture: u64,
}
impl Preview {
    pub fn step(&self, commands: Vec<Command>) -> Request {
        Request::new(
            Caller::native(),
            GestureCall::Step {
                gesture: self.gesture,
                body: WriteBody::Commands(commands),
                replace: false,
            },
        )
    }
    pub fn finish(&self, reason: super::colour::Dismiss) -> Request {
        Request::new(
            Caller::native(),
            if reason.cancel() {
                GestureCall::Cancel {
                    gesture: self.gesture,
                }
            } else {
                GestureCall::Commit {
                    gesture: self.gesture,
                }
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Closed,
    Stale,
    MissingLayer,
}
pub type EditPending = Pending<Result<Ack, KernelFailure>>;
/// A transport acknowledgment can carry a refused command. Native consumers
/// must check it before updating recents or closing a successful editor flow.
pub fn accepted(ack: Ack) -> Result<Ack, pm_kernel::tx::KernelError> {
    if let pm_kernel::Reply::Throw(error) = &ack.reply {
        return Err(error.clone());
    }
    Ok(ack)
}
#[derive(Clone)]
pub struct EditingKernel {
    pub handle: KernelHandle,
}
impl EditingKernel {
    /// Nonblocking. Revision validation also runs inside the actor. The actor
    /// currently has no document-id guard for Transact; an atomic session guard
    /// across close/reopen is an explicit kernel-owner integration requirement.
    pub fn submit(&self, target: &Target, request: Request) -> Result<EditPending, AdmissionError> {
        let p = self
            .handle
            .published(&target.project)
            .ok_or(AdmissionError::Closed)?;
        if p.snapshot.doc_id != target.document || p.revision() != target.revision {
            return Err(AdmissionError::Stale);
        }
        let root = p.snapshot.root().ok_or(AdmissionError::Closed)?;
        if pm_kernel::model::find_layer(root, &target.layer).is_none() {
            return Err(AdmissionError::MissingLayer);
        }
        Ok(self.handle.call(&target.project, request))
    }
    pub fn animators(&self, target: &Target) -> Option<Animators> {
        let p = self.handle.published(&target.project)?;
        if p.snapshot.doc_id != target.document {
            return None;
        }
        let root = p.snapshot.root()?;
        let loc = pm_kernel::model::find_layer(root, &target.layer)?;
        let layer = pm_kernel::model::layer_at(root, &loc)?.to_value();
        let items = layer
            .get("d")
            .and_then(|d| d.get("animators"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Animator::from_value).collect())
            .unwrap_or_default();
        let locked = p
            .snapshot
            .layers(loc.comp_id.as_deref())
            .iter()
            .find(|l| l.id == target.layer)?
            .effectively_locked;
        Some(Animators {
            items,
            layer: target.layer.clone(),
            locked,
            ..Animators::default()
        })
    }
}
