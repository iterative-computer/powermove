//! Worker-side panel adapter. All document edits run on pm-kernel; no JS waits,
//! document mirror writes, renderer substitutes, or platform handles live here.
mod fields;
mod light;
pub mod view;
use pm_kernel::{
    Ack, Call, Caller, Command, GestureCall, KernelHandle, Published, QueryMirror, Reply, Request,
    Scene3dCommand, WriteBody,
};
use pm_scene3d::{from_model, to_model};
use serde_json::{Value, json};
use std::sync::Arc;
pub const CHANNEL: &str = "studio3d";
pub const COMMAND: &str = "studio3d.drive";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<pm_scene3d::Error> for Error {
    fn from(e: pm_scene3d::Error) -> Self {
        Self(e.0)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(s: impl Into<String>) -> Error {
    Error(s.into())
}
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Tools {
    pub free: bool,
    pub transform: Transform,
    pub navigation: Navigation,
    pub object_axes: bool,
    pub light: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Transform {
    #[default]
    Move,
    Rotate,
    Scale,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Navigation {
    #[default]
    Select,
    Orbit,
    Pan,
    Zoom,
}
/// One worker-owned session per panel/realm generation. Tickets retain exact
/// revisions and selection in Rust. Retire the session when the realm closes.
pub struct Session {
    kernel: KernelHandle,
    project: String,
    caller: Caller,
    ticket: u64,
    published: Option<Arc<Published>>,
    time: f64,
    pub tools: Tools,
    free_pose: Option<pm_scene3d::navigation::Pose>,
    light_settings: light::Settings,
    gesture: Option<u64>,
    gesture_scope: Option<(u64, Option<String>, Vec<String>)>,
}
impl Session {
    pub fn new(kernel: KernelHandle, project: impl Into<String>, caller: Caller) -> Self {
        Self {
            kernel,
            project: project.into(),
            caller,
            ticket: 0,
            published: None,
            time: 0.,
            tools: Tools::default(),
            free_pose: None,
            light_settings: light::Settings::default(),
            gesture: None,
            gesture_scope: None,
        }
    }
    fn current(&self) -> Result<Arc<Published>> {
        self.kernel
            .published(&self.project)
            .ok_or_else(|| invalid("Project closed"))
    }
    fn checked(&self, ticket: &str) -> Result<Arc<Published>> {
        let p = self.current()?;
        if ticket != self.ticket.to_string()
            || !self
                .published
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, &p))
        {
            return Err(invalid("Stale Studio panel"));
        }
        Ok(p)
    }
    /// Run on the panel worker. Content is evaluated from one immutable kernel
    /// publication, including hidden/inactive selected layers, never static v.
    pub fn publish(
        &mut self,
        time: f64,
        width: f32,
        source: bool,
        overlay: bool,
        tokens: &pm_design::Tokens,
    ) -> Result<Value> {
        if !time.is_finite() || time < 0. || !width.is_finite() || width < 0. {
            return Err(invalid("Invalid Studio viewport or time"));
        }
        let p = self.current()?;
        let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
        let comp_id = p.snapshot.open_comp();
        let comp = pm_kernel::model::comp_node(root, comp_id.as_deref())
            .ok_or_else(|| invalid("No composition"))?;
        let comp = from_model(&comp.to_value())?;
        let layers = comp["layers"]
            .as_array()
            .ok_or_else(|| invalid("No layers"))?;
        let has_3d = layers.iter().any(|l| pm_scene3d::schema::role(l).is_some());
        let selected = if p.selection.layers().len() == 1 {
            layers.iter().find(|l| l["id"] == p.selection.layers()[0])
        } else {
            None
        };
        let mut mirror = QueryMirror::new(&p);
        let inspector = selected
            .map(|l| view::inspector(&p, &comp, l, time, &mut mirror))
            .transpose()?;
        let orthographic = layers
            .iter()
            .find(|l| {
                pm_scene3d::schema::role(l) == Some(pm_scene3d::schema::Role::Camera)
                    && l["on"] != false
            })
            .is_some_and(|l| l["d"]["data"]["camera"]["projection"] == "orthographic");
        let quality = p
            .ui_state
            .viewer
            .get("studioPreview")
            .and_then(pm_model::Value::as_str)
            == Some("quality");
        self.ticket = self
            .ticket
            .checked_add(1)
            .ok_or_else(|| invalid("Ticket exhausted"))?;
        self.time = time;
        let layout = if width < tokens.metrics.viewer_3d_collapse {
            "menu"
        } else if overlay || width < tokens.metrics.viewer_3d_labels {
            "icons"
        } else {
            "labels"
        };
        let value = json!({"ticket":self.ticket.to_string(),"project":self.project,"visible":has_3d&&!source,"layout":layout,"overlay":overlay,"tools":self.tools,"freePose":self.free_pose,
            "quality":quality&&!orthographic,"qualityStored":quality,"orthographic":orthographic,"allowed":{"move":view::allow_transform(&p,"move").is_ok(),"rotate":view::allow_transform(&p,"rotate").is_ok(),"scale":view::allow_transform(&p,"scale").is_ok()},"inspector":inspector,
            "lightPanel":if self.tools.light {light::projection(&p,time,&self.light_settings)?}else{Value::Null},"disclosures":p.ui_state.viewer.get("studioDisclosures").map(from_model).transpose()?.unwrap_or(json!({})),"finishes":pm_scene3d::service::presets(),"add":view::add_menu(),"shots":view::shots()});
        self.published = Some(p);
        Ok(value)
    }
    async fn call(&self, call: impl Into<Call>) -> Result<Ack> {
        let ack = self
            .kernel
            .call(&self.project, Request::new(self.caller, call))
            .await
            .ok_or_else(|| invalid("Kernel stopped"))?
            .map_err(|e| invalid(format!("{e:?}")))?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(invalid(e.to_string()));
        }
        Ok(ack)
    }
    async fn document(&self, p: &Published, label: &str, commands: Vec<Command>) -> Result<()> {
        self.call(Call::Transact {
            label: Some(label.into()),
            body: WriteBody::Commands(commands),
            expected_revision: Some(p.revision()),
        })
        .await?;
        Ok(())
    }
    async fn write_ui(&self, p: &Arc<Published>, ui: pm_kernel::state::UiState) -> Result<()> {
        let state = self.kernel.state();
        if !Arc::ptr_eq(p, &self.current()?) {
            return Err(invalid("Stale Studio UI state"));
        }
        self.kernel
            .state_transaction(pm_kernel::state::StateTransaction {
                expected_revision: Some(state.revision),
                ops: vec![pm_kernel::state::StateOp::Ui {
                    project: self.project.clone(),
                    next: ui,
                }],
            })
            .await
            .ok_or_else(|| invalid("Kernel stopped"))?
            .map_err(|e| invalid(format!("{e:?}")))?;
        Ok(())
    }
    /// Shared native-control and agent-by-ID route. Native binding writes must
    /// enter exactly once. Returns only after actual kernel acknowledgement.
    pub async fn drive(&mut self, ticket: &str, id: &str, arg: Value) -> Result<()> {
        let p = self.checked(ticket)?;
        if let Some(action) = id.strip_prefix("tool.") {
            if !arg.is_null() {
                return Err(invalid("Unexpected tool argument"));
            }
            match action {
                "camera" => self.tools.free = false,
                "free" => self.tools.free = true,
                "move" | "rotate" | "scale" => {
                    view::allow_transform(&p, action)?;
                    self.tools.transform = match action {
                        "rotate" => Transform::Rotate,
                        "scale" => Transform::Scale,
                        _ => Transform::Move,
                    };
                    self.tools.navigation = Navigation::Select;
                    self.tools.light = false;
                }
                "orbit" | "pan" | "zoom" => {
                    self.tools.navigation = match action {
                        "orbit" => Navigation::Orbit,
                        "pan" => Navigation::Pan,
                        _ => Navigation::Zoom,
                    };
                    self.tools.light = false;
                }
                "light" => {
                    self.tools.light = !self.tools.light;
                    self.tools.navigation = Navigation::Select;
                }
                "axes" => self.tools.object_axes = !self.tools.object_axes,
                _ => return Err(invalid("Unknown Studio tool")),
            }
            // Local tool changes invalidate tickets even without document edits.
            self.published = None;
            return Ok(());
        }
        if ["light.aim", "light.add", "light.kind"].contains(&id) {
            match id {
                "light.aim" => {
                    self.light_settings.aim = match arg.as_str() {
                        Some("reflect") => "reflect",
                        Some("direct") => "direct",
                        Some("orbit") => "orbit",
                        _ => return Err(invalid("Unknown light aim")),
                    }
                }
                "light.add" => {
                    self.light_settings.add =
                        arg.as_bool().ok_or_else(|| invalid("Expected Add state"))?
                }
                _ => {
                    self.light_settings.kind = match arg.as_str() {
                        Some("area") => "area",
                        Some("spot") => "spot",
                        Some("point") => "point",
                        Some("sun") => "sun",
                        _ => return Err(invalid("Unknown light kind")),
                    };
                    self.light_settings.add = true;
                }
            }
            self.published = None;
            return Ok(());
        }
        if let Some(group) = id.strip_prefix("disclosure.") {
            if ![
                "Model",
                "Material",
                "Coat and Glass",
                "Surface",
                "Emission",
                "Texture Maps",
                "Light",
                "Camera",
                "Shot",
                "Environment",
            ]
            .contains(&group)
                || !arg.is_boolean()
            {
                return Err(invalid("Unknown Studio disclosure"));
            }
            let mut ui = p.ui_state.clone();
            let viewer = ui
                .viewer
                .as_object_mut()
                .ok_or_else(|| invalid("Invalid viewer state"))?;
            let mut d = viewer
                .get("studioDisclosures")
                .map(from_model)
                .transpose()?
                .unwrap_or(json!({}));
            d[group] = arg;
            viewer.insert("studioDisclosures", to_model(&d));
            self.write_ui(&p, ui).await?;
            return Ok(());
        }
        if id == "preview" {
            let value = arg
                .as_str()
                .filter(|s| ["fast", "quality"].contains(s))
                .ok_or_else(|| invalid("Invalid preview"))?;
            if value == "quality" && view::orthographic(&p)? {
                return Err(invalid("Orthographic cameras draw in Fast."));
            }
            let mut ui = p.ui_state.clone();
            ui.viewer
                .as_object_mut()
                .ok_or_else(|| invalid("Invalid viewer state"))?
                .insert("studioPreview", pm_model::Value::from(value));
            self.write_ui(&p, ui).await?;
            return Ok(());
        }
        let (label, commands) = view::commands(&p, self.time, id, arg)?;
        self.document(&p, &label, commands).await
    }
    /// Geometry-dependent operations prepare against the exact stamped snapshot
    /// with real loaded asset capabilities. An absent asset remains an error.
    pub async fn geometry(
        &mut self,
        ticket: &str,
        operation: &str,
        assets: &dyn pm_scene3d::geometry::AssetGeometry,
    ) -> Result<()> {
        let p = self.checked(ticket)?;
        let label = match operation {
            "drop_to_floor" => "Drop to Floor",
            "auto_studio" => "Set Up Studio",
            _ => return Err(invalid("Unknown Studio geometry command")),
        };
        let args = json!({"operation":operation});
        let geometry = pm_kernel::commands::scene3d::prepare_geometry(
            &p.snapshot,
            p.snapshot.open_comp(),
            &to_model(&args),
            p.selection.layers(),
            self.time,
            assets,
        )
        .map_err(|e| invalid(e.to_string()))?;
        let cmd = Scene3dCommand::ApplyPrepared {
            comp: p.snapshot.open_comp(),
            args: to_model(&args),
            selection: p.selection.layers().to_vec(),
            time: self.time,
            autokey: false,
            geometry,
        };
        self.document(&p, label, vec![cmd.into()]).await
    }
    /// Import workers admit selected bytes into Archive before calling this.
    /// Undo never deletes that archive. No UI or synchronous file discovery.
    pub async fn import(
        &mut self,
        ticket: &str,
        asset: String,
        layer: String,
        receipt: Arc<pm_scene3d::import::archive::Receipt>,
    ) -> Result<()> {
        let p = self.checked(ticket)?;
        let label = format!("Import {}", receipt.primary().name());
        let cmd = Scene3dCommand::Import {
            comp: p.snapshot.open_comp(),
            asset_id: asset,
            layer_id: layer,
            receipt,
            time: self.time,
            setup_studio: true,
        };
        self.document(&p, &label, vec![cmd.into()]).await
    }
    /// Sliders, wells and picked light/gizmo gestures use the actor's lease.
    /// Only typed Studio commands validated by the same panel route are allowed.
    pub async fn begin(&mut self, ticket: &str, label: &str) -> Result<()> {
        let p = self.checked(ticket)?;
        if self.gesture.is_some() {
            return Err(invalid("Studio gesture already active"));
        }
        if !view::gesture_label(label) {
            return Err(invalid("Unknown Studio gesture"));
        }
        let ack = self
            .call(GestureCall::Begin {
                label: label.into(),
            })
            .await?;
        let Reply::Gesture(id) = ack.reply else {
            return Err(invalid("Missing gesture acknowledgement"));
        };
        self.gesture_scope = Some((
            p.snapshot.doc_id,
            p.snapshot.open_comp(),
            p.selection.layers().to_vec(),
        ));
        self.gesture = Some(id);
        Ok(())
    }
    pub async fn step(&mut self, id: &str, arg: Value) -> Result<()> {
        let gesture = self.gesture.ok_or_else(|| invalid("No Studio gesture"))?;
        let p = self.current()?;
        let scope = (
            p.snapshot.doc_id,
            p.snapshot.open_comp(),
            p.selection.layers().to_vec(),
        );
        if self.gesture_scope.as_ref() != Some(&scope) {
            return Err(invalid("Studio gesture target changed"));
        }
        if !id.starts_with("field.") && !id.starts_with("light.adjust.") {
            return Err(invalid(
                "Only field changes may stream through this gesture",
            ));
        }
        let (_, commands) = view::commands(&p, self.time, id, arg)?;
        self.call(GestureCall::Step {
            gesture,
            body: WriteBody::Commands(commands),
            replace: true,
        })
        .await?;
        Ok(())
    }
    pub async fn end(&mut self, cancel: bool) -> Result<()> {
        let gesture = self.gesture.ok_or_else(|| invalid("No Studio gesture"))?;
        self.call(if cancel {
            GestureCall::Cancel { gesture }
        } else {
            GestureCall::Commit { gesture }
        })
        .await?;
        self.gesture = None;
        self.gesture_scope = None;
        self.published = None;
        Ok(())
    }
    /// Route a v4 command on the serialized panel worker. File selection and
    /// durable texture admission are separate explicit native capabilities.
    pub async fn dispatch(
        &mut self,
        request: Value,
        assets: &dyn pm_scene3d::geometry::AssetGeometry,
    ) -> Result<()> {
        let map = request
            .as_object()
            .ok_or_else(|| invalid("Invalid Studio request"))?;
        if map.len() != 3 {
            return Err(invalid("Invalid Studio request fields"));
        }
        let ticket = request["ticket"]
            .as_str()
            .ok_or_else(|| invalid("Missing Studio ticket"))?;
        let id = request["element"]
            .as_str()
            .ok_or_else(|| invalid("Missing Studio element"))?;
        let arg = request
            .get("argument")
            .ok_or_else(|| invalid("Missing Studio argument"))?
            .clone();
        match id {
            "geometry" => {
                self.geometry(
                    ticket,
                    arg.as_str()
                        .ok_or_else(|| invalid("Invalid geometry operation"))?,
                    assets,
                )
                .await
            }
            "frame" => {
                if !arg.is_null() {
                    return Err(invalid("Unexpected Frame argument"));
                }
                self.frame(ticket, assets)
            }
            _ => self.drive(ticket, id, arg).await,
        }
    }
    /// Geometry and lens are evaluated from the stamped document. Frame changes
    /// only Free View and never authors shot channels or an Undo entry.
    pub fn frame(
        &mut self,
        ticket: &str,
        assets: &dyn pm_scene3d::geometry::AssetGeometry,
    ) -> Result<()> {
        use pm_scene3d::navigation::{Bounds, Lens, Pose};
        let p = self.checked(ticket)?;
        let comp = p.snapshot.open_comp();
        let geometry = pm_kernel::commands::scene3d::prepare_geometry(
            &p.snapshot,
            comp.clone(),
            &to_model(&json!({"operation":"auto_studio"})),
            &[],
            self.time,
            assets,
        )
        .map_err(|e| invalid(e.to_string()))?;
        let resolved =
            pm_kernel::commands::scene3d::resolve_scene(&p.snapshot, comp.as_deref(), self.time)
                .map_err(|e| invalid(e.to_string()))?;
        let root = p.snapshot.root().ok_or_else(|| invalid("No project"))?;
        let c = pm_kernel::model::comp_node(root, comp.as_deref())
            .ok_or_else(|| invalid("No composition"))?;
        let c = from_model(&c.to_value())?;
        let selected = p.selection.layers();
        let layers = c["layers"].as_array().ok_or_else(|| invalid("No layers"))?;
        let mut b: Option<Bounds> = None;
        for (id, v) in geometry
            .bounds
            .as_object()
            .ok_or_else(|| invalid("No geometry bounds"))?
        {
            let include = selected.is_empty()
                || selected.contains(id)
                || layers.iter().find(|l| l["id"] == *id).is_some_and(|l| {
                    selected
                        .iter()
                        .any(|g| pm_scene3d::geometry::descendants(l, g, layers))
                });
            if !include {
                continue;
            }
            let v = v
                .as_array()
                .filter(|a| a.len() == 6)
                .ok_or_else(|| invalid("Invalid geometry bounds"))?;
            let mut a = [0.; 6];
            for (i, n) in v.iter().enumerate() {
                a[i] = n
                    .as_f64()
                    .ok_or_else(|| invalid("Invalid geometry bounds"))?;
            }
            let next = Bounds {
                min: [a[0], a[1], a[2]],
                max: [a[3], a[4], a[5]],
            };
            next.validate()?;
            b = Some(match b {
                None => next,
                Some(old) => Bounds {
                    min: std::array::from_fn(|i| old.min[i].min(next.min[i])),
                    max: std::array::from_fn(|i| old.max[i].max(next.max[i])),
                },
            });
        }
        let b = b.ok_or_else(|| invalid("No model geometry to frame"))?;
        let cam = &resolved.scene["camera"];
        let n = |key: &str| {
            cam["p"][key]["v"]
                .as_f64()
                .ok_or_else(|| invalid("Invalid camera lens"))
        };
        let w = c["w"]
            .as_f64()
            .ok_or_else(|| invalid("Invalid composition width"))?;
        let h = c["h"]
            .as_f64()
            .ok_or_else(|| invalid("Invalid composition height"))?;
        let lens = if cam["projection"] == "orthographic" {
            Lens::Orthographic {
                width: w,
                height: h,
            }
        } else {
            Lens::Perspective {
                fov: n("fov")?,
                aspect: w / h,
                near: n("near")?,
                far: n("far")?,
            }
        };
        lens.validate()?;
        let start = self.free_pose.unwrap_or(Pose {
            position: resolved
                .camera_position
                .unwrap_or([n("x")?, n("y")?, n("z")?]),
            target: resolved
                .camera_id
                .as_ref()
                .and_then(|id| resolved.targets.get(id))
                .copied()
                .unwrap_or([n("targetX")?, n("targetY")?, n("targetZ")?]),
            zoom: n("zoom")?,
        });
        start.validate()?;
        self.free_pose = Some(pm_scene3d::navigation::frame(start, b, lens));
        self.tools.free = true;
        self.published = None;
        Ok(())
    }
    /// Must be awaited by worker lifecycle cleanup before retiring this session.
    pub async fn dispose(&mut self) -> Result<()> {
        if self.gesture.is_some() {
            self.end(true).await?;
        }
        self.published = None;
        Ok(())
    }
}
