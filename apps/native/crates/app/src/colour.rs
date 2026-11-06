//! Colour tools for pros: camera log readings, `.cube` LUTs on a layer or
//! adjustment layer, and the Scopes panel. Commands go through the kernel;
//! file reads and rasterizing run on background threads.
use crate::session::Controller;
use gpui::{AppContext, Context, Entity, prelude::*};
use pm_kernel::{Call, Caller, Command, Published, Request, WriteBody};
use pm_model::{Map, Value};
use pm_render::scopes::{ScopeFrame, ScopeSignal};
use pm_shell::scopes::{Counts, Kind, ScopesView, raster};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

/// A camera's log curve with the gamut it records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraLog {
    pub label: &'static str,
    pub transfer: &'static str,
    pub primaries: &'static str,
}

/// Interpret Footage's camera readings, in menu order.
pub const CAMERA_LOGS: [CameraLog; 6] = [
    CameraLog {
        label: "Apple Log",
        transfer: "appleLog",
        primaries: "bt2020",
    },
    CameraLog {
        label: "S-Log3 (S-Gamut3.Cine)",
        transfer: "sLog3",
        primaries: "sGamut3Cine",
    },
    CameraLog {
        label: "S-Log3 (S-Gamut3)",
        transfer: "sLog3",
        primaries: "sGamut3",
    },
    CameraLog {
        label: "V-Log (V-Gamut)",
        transfer: "vLog",
        primaries: "vGamut",
    },
    CameraLog {
        label: "LogC3 (AWG3)",
        transfer: "logC3",
        primaries: "awgGen3",
    },
    CameraLog {
        label: "LogC4 (AWG4)",
        transfer: "logC4",
        primaries: "awgGen4",
    },
];

/// The video or image asset a layer shows.
fn layer_asset(p: &Published, layer: &str) -> Option<String> {
    let root = p.snapshot.root()?;
    let at = pm_kernel::model::find_layer(root, layer)?;
    let node = pm_kernel::model::layer_at(root, &at)?;
    let value = node.get("d")?.get("asset")?.to_value();
    value.as_str().map(str::to_owned)
}

/// Reads the selected clip's asset with `log`, or as the file says with `None`.
pub fn interpret_command(
    p: &Published,
    layer: &str,
    log: Option<CameraLog>,
) -> Result<Command, String> {
    let asset_id = layer_asset(p, layer).ok_or("Select a video clip to interpret")?;
    let root = p.snapshot.root().ok_or("Project closed")?;
    let saved = root
        .get("assets")
        .and_then(|assets| assets.get(&asset_id))
        .and_then(|asset| asset.get("interpret"))
        .map(pm_model::Node::to_value);
    let mut interpret = saved
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_else(Map::new);
    match log {
        Some(log) => {
            interpret.insert("transfer", Value::from(log.transfer));
            interpret.insert("primaries", Value::from(log.primaries));
        }
        None => {
            interpret.remove("transfer");
            interpret.remove("primaries");
        }
    }
    Ok(pm_kernel::MediaCommand::SetInterpretation {
        asset_id,
        interpret: (!interpret.is_empty()).then_some(Value::Object(interpret)),
    }
    .into())
}

/// Adds the LUT as an asset and an Apply Color LUT effect on `layer`, in one
/// Undo step. `path` is the retained copy of the file.
pub fn lut_commands(
    p: &Published,
    layer: &str,
    asset_id: &str,
    name: &str,
    path: &Path,
) -> Result<Vec<Command>, String> {
    use pm_kernel::commands::effects::{AddEffect, EffectCommand, saved_builtin_definition};
    let mut asset = Map::new();
    asset.insert("name", Value::from(name));
    asset.insert("kind", Value::from("lut"));
    asset.insert("format", Value::from("cube"));
    asset.insert("sourcePath", Value::from(path.to_string_lossy().as_ref()));
    let definition = saved_builtin_definition(
        &p.snapshot.document,
        "applyColorLut",
        Some(&Value::from(2)),
        0.,
        0.,
    )
    .map_err(|e| e.to_string())?;
    let mut input = Map::new();
    input.insert("kind", Value::from("asset"));
    input.insert("assetId", Value::from(asset_id));
    let mut inputs = Map::new();
    inputs.insert("lut", Value::Object(input));
    Ok(vec![
        pm_kernel::MediaCommand::AddAsset {
            asset_id: asset_id.to_owned(),
            asset: Value::Object(asset),
        }
        .into(),
        EffectCommand::Add(Box::new(AddEffect {
            layer_id: layer.to_owned(),
            effect_id: pm_kernel::new_id("f"),
            definition,
            parameters: Map::new(),
            inputs: Some(Value::Object(inputs)),
            open: true,
            enabled: true,
            enabled_animation: None,
            index: None,
        }))
        .into(),
    ])
}

