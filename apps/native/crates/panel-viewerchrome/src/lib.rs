//! Viewer chrome MODEL only. No renderer, JS, platform event loop, or UI copy.
//! Plans are inspectable; only the kernel owns document/state/history writes.
pub mod geometry;
pub mod input;
pub mod inspection;
pub mod source;
pub mod state;
pub mod transport;
pub mod v4;

pub use geometry::*;
pub use inspection::*;
use pm_kernel::{
    Call, Caller, Command, CompCommand, KernelHandle, Published, QueryMirror, Request,
    SelectionCall, Snapshot, WriteBody, model,
    state::{StateOp, StateTransaction, UiState},
};
use pm_model::{Node, NodeKind, Value};
pub use source::*;
pub use state::*;
use std::sync::Arc;
pub use transport::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Invalid(&'static str),
    Closed,
    Stale,
    Query(pm_kernel::QueryError),
    Unsupported(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub(crate) fn finite(value: f64) -> Result<f64, Error> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::Invalid("finite number"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub project: String,
    pub document: u64,
    pub revision: u64,
    pub state_revision: u64,
    pub selection: pm_kernel::Selection,
}
#[derive(Clone, Debug)]
pub struct ViewModel {
    pub stamp: Stamp,
    pub chrome: Chrome,
    pub transport: Transport,
    pub selection: pm_kernel::Selection,
    pub composition: Option<String>,
    pub size: Size,
    ui: UiState,
    snapshot: Snapshot,
}
impl ViewModel {
    /// Caller supplies a coherent publication and state revision. Controller::read
    /// obtains them together and refuses if a publication changed during the read.
    pub fn read(published: &Published, state_revision: u64) -> Result<Self, Error> {
        let snapshot = published.snapshot.clone();
        let composition = snapshot.open_comp();
        let root = snapshot.root().ok_or(Error::Invalid("project"))?;
        let comp =
            model::comp_node(root, composition.as_deref()).ok_or(Error::Invalid("composition"))?;
        let field = |key| -> Result<f64, Error> {
            match comp.get(key).map(Node::kind) {
                Some(NodeKind::Number(n)) => finite(*n),
                _ => Err(Error::Invalid("composition number")),
            }
        };
        let size = Size {
            width: field("w")?,
            height: field("h")?,
        }
        .checked()?;
        let duration = field("dur")?;
        let work = match comp.get("work").map(Node::kind) {
            Some(NodeKind::Array(a)) if a.len() == 2 => (node_number(&a[0])?, node_number(&a[1])?),
            None => (0.0, duration),
            _ => return Err(Error::Invalid("work area")),
        };
        let ui = published.ui_state.clone();
        Ok(Self {
            stamp: Stamp {
                project: published.project.clone(),
                document: snapshot.doc_id,
                revision: snapshot.revision,
                state_revision,
                selection: published.selection.clone(),
            },
            chrome: Chrome::read(&ui)?,
            transport: Transport::read(&ui, field("fps")?, duration, work)?,
            selection: published.selection.clone(),
            composition,
            size,
            ui,
            snapshot,
        })
    }
    pub fn viewport(&self, stage: Size, fit_inset: f64) -> Result<Viewport, Error> {
        Viewport {
            stage,
            composition: self.size,
            fit_inset,
        }
        .checked()
    }
    /// Bounded ID inspection for agents. No row list or per-frame walk over all
    /// layers: chrome is constant-size even for 60k+ layers.
    pub fn selection_page(&self, start: usize, limit: usize) -> &[String] {
        let ids = self.selection.layers();
        let start = start.min(ids.len());
        &ids[start..start.saturating_add(limit.min(1024)).min(ids.len())]
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn plan(&self, action: Action, caller: Caller) -> Result<Plan, Error> {
        let mut ui = self.ui.clone();
        match action {
            Action::SetZoom(zoom) => {
                Chrome::write_navigation(&mut ui, self.chrome.navigation.set_zoom(zoom)?)?
            }
            Action::ZoomBy(factor, viewport) => {
                if finite(factor)? <= 0.0 {
                    return Err(Error::Invalid("zoom factor"));
                }
                if viewport.composition != self.size {
                    return Err(Error::Invalid("viewport composition"));
                }
                Chrome::write_navigation(
                    &mut ui,
                    self.chrome
                        .navigation
                        .set_zoom(self.chrome.navigation.shown(viewport)? * factor)?,
                )?;
            }
            Action::Fit => Chrome::write_navigation(&mut ui, self.chrome.navigation.fit())?,
            Action::Navigate(navigation) => Chrome::write_navigation(&mut ui, navigation)?,
            Action::Quality(quality) => Chrome::write_quality(&mut ui, quality)?,
            Action::StudioTier(tier) => put(&mut ui.viewer, "studioTier", Value::from(tier))?,
            Action::ToggleControls => put(
                &mut ui.viewer,
                "showControls",
                Value::Bool(!self.chrome.show_controls),
            )?,
            Action::ToggleGuides => OverlayState::toggle(&mut ui, false)?,
            Action::ToggleSafeAreas => OverlayState::toggle(&mut ui, true)?,
            Action::Transport(action) => {
                // Playing intent is handed to the engine host; publish only its
                // observed state after readiness/audio/decoder acceptance.
                // Validate before dispatching, including invalid seeks during playback.
                self.transport.apply(action, &mut ui)?;
                if matches!(action, TransportAction::Toggle | TransportAction::Pause)
                    || self.transport.playing
                {
                    return Ok(self.host(HostAction::Transport(action)));
                }
            }
            Action::WorkIn | Action::WorkOut | Action::ResetWork => {
                let command = match action {
                    Action::WorkIn => CompCommand::SetWorkArea {
                        comp: self.composition.clone(),
                        start: Some(
                            self.transport
                                .time
                                .min(self.transport.work.1 - 1.0 / self.transport.fps),
                        ),
                        end: None,
                    },
                    Action::WorkOut => CompCommand::SetWorkArea {
                        comp: self.composition.clone(),
                        start: None,
                        end: Some(
                            self.transport
                                .time
                                .max(self.transport.work.0 + 1.0 / self.transport.fps),
                        ),
                    },
                    _ => CompCommand::ResetWorkArea {
                        comp: self.composition.clone(),
                    },
                };
                return Ok(Plan {
                    stamp: self.stamp.clone(),
                    effect: Effect::Document(
                        Request::new(
                            caller,
                            Call::Transact {
                                label: Some("Work area".into()),
                                body: WriteBody::Commands(vec![Command::Comp(command)]),
                                expected_revision: Some(self.stamp.revision),
                            },
                        )
                        .with_document(self.stamp.document),
                    ),
                });
            }
            Action::Select { ids, mode } => {
                let valid = model::layer_ids(
                    self.snapshot.root().ok_or(Error::Invalid("project"))?,
                    self.composition.as_deref(),
                );
                let mut incoming = std::collections::HashSet::new();
                let ids: Vec<_> = ids
                    .into_iter()
                    .filter(|id| incoming.insert(id.clone()))
                    .collect();
                if ids.iter().any(|id| !valid.contains(id)) {
                    return Err(Error::Invalid("layer id"));
                }
                let next = if mode == SelectionMode::Toggle {
                    let mut selected: std::collections::HashSet<_> =
                        self.selection.layers().iter().cloned().collect();
                    let mut appended = vec![];
                    for id in ids {
                        if !selected.remove(&id) {
                            selected.insert(id.clone());
                            appended.push(id);
                        }
                    }
                    self.selection
                        .layers()
                        .iter()
                        .cloned()
                        .chain(appended)
                        .filter(|id| selected.contains(id))
                        .collect()
                } else {
                    ids
                };
                return Ok(Plan {
                    stamp: self.stamp.clone(),
                    effect: Effect::Document(
                        Request::new(
                            caller,
                            SelectionCall::Layers {
                                ids: next,
                                add: mode == SelectionMode::Add,
                            },
                        )
                        .with_document(self.stamp.document),
                    ),
                });
            }
            Action::Host(action) => return Ok(self.host(action)),
        }
        Ok(Plan {
            stamp: self.stamp.clone(),
            effect: Effect::State(StateTransaction {
                expected_revision: Some(self.stamp.state_revision),
                ops: vec![StateOp::Ui {
                    project: self.stamp.project.clone(),
                    next: ui,
                }],
            }),
        })
    }
    fn host(&self, action: HostAction) -> Plan {
        Plan {
            stamp: self.stamp.clone(),
            effect: Effect::Host(action),
        }
    }
    /// Same stable element IDs are used by pointer controls and MCP adapters.
    /// Unknown IDs/arguments fail; callers cannot inject arbitrary JS or paths.
    pub fn drive(
        &self,
        id: &str,
        argument: Value,
        viewport: Option<Viewport>,
        caller: Caller,
    ) -> Result<Plan, Error> {
        let no_arg = || {
            if argument == Value::Null {
                Ok(())
            } else {
                Err(Error::Invalid("unexpected argument"))
            }
        };
        let action = match id {
            "viewer.zoom" => {
                Action::SetZoom(argument.as_f64().ok_or(Error::Invalid("zoom argument"))?)
            }
            "viewer.zoom.in" | "viewer.zoom.out" => {
                no_arg()?;
                Action::ZoomBy(
                    if id.ends_with(".in") { 1.25 } else { 0.8 },
                    viewport.ok_or(Error::Invalid("viewport required"))?,
                )
            }
            "viewer.fit" | "viewer.recovery.fit" => {
                no_arg()?;
                Action::Fit
            }
            "viewer.quality" => Action::Quality(Quality::parse(
                argument
                    .as_str()
                    .ok_or(Error::Invalid("quality argument"))?,
            )?),
            "viewer.studio.tier" => Action::StudioTier(match argument.as_str() {
                Some("live") => "live",
                Some("refine") => "refine",
                _ => return Err(Error::Invalid("studio tier")),
            }),
            "viewer.controls" => {
                no_arg()?;
                Action::ToggleControls
            }
            "viewer.guides" => {
                no_arg()?;
                Action::ToggleGuides
            }
            "viewer.safe-areas" => {
                no_arg()?;
                Action::ToggleSafeAreas
            }
            "viewer.transport.play" => {
                no_arg()?;
                Action::Transport(TransportAction::Toggle)
            }
            "viewer.transport.seek" => Action::Transport(TransportAction::Seek(
                argument.as_f64().ok_or(Error::Invalid("seek argument"))?,
            )),
            "viewer.transport.step" => {
                let n = argument.as_f64().ok_or(Error::Invalid("step argument"))?;
                if !n.is_finite()
                    || n.fract() != 0.0
                    || n < f64::from(i32::MIN)
                    || n > f64::from(i32::MAX)
                {
                    return Err(Error::Invalid("step argument"));
                }
                Action::Transport(TransportAction::Step(n as i32))
            }
            "viewer.work.in" => {
                no_arg()?;
                Action::WorkIn
            }
            "viewer.work.out" => {
                no_arg()?;
                Action::WorkOut
            }
            "viewer.work.reset" => {
                no_arg()?;
                Action::ResetWork
            }
            "viewer.source.show" | "viewer.source.toggle" => {
                let asset = argument
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or(Error::Invalid("source id"))?;
                let root = self.snapshot.root().ok_or(Error::Invalid("project"))?;
                if root.get("assets").and_then(|a| a.get(asset)).is_none() {
                    return Err(Error::Invalid("source asset"));
                }
                Action::Host(HostAction::Source(if id.ends_with(".show") {
                    SourceCommand::Show(asset.into())
                } else {
                    SourceCommand::Toggle(asset.into())
                }))
            }
            "viewer.source.play" => {
                no_arg()?;
                Action::Host(HostAction::Source(SourceCommand::PlayPause))
            }
            "viewer.source.close" => {
                no_arg()?;
                Action::Host(HostAction::Source(SourceCommand::Close))
            }
            "viewer.source.seek" => {
                let ratio = finite(argument.as_f64().ok_or(Error::Invalid("source ratio"))?)?;
                Action::Host(HostAction::Source(SourceCommand::SeekRatio(
                    ratio.clamp(0.0, 1.0),
                )))
            }
            "viewer.context" => {
                let layer = match &argument {
                    Value::Null => None,
                    Value::String(s) => Some(s.to_string_lossy()),
                    _ => return Err(Error::Invalid("context target")),
                };
                return self.context_menu(layer);
            }
            "viewer.selection" | "viewer.selection.add" | "viewer.selection.toggle" => {
                let ids = argument
                    .as_array()
                    .ok_or(Error::Invalid("selection ids"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or(Error::Invalid("layer id"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Action::Select {
                    ids,
                    mode: if id.ends_with(".add") {
                        SelectionMode::Add
                    } else if id.ends_with(".toggle") {
                        SelectionMode::Toggle
                    } else {
                        SelectionMode::Replace
                    },
                }
            }
            "viewer.scene.transform" => Action::Host(HostAction::Scene(SceneCommand::Transform(
                match argument.as_str() {
                    Some("translate") => TransformMode::Translate,
                    Some("rotate") => TransformMode::Rotate,
                    Some("scale") => TransformMode::Scale,
                    _ => return Err(Error::Invalid("transform mode")),
                },
            ))),
            "viewer.scene.axes" => Action::Host(HostAction::Scene(SceneCommand::Axes(
                match argument.as_str() {
                    Some("world") => AxisSpace::World,
                    Some("local") => AxisSpace::Local,
                    _ => return Err(Error::Invalid("axis space")),
                },
            ))),
            "viewer.scene.view" => Action::Host(HostAction::Scene(SceneCommand::View(
                match argument.as_str() {
                    Some("camera") => SceneView::Camera,
                    Some("editor") => SceneView::Editor,
                    _ => return Err(Error::Invalid("scene view")),
                },
            ))),
            "viewer.scene.navigation" => Action::Host(HostAction::Scene(match argument.as_str() {
                Some("select") => SceneCommand::Navigate(NavigationMode::Select),
                Some("orbit") => SceneCommand::Navigate(NavigationMode::Orbit),
                Some("pan") => SceneCommand::Navigate(NavigationMode::Pan),
                Some("dolly") => SceneCommand::Navigate(NavigationMode::Dolly),
                Some("frame-selection") => SceneCommand::FrameSelection,
                Some("frame-all") => SceneCommand::FrameAll,
                _ => return Err(Error::Invalid("navigation mode")),
            })),
            _ => return Err(Error::Unsupported(id.into())),
        };
        self.plan(action, caller)
    }
    /// Engine query errors remain errors (fonts/models/host unavailable is never
    /// interpreted as empty canvas). Context-menu picking includes locked layers.
    pub fn pick(
        &self,
        mirror: &mut ViewerQueries,
        point: Point,
        context_menu: bool,
    ) -> Result<Option<String>, Error> {
        point.checked()?;
        if mirror.document != self.stamp.document || mirror.mirror.revision() != self.stamp.revision
        {
            return Err(Error::Stale);
        }
        mirror
            .mirror
            .queries()
            .pick(
                self.composition.as_deref(),
                point.x,
                point.y,
                self.transport.time,
                &pm_kernel::PickOptions {
                    include_locked: context_menu,
                },
            )
            .map(|hit| hit.layer_id)
            .map_err(Error::Query)
    }
    /// Resolve raw engine hit IDs with tag group-entry semantics. Cmd/Ctrl is
    /// deep selection; double-click descends one selected group at a time.
    pub fn pick_target(
        &self,
        hit: Option<&str>,
        deep: bool,
        drill: bool,
    ) -> Result<Option<String>, Error> {
        let Some(id) = hit else {
            return Ok(None);
        };
        let root = self.snapshot.root().ok_or(Error::Invalid("project"))?;
        let comp = model::comp_node(root, self.composition.as_deref())
            .ok_or(Error::Invalid("composition"))?;
        let loc = model::find_layer(root, id).ok_or(Error::Invalid("layer id"))?;
        if loc.comp_id != self.composition {
            return Err(Error::Invalid("layer composition"));
        }
        let layer = model::layer_at(root, &loc).ok_or(Error::Invalid("layer id"))?;
        let id_of = |node: &Node| match node.get("id").map(Node::kind) {
            Some(NodeKind::String(s)) => s.as_str().map(str::to_owned),
            _ => None,
        };
        let mut chain: Vec<_> = model::group_ancestors(comp, layer)
            .into_iter()
            .rev()
            .filter_map(id_of)
            .collect();
        if drill {
            chain.push(id.into());
            let index = chain.iter().rposition(|id| self.selection.contains(id));
            return Ok(index.and_then(|index| chain.get(index + 1)).cloned());
        }
        if deep {
            return Ok(Some(id.into()));
        }
        let mut entered = std::collections::HashSet::new();
        for selected in self.selection.layers() {
            if let Some(loc) = model::find_layer(root, selected)
                && let Some(layer) = model::layer_at(root, &loc)
            {
                entered.extend(
                    model::group_ancestors(comp, layer)
                        .into_iter()
                        .filter_map(id_of),
                );
            }
        }
        Ok(Some(
            chain
                .into_iter()
                .find(|group| !entered.contains(group))
                .unwrap_or_else(|| id.into()),
        ))
    }
    pub fn context_menu(&self, layer: Option<String>) -> Result<Plan, Error> {
        if let Some(id) = &layer {
            let valid = model::layer_ids(
                self.snapshot.root().ok_or(Error::Invalid("project"))?,
                self.composition.as_deref(),
            );
            if !valid.contains(id) {
                return Err(Error::Invalid("layer id"));
            }
        }
        Ok(self.host(HostAction::ContextMenu {
            layer,
            time: self.transport.time,
        }))
    }
}
/// A query mirror bound to a document lifetime, not merely a numeric revision.
pub struct ViewerQueries {
    document: u64,
    mirror: QueryMirror,
}
impl ViewerQueries {
    pub fn new(published: &Published) -> Self {
        Self {
            document: published.snapshot.doc_id,
            mirror: QueryMirror::new(published),
        }
    }
    pub fn sync(&mut self, published: &Published) {
        self.document = published.snapshot.doc_id;
        self.mirror.sync(published);
    }
}
fn node_number(node: &Node) -> Result<f64, Error> {
    match node.kind() {
        NodeKind::Number(n) => finite(*n),
        _ => Err(Error::Invalid("number")),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Replace,
    Add,
    Toggle,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SetZoom(f64),
    ZoomBy(f64, Viewport),
    Fit,
    Navigate(Navigation),
    Quality(Quality),
    /// Studio 3D preview tier (`live` = Fast, `refine` = Quality), 3d-studio.md §2.
    StudioTier(&'static str),
    ToggleControls,
    ToggleGuides,
    ToggleSafeAreas,
    Transport(TransportAction),
    WorkIn,
    WorkOut,
    ResetWork,
    Select {
        ids: Vec<String>,
        mode: SelectionMode,
    },
    Host(HostAction),
}
#[derive(Clone, Debug, PartialEq)]
pub enum HostAction {
    Transport(TransportAction),
    /// The host gathers current extension contributions. No built-in menu copy
    /// or invented Fit/Settings rows are hardcoded here.
    ContextMenu {
        layer: Option<String>,
        time: f64,
    },
    Command {
        id: String,
        args: Vec<Value>,
    },
    Source(SourceCommand),
    Scene(SceneCommand),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneCommand {
    Transform(TransformMode),
    Axes(AxisSpace),
    View(SceneView),
    Navigate(NavigationMode),
    FrameSelection,
    FrameAll,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformMode {
    Translate,
    Rotate,
    Scale,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisSpace {
    World,
    Local,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneView {
    Camera,
    Editor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationMode {
    Select,
    Orbit,
    Pan,
    Dolly,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneState {
    pub transform: TransformMode,
    pub axes: AxisSpace,
    pub view: SceneView,
    pub navigation: NavigationMode,
}
impl SceneState {
    /// GizmoToolbar.svelte:20–24. Framing is an action, not a navigation mode.
    pub fn apply(&mut self, command: &SceneCommand) {
        match *command {
            SceneCommand::Transform(mode) => {
                self.navigation = NavigationMode::Select;
                self.transform = mode;
            }
            SceneCommand::Axes(space) => self.axes = space,
            SceneCommand::View(view) => self.view = view,
            SceneCommand::Navigate(mode) => self.navigation = mode,
            SceneCommand::FrameSelection | SceneCommand::FrameAll => {}
        }
    }
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub stamp: Stamp,
    pub effect: Effect,
}
#[derive(Clone, Debug)]
pub enum Effect {
    State(StateTransaction),
    Document(Request),
    Host(HostAction),
}
/// Platform work is enqueued, never synchronously awaited on the UI thread.
/// Implementations must reject stale stamps again at consumption/presentation.
pub trait ViewerHost {
    fn enqueue(&mut self, stamp: Stamp, action: HostAction) -> Result<(), Error>;
}
/// Linux test double records requests; it deliberately claims no media/render success.
#[derive(Default, Debug)]
pub struct InspectHost {
    pub requests: Vec<(Stamp, HostAction)>,
}
impl ViewerHost for InspectHost {
    fn enqueue(&mut self, stamp: Stamp, action: HostAction) -> Result<(), Error> {
        self.requests.push((stamp, action));
        Ok(())
    }
}
#[derive(Clone)]
pub struct Controller {
    kernel: KernelHandle,
    project: String,
}
pub enum Submission {
    State(pm_kernel::Pending<Result<pm_kernel::state::StateAck, pm_kernel::KernelFailure>>),
    Document(pm_kernel::Pending<Result<pm_kernel::Ack, pm_kernel::KernelFailure>>),
    Enqueued,
}
impl Controller {
    pub fn new(kernel: KernelHandle, project: impl Into<String>) -> Self {
        Self {
            kernel,
            project: project.into(),
        }
    }
    pub fn read(&self) -> Result<ViewModel, Error> {
        let before = self.kernel.state();
        let published = self.kernel.published(&self.project).ok_or(Error::Closed)?;
        let after = self.kernel.state();
        let latest = self.kernel.published(&self.project).ok_or(Error::Closed)?;
        if !Arc::ptr_eq(&published, &latest)
            || before.revision != after.revision
            || after
                .ui
                .get(&self.project)
                .is_some_and(|ui| ui != &published.ui_state)
        {
            return Err(Error::Stale);
        }
        ViewModel::read(&published, after.revision)
    }
    pub fn submit(&self, plan: Plan, host: &mut impl ViewerHost) -> Result<Submission, Error> {
        let current = self.read()?;
        if current.stamp != plan.stamp {
            return Err(Error::Stale);
        }
        Ok(match plan.effect {
            Effect::State(tx) => Submission::State(self.kernel.state_transaction(tx)),
            Effect::Document(request) => {
                Submission::Document(self.kernel.call(&self.project, request))
            }
            Effect::Host(action) => {
                host.enqueue(plan.stamp, action)?;
                Submission::Enqueued
            }
        })
    }
    pub fn published(&self) -> Option<Arc<Published>> {
        self.kernel.published(&self.project)
    }
}