/// Worker: validates the `.cube` and keeps a copy that outlives the original.
fn retain_lut(profile: &Path, path: &Path) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    pm_color::lut::CubeLut::parse(&text).map_err(|e| e.to_string())?;
    let archive = pm_kernel::media_archive::Archive::open(profile).map_err(|e| e.to_string())?;
    let original = archive
        .retain_path(path, "text/plain", &AtomicBool::new(false))
        .map_err(|e| e.to_string())?;
    Ok(original.path())
}

fn transact(p: &Published, label: &str, commands: Vec<Command>) -> Request {
    Request::new(
        Caller::native(),
        Call::Transact {
            label: Some(label.into()),
            body: WriteBody::Commands(commands),
            expected_revision: Some(p.revision()),
        },
    )
    .with_document(p.snapshot.doc_id)
}

/// The Scopes panel's state on the controller.
#[derive(Default)]
pub struct Scopes {
    view: Option<Entity<ScopesView>>,
    /// The viewer analyzed now.
    tapped: Option<u64>,
    latest: Option<Arc<ScopeFrame>>,
}

fn counts(frame: &ScopeFrame) -> Counts<'_> {
    Counts {
        columns: frame.config.columns as usize,
        levels: frame.config.levels as usize,
        vector_bins: frame.config.vector_bins as usize,
        pixels: frame.pixels,
        waveform: &frame.waveform,
        parade: [&frame.parade[0], &frame.parade[1], &frame.parade[2]],
        vectorscope: &frame.vectorscope,
        luma_histogram: &frame.luma_histogram,
        histogram: [
            &frame.histogram[0],
            &frame.histogram[1],
            &frame.histogram[2],
        ],
    }
}

/// A panel that stops painting for this long stops the GPU analysis.
const HIDDEN: Duration = Duration::from_secs(1);

impl Controller {
    /// `Interpret Selection As`: the selected clip's camera log.
    pub fn interpret_selection(&mut self, log: Option<CameraLog>, cx: &mut Context<Self>) {
        let (Some(p), Some(layer)) = (self.published.clone(), self.selected.clone()) else {
            self.message = "Select a video clip to interpret".into();
            cx.notify();
            return;
        };
        let command = match interpret_command(&p, &layer, log) {
            Ok(command) => command,
            Err(message) => {
                self.message = message;
                cx.notify();
                return;
            }
        };
        let pending = self
            .kernel
            .call(&p.project, transact(&p, "Interpret Footage", vec![command]));
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            this.update(cx, |v, cx| {
                if let Some(Err(e)) = result {
                    v.message = e.to_string();
                }
                v.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// `Apply LUT…`: asks for a `.cube` file and adds it to the selected layer.
    pub fn apply_lut(&mut self, cx: &mut Context<Self>) {
        let Some(layer) = self.selected.clone() else {
            self.message = "Select a layer to apply a LUT".into();
            cx.notify();
            return;
        };
        let picked = crate::platform::cube_file(&self.host.borrow().window);
        let profile = self.profile.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Some(path)) = picked.recv().await else {
                return;
            };
            let name = path
                .file_name()
                .map_or_else(|| "LUT".into(), |n| n.to_string_lossy().into_owned());
            let source = path.clone();
            let retained = cx
                .background_spawn(async move { retain_lut(&profile, &source) })
                .await;
            let start = this.update(cx, |v, cx| match retained {
                Ok(retained) => {
                    let p = v.published.clone()?;
                    let asset_id = pm_kernel::new_id("lut");
                    match lut_commands(&p, &layer, &asset_id, &name, &retained) {
                        Ok(commands) => {
                            v.media.originals.insert(asset_id, retained);
                            if let Some(viewer) = &v.viewer {
                                viewer.media(v.media.clone()).ok();
                            }
                            Some(
                                v.kernel
                                    .call(&p.project, transact(&p, "Apply LUT", commands)),
                            )
                        }
                        Err(message) => {
                            v.message = message;
                            cx.notify();
                            None
                        }
                    }
                }
                Err(message) => {
                    v.message = format!("“{name}” is not a LUT this app reads: {message}");
                    cx.notify();
                    None
                }
            });
            if let Ok(Some(pending)) = start {
                let result = pending.await;
                this.update(cx, |v, cx| {
                    if let Some(Err(e)) = result {
                        v.message = e.to_string();
                    }
                    v.refresh(cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// The Scopes panel's content, and its tap on the viewer.
    pub fn scopes_panel(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = match &self.scopes.view {
            Some(view) => view.clone(),
            None => {
                let tokens = self.host.borrow().tokens.clone();
                let controller = cx.entity().downgrade();
                let view = cx.new(|_| {
                    let mut view = ScopesView::new(tokens);
                    view.on_kind(move |kind, cx| {
                        controller
                            .update(cx, |v, cx| v.draw_scope(None, kind, cx))
                            .ok();
                    });
                    view
                });
                self.scopes.view = Some(view.clone());
                view
            }
        };
        let id = self.viewer.as_ref().map(|viewer| viewer.identity());
        if id.is_some() && self.scopes.tapped != id {
            let (frames, inbox) = async_channel::bounded(1);
            if let Some(viewer) = &self.viewer
                && viewer.scopes(Some(frames)).is_ok()
            {
                self.scopes.tapped = id;
                cx.spawn(async move |this, cx| {
                    while let Ok(frame) = inbox.recv().await {
                        let stop = this
                            .update(cx, |v, cx| v.scope_frame(Arc::new(frame), cx))
                            .unwrap_or(true);
                        if stop {
                            break;
                        }
                    }
                })
                .detach();
            }
        }
        view.into_any_element()
    }

    /// Returns whether the panel stopped showing.
    fn scope_frame(&mut self, frame: Arc<ScopeFrame>, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.scopes.view.clone() else {
            return true;
        };
        if view.read(cx).painted().elapsed() > HIDDEN {
            if let Some(viewer) = &self.viewer {
                viewer.scopes(None).ok();
            }
            self.scopes.tapped = None;
            return true;
        }
        let kind = view.read(cx).kind;
        self.draw_scope(Some(frame), kind, cx);
        false
    }

    fn draw_scope(&mut self, frame: Option<Arc<ScopeFrame>>, kind: Kind, cx: &mut Context<Self>) {
        if frame.is_some() {
            self.scopes.latest = frame;
        }
        let (Some(frame), Some(view)) = (self.scopes.latest.clone(), self.scopes.view.clone())
        else {
            return;
        };
        let colors = self.host.borrow().tokens.colors.clone();
        cx.spawn(async move |_, cx| {
            let hdr = frame.config.signal == ScopeSignal::Pq;
            let image = cx
                .background_spawn(async move { raster(&counts(&frame), kind, &colors) })
                .await;
            view.update(cx, |view, cx| {
                if view.kind == kind {
                    view.show(image, hdr, cx);
                }
            });
        })
        .detach();
    }

    /// Follows the project's HDR switch in the viewer's default output.
    pub fn follow_project_hdr(&mut self) {
        use crate::viewer2::OutputMode;
        let hdr = self
            .published
            .as_ref()
            .and_then(|p| p.snapshot.root())
            .and_then(|root| pm_render::composite::project_color(root).ok())
            .is_some_and(|color| color.mode == pm_protocol::color::ColorMode::Hdr);
        if hdr == self.project_hdr {
            return;
        }
        self.project_hdr = hdr;
        self.output_mode = match (hdr, self.output_mode) {
            (true, OutputMode::Sdr) => OutputMode::Edr,
            (false, OutputMode::Edr) => OutputMode::Sdr,
            (_, mode) => mode,
        };
        self.apply_viewer_options();
    }
}

#[cfg(test)]
#[path = "colour_tests.rs"]
mod tests;
