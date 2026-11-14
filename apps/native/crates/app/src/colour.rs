//! Colour tools for pros: camera log readings, `.cube` LUTs on a layer or
//! adjustment layer, and the Scopes panel. Commands go through the kernel;
//! file reads and rasterizing run on background threads.
use crate::session::Controller;
use gpui::{AppContext, Context, Entity, prelude::*};
use pm_kernel::{Call, Caller, Command, Published, Request, WriteBody};
use pm_model::{Map, Value};
use pm_render::scopes::{ScopeConfig, ScopeFrame, ScopeMatrix, ScopeSignal};
use pm_shell::scopes::{
    Counts, INVALID_FRAME, Kind, Menu, READ_FAILED, ScopesView, raster, reading,
};
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
        label: "LogC3 EI 800 (AWG3)",
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
pub(crate) fn layer_asset(p: &Published, layer: &str) -> Option<String> {
    let root = p.snapshot.root()?;
    let at = pm_kernel::model::find_layer(root, layer)?;
    let node = pm_kernel::model::layer_at(root, &at)?;
    let value = node.get("d")?.get("asset")?.to_value();
    value.as_str().map(str::to_owned)
}

/// Edits the Interpret Footage overrides of the asset `layer` shows.
pub(crate) fn edit_interpretation(
    p: &Published,
    layer: &str,
    edit: impl FnOnce(&mut Map),
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
    edit(&mut interpret);
    Ok(pm_kernel::MediaCommand::SetInterpretation {
        asset_id,
        interpret: (!interpret.is_empty()).then_some(Value::Object(interpret)),
    }
    .into())
}

/// Reads the selected clip's asset with `log`, or as the file says with `None`.
pub fn interpret_command(
    p: &Published,
    layer: &str,
    log: Option<CameraLog>,
) -> Result<Command, String> {
    edit_interpretation(p, layer, |interpret| match log {
        Some(log) => {
            interpret.insert("transfer", Value::from(log.transfer));
            interpret.insert("primaries", Value::from(log.primaries));
        }
        None => {
            interpret.remove("transfer");
            interpret.remove("primaries");
        }
    })
}

/// Switches the selected clip between its full encoded frame, the default, and
/// the file's clean aperture. A file without one stays full.
pub fn clean_aperture_command(p: &Published, layer: &str) -> Result<Command, String> {
    edit_interpretation(p, layer, |interpret| {
        if interpret.get("cleanAperture").is_some() {
            interpret.remove("cleanAperture");
        } else {
            interpret.insert("cleanAperture", Value::from(true));
        }
    })
}

/// The first Apply Color LUT on `layer` that needs a file: one the Effects
/// browser added with none yet, or one whose LUT is in `missing`.
fn open_lut_effect(
    p: &Published,
    layer: &str,
    missing: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let root = p.snapshot.root()?;
    let at = pm_kernel::model::find_layer(root, layer)?;
    let effects = pm_kernel::model::layer_at(root, &at)?.get("fx")?.to_value();
    effects.as_array()?.iter().find_map(|effect| {
        let id = effect.get("id")?.as_str()?;
        let asset = effect
            .get("inputs")
            .and_then(|inputs| inputs.get("lut"))
            .and_then(|lut| lut.get("assetId"));
        let open = effect.get("type")?.as_str()? == "applyColorLut"
            && asset.is_none_or(|asset| {
                asset.is_null() || asset.as_str().is_some_and(|id| missing.contains(id))
            });
        open.then(|| id.to_owned())
    })
}

/// A LUT file's format by its extension, or `None` for a file that is not
/// one (`.cube` or `.3dl`).
pub(crate) fn lut_file(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("cube") {
        Some("cube")
    } else if extension.eq_ignore_ascii_case("3dl") {
        Some("3dl")
    } else {
        None
    }
}

/// The asset record of a retained LUT file, as Apply LUT… and an agent's
/// import write it.
pub(crate) fn lut_asset(name: &str, path: &Path) -> Value {
    let mut asset = Map::new();
    asset.insert("name", Value::from(name));
    asset.insert("kind", Value::from("lut"));
    asset.insert("format", Value::from(lut_format(name)));
    asset.insert("sourcePath", Value::from(path.to_string_lossy().as_ref()));
    Value::Object(asset)
}

/// A LUT file's format, from its name: `3dl`, or `cube`.
fn lut_format(name: &str) -> &'static str {
    let extension = Path::new(name).extension().and_then(|e| e.to_str());
    if extension.is_some_and(|e| e.eq_ignore_ascii_case("3dl")) {
        "3dl"
    } else {
        "cube"
    }
}

/// Adds the LUT as an asset on `layer`'s Apply Color LUT that needs a file (an
/// empty one, or one whose LUT is in `missing`: a relink), or on a new one, in
/// one Undo step. `path` is the retained copy of the file.
pub fn lut_commands(
    p: &Published,
    layer: &str,
    asset_id: &str,
    name: &str,
    path: &Path,
    missing: &std::collections::BTreeSet<String>,
) -> Result<Vec<Command>, String> {
    use pm_kernel::commands::effects::{
        AddEffect, EffectCommand, EffectPatch, saved_builtin_definition,
    };
    let mut input = Map::new();
    input.insert("kind", Value::from("asset"));
    input.insert("assetId", Value::from(asset_id));
    let mut inputs = Map::new();
    inputs.insert("lut", Value::Object(input));
    let inputs = Some(Value::Object(inputs));
    let effect = match open_lut_effect(p, layer, missing) {
        Some(effect) => EffectCommand::Set {
            layer_id: layer.to_owned(),
            effect,
            patch: EffectPatch {
                inputs,
                ..Default::default()
            },
        },
        None => EffectCommand::Add(Box::new(AddEffect {
            layer_id: layer.to_owned(),
            effect_id: pm_kernel::new_id("f"),
            definition: saved_builtin_definition(
                &p.snapshot.document,
                "applyColorLut",
                Some(&Value::from(2)),
                0.,
                0.,
            )
            .map_err(|e| e.to_string())?,
            parameters: Map::new(),
            inputs,
            open: true,
            enabled: true,
            enabled_animation: None,
            index: None,
        })),
    };
    Ok(vec![
        pm_kernel::MediaCommand::AddAsset {
            asset_id: asset_id.to_owned(),
            asset: lut_asset(name, path),
        }
        .into(),
        effect.into(),
    ])
}

/// The first Color Wheels on `layer`, by effect id.
fn color_wheels(p: &Published, layer: &str) -> Option<String> {
    let root = p.snapshot.root()?;
    let at = pm_kernel::model::find_layer(root, layer)?;
    let effects = pm_kernel::model::layer_at(root, &at)?.get("fx")?.to_value();
    effects.as_array()?.iter().find_map(|effect| {
        (effect.get("type")?.as_str()? == "colorWheels")
            .then(|| effect.get("id")?.as_str().map(str::to_owned))
            .flatten()
    })
}

/// Import CDL: `cdl` onto `layer`'s first Color Wheels at the playhead (as a
/// value typed into each control: a key where the control is animated), or
/// on a new Color Wheels, in one Undo step. Balance, exposure and contrast
/// are left as they are; lift becomes 0, as a CDL has none.
pub fn cdl_commands(
    p: &Published,
    layer: &str,
    cdl: &pm_color::cdl::Cdl,
    time: f64,
) -> Result<Vec<Command>, String> {
    use pm_kernel::commands::effects::{AddEffect, EffectCommand, saved_builtin_definition};
    use pm_kernel::commands::properties::{KeyMode, SetProperty};
    let values = pm_color::cdl::to_wheels(cdl)?;
    Ok(match color_wheels(p, layer) {
        Some(effect) => values
            .into_iter()
            .map(|(key, value)| {
                let mut set =
                    SetProperty::new(layer, &format!("{effect}.{key}"), Value::from(value), time);
                set.mode = KeyMode::Auto;
                pm_kernel::PropertyCommand::Set(set).into()
            })
            .collect(),
        None => {
            let mut parameters = Map::new();
            for (key, value) in values {
                parameters.insert(key.as_str(), Value::from(value));
            }
            vec![
                EffectCommand::Add(Box::new(AddEffect {
                    layer_id: layer.to_owned(),
                    effect_id: pm_kernel::new_id("f"),
                    definition: saved_builtin_definition(
                        &p.snapshot.document,
                        "colorWheels",
                        None,
                        0.,
                        0.,
                    )
                    .map_err(|e| e.to_string())?,
                    parameters,
                    inputs: None,
                    open: true,
                    enabled: true,
                    enabled_animation: None,
                    index: None,
                }))
                .into(),
            ]
        }
    })
}

/// Export CDL: the correction `layer`'s first Color Wheels grades with at
/// `time` (keys and expressions evaluated as the renderer does), named after
/// the layer.
pub fn layer_cdl(p: &Published, layer: &str, time: f64) -> Result<pm_color::cdl::Cdl, String> {
    let effect =
        color_wheels(p, layer).ok_or("The layer has no Color Wheels to export as a CDL.")?;
    let root = p.snapshot.root().ok_or("No project is open.")?;
    let at = pm_kernel::model::find_layer(root, layer).ok_or("The layer is gone.")?;
    let node = pm_kernel::model::layer_at(root, &at).ok_or("The layer is gone.")?;
    let fx = node.get("fx").map(|f| f.to_value()).unwrap_or(Value::Null);
    let params = fx
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|e| e.get("id").and_then(Value::as_str) == Some(&effect))
        })
        .and_then(|e| e.get("p"))
        .cloned()
        .unwrap_or(Value::Null);
    let open = p.snapshot.open_comp().map(|c| c.as_str().to_owned());
    let mut mirror = pm_kernel::QueryMirror::new(p);
    let mut queries = mirror.queries();
    let mut failed = None;
    let mut cdl = pm_color::cdl::from_wheels(|key| {
        let raw = params.get(key).cloned().unwrap_or(Value::Null);
        let animated = raw
            .get("kf")
            .and_then(Value::as_array)
            .is_some_and(|k| !k.is_empty())
            || raw.get("expr").and_then(Value::as_str).is_some();
        let value = if animated {
            let mut q = Map::new();
            q.insert("op", Value::from("evP"));
            q.insert(
                "comp",
                at.comp_id
                    .clone()
                    .or(open.clone())
                    .map_or(Value::Null, Value::from),
            );
            q.insert("open", open.clone().map_or(Value::Null, Value::from));
            q.insert("layer", Value::Number(at.index as f64));
            q.insert("t", Value::Number(time));
            q.insert("key", Value::from(format!("{effect}.{key}")));
            q.insert("prop", raw);
            queries
                .query(&Value::Object(q))
                .ok()
                .and_then(|v| v.as_f64())
        } else {
            raw.get("v").and_then(Value::as_f64)
        };
        value.unwrap_or_else(|| {
            failed = Some(key.to_owned());
            0.
        })
    })?;
    if let Some(key) = failed {
        return Err(format!("The Color Wheels’ {key} can’t be read."));
    }
    cdl.id = node
        .get("name")
        .and_then(|n| n.to_value().as_str().map(str::to_owned))
        .filter(|name| !name.is_empty());
    Ok(cdl)
}

/// Worker: validates the LUT and keeps a copy that outlives the original.
pub(crate) fn retain_lut(profile: &Path, path: &Path) -> Result<PathBuf, String> {
    let unreadable = |reason: String| format!("This LUT couldn’t be read. {reason}");
    let size = std::fs::metadata(path)
        .map_err(|e| unreadable(e.to_string()))?
        .len();
    if size > pm_color::lut::MAX_FILE_BYTES {
        return Err("This LUT is too large.".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| unreadable(e.to_string()))?;
    pm_color::lut::CubeLut::parse_format(lut_format(&path.to_string_lossy()), &text)
        .and_then(|lut| lut.gpu())
        .map_err(|e| unreadable(e.to_string()))?;
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
    /// The viewer analyzed now, and what it was asked for.
    tapped: Option<(u64, ScopeConfig)>,
    latest: Option<Arc<ScopeFrame>>,
    /// The project's defaults were applied to this document.
    defaults: Option<u64>,
}

/// What the panel's settings ask the engine to count.
fn scope_config(rec2020: bool, extended: bool) -> ScopeConfig {
    ScopeConfig {
        signal: if extended {
            ScopeSignal::Pq
        } else {
            ScopeSignal::Display
        },
        matrix: if rec2020 {
            ScopeMatrix::Rec2020
        } else {
            ScopeMatrix::Rec709
        },
        ..ScopeConfig::PANEL
    }
}

/// The More menu, as AppKit rows.
fn more_menu(rec2020: bool, extended: bool, depth8: bool) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut range = json!({ "label": "Range: Extended", "checked": extended });
    if depth8 {
        range["disabled"] = true.into();
        range["tooltip"] = "This project has no values above SDR.".into();
    }
    vec![
        json!({ "label": "Matrix: Rec. 709", "checked": !rec2020 }),
        json!({ "label": "Matrix: Rec. 2020", "checked": rec2020 }),
        json!({ "separator": true }),
        json!({ "label": "Range: Standard", "checked": !extended }),
        range,
    ]
}

fn counts(frame: &ScopeFrame) -> Counts<'_> {
    Counts {
        extended: frame.config.signal == ScopeSignal::Pq,
        rec2020: frame.config.matrix == ScopeMatrix::Rec2020,
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
        under: frame.under,
        over: frame.over,
        outside: frame.outside,
    }
}

/// A panel that stops painting for this long stops the GPU analysis.
const HIDDEN: Duration = Duration::from_secs(1);

impl Controller {
    /// `Interpret Selection As`: the selected clip's camera log.
    pub fn interpret_selection(&mut self, log: Option<CameraLog>, cx: &mut Context<Self>) {
        self.change_interpretation(|p, layer| interpret_command(p, layer, log), cx);
    }

    /// `Crop to Clean Aperture`, on and off.
    pub fn crop_selection(&mut self, cx: &mut Context<Self>) {
        self.change_interpretation(clean_aperture_command, cx);
    }

    pub(crate) fn change_interpretation(
        &mut self,
        change: impl FnOnce(&Published, &str) -> Result<Command, String>,
        cx: &mut Context<Self>,
    ) {
        let (Some(p), Some(layer)) = (self.published.clone(), self.selected.clone()) else {
            self.message = "Select a video clip to interpret".into();
            cx.notify();
            return;
        };
        let command = match change(&p, &layer) {
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

    /// `Apply LUT…`: asks for a `.cube` or `.3dl` file and adds it to the
    /// selected layer, or relinks the layer's LUT whose file is missing.
    pub fn apply_lut(&mut self, cx: &mut Context<Self>) {
        let Some(layer) = self.selected.clone() else {
            self.message = "Select a layer to apply a LUT".into();
            cx.notify();
            return;
        };
        let picked = crate::platform::lut_file(&self.host.borrow().window);
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
                    let missing = v.offline_media();
                    match lut_commands(&p, &layer, &asset_id, &name, &retained, &missing) {
                        // The media refresh resolves the new asset's sourcePath.
                        Ok(commands) => Some(
                            v.kernel
                                .call(&p.project, transact(&p, "Apply LUT", commands)),
                        ),
                        Err(message) => {
                            v.message = message;
                            cx.notify();
                            None
                        }
                    }
                }
                Err(message) => {
                    v.message = message;
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

    /// Import CDL…: a `.cc`, `.cdl` or `.ccc` onto the selected layer's
    /// Color Wheels. `path` is the agent's file; a person picks one.
    pub fn import_cdl(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(layer) = self.selected.clone() else {
            self.message = "Select a layer to import a CDL onto".into();
            cx.notify();
            return;
        };
        let picked = match path {
            Some(path) => {
                let (tx, rx) = async_channel::bounded(1);
                let _ = tx.try_send(Some(path));
                rx
            }
            None => crate::platform::cdl_file(&self.host.borrow().window),
        };
        cx.spawn(async move |this, cx| {
            let Ok(Some(path)) = picked.recv().await else {
                return;
            };
            let read = cx
                .background_spawn(async move {
                    let text = std::fs::read_to_string(&path)
                        .map_err(|e| format!("The CDL can’t be read: {e}."))?;
                    pm_color::cdl::parse(&text)
                })
                .await;
            let start = this.update(cx, |v, cx| {
                let p = v.published.clone()?;
                match read.and_then(|cdl| cdl_commands(&p, &layer, &cdl, v.time())) {
                    Ok(commands) => {
                        v.message = String::new();
                        Some(
                            v.kernel
                                .call(&p.project, transact(&p, "Import CDL", commands)),
                        )
                    }
                    Err(message) => {
                        v.message = message;
                        cx.notify();
                        None
                    }
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

    /// Export CDL…: the selected layer's Color Wheels at the playhead as a
    /// `.cc` (or `.cdl` by its name). `path` is the agent's file, which must
    /// not exist yet; a person names one in a save panel.
    pub fn export_cdl(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let (Some(layer), Some(p)) = (self.selected.clone(), self.published.clone()) else {
            self.message = "Select a layer to export its CDL".into();
            cx.notify();
            return;
        };
        let cdl = match layer_cdl(&p, &layer, self.time()) {
            Ok(cdl) => cdl,
            Err(message) => {
                self.message = message;
                cx.notify();
                return;
            }
        };
        let agent = path.is_some();
        let picked = match path {
            Some(path) => {
                let (tx, rx) = async_channel::bounded(1);
                let _ = tx.try_send(Some(path));
                rx
            }
            None => {
                let name = format!("{}.cc", cdl.id.as_deref().unwrap_or("Grade"));
                crate::platform::paths_named(&self.host.borrow().window, true, &name)
            }
        };
        cx.spawn(async move |this, cx| {
            let Ok(Some(path)) = picked.recv().await else {
                return;
            };
            let written = cx
                .background_spawn(async move {
                    if agent && path.exists() {
                        return Err("A file is already there; choose another name.".to_owned());
                    }
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let text = pm_color::cdl::write(&cdl, pm_color::cdl::Layout::of(&name));
                    std::fs::write(&path, text)
                        .map_err(|e| format!("The CDL can’t be written: {e}."))
                })
                .await;
            this.update(cx, |v, cx| {
                v.message = written.err().unwrap_or_default();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The Scopes panel's content, and its tap on the viewer.
    pub fn scopes_panel(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = self.scopes_view(cx);
        let tokens = self.host.borrow().tokens.clone();
        view.update(cx, |view, _| view.tokens = tokens);
        // An HDR project reads as Rec. 2020, on the extended range.
        if let Some(p) = self.published.clone()
            && self.scopes.defaults != Some(p.snapshot.doc_id)
        {
            self.scopes.defaults = Some(p.snapshot.doc_id);
            let hdr = self.project_hdr;
            view.update(cx, |view, _| {
                view.rec2020 = hdr;
                view.extended = hdr;
            });
        }
        self.tap_scopes(cx);
        view.into_any_element()
    }

    /// The panel header's Scope pop-up and More menu.
    pub fn scopes_header(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let view = self.scopes_view(cx);
        view.update(cx, |view, cx| view.header(cx))
            .into_any_element()
    }

    fn scopes_view(&mut self, cx: &mut Context<Self>) -> Entity<ScopesView> {
        if let Some(view) = &self.scopes.view {
            return view.clone();
        }
        let tokens = self.host.borrow().tokens.clone();
        let controller = cx.entity().downgrade();
        let view = cx.new(|_| {
            let mut view = ScopesView::new(tokens);
            view.on_menu(move |menu, window, cx| {
                controller
                    .update(cx, |v, cx| v.scopes_menu(menu, window, cx))
                    .ok();
            });
            view
        });
        self.scopes.view = Some(view.clone());
        view
    }

    fn scopes_menu(&mut self, menu: Menu, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(view) = self.scopes.view.clone() else {
            return;
        };
        let (kind, rec2020, extended) = {
            let view = view.read(cx);
            (view.kind, view.rec2020, view.extended)
        };
        let this = cx.entity().downgrade();
        match menu {
            Menu::Scope => {
                let options: Vec<_> = Kind::ALL
                    .iter()
                    .map(|k| serde_json::json!({ "label": k.label(), "checked": *k == kind }))
                    .collect();
                crate::v4_native::menu(window, &options, cx, move |chosen, cx| {
                    let Some(kind) = chosen.and_then(|i| Kind::ALL.get(i).copied()) else {
                        return;
                    };
                    this.update(cx, |v, cx| {
                        view.update(cx, |view, _| view.kind = kind);
                        v.draw_scope(None, cx);
                    })
                    .ok();
                });
            }
            Menu::More => {
                let depth8 = self
                    .published
                    .as_ref()
                    .and_then(|p| p.snapshot.root())
                    .and_then(|root| root.get("colorDepth"))
                    .is_some_and(|d| d.to_value().as_f64() == Some(8.));
                let options = more_menu(rec2020, extended, depth8);
                crate::v4_native::menu(window, &options, cx, move |chosen, cx| {
                    let (rec2020, extended) = match chosen {
                        Some(0) => (false, extended),
                        Some(1) => (true, extended),
                        Some(3) => (rec2020, false),
                        Some(4) => (rec2020, true),
                        _ => return,
                    };
                    this.update(cx, |v, cx| {
                        view.update(cx, |view, _| {
                            view.rec2020 = rec2020;
                            view.extended = extended;
                        });
                        v.tap_scopes(cx);
                    })
                    .ok();
                });
            }
        }
    }

    /// Keeps the engine's tap on this viewer, with the panel's settings.
    fn tap_scopes(&mut self, cx: &mut Context<Self>) {
        let (Some(view), Some(viewer)) = (&self.scopes.view, &self.viewer) else {
            return;
        };
        let settings = view.read(cx);
        let config = scope_config(settings.rec2020, settings.extended);
        let wanted = (viewer.identity(), config);
        if self.scopes.tapped == Some(wanted) {
            return;
        }
        let (frames, inbox) = async_channel::bounded(1);
        if viewer.scopes(Some((config, frames))).is_err() {
            return;
        }
        self.scopes.tapped = Some(wanted);
        cx.spawn(async move |this, cx| {
            while let Ok(result) = inbox.recv().await {
                let stop = this
                    .update(cx, |v, cx| v.scope_result(result, cx))
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    /// Returns whether the panel stopped showing.
    fn scope_result(
        &mut self,
        result: pm_engine_service::ScopeResult,
        cx: &mut Context<Self>,
    ) -> bool {
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
        match result {
            Ok(frame) if frame.invalid > 0 => {
                self.message = format!("Scopes: {} pixels are not finite", frame.invalid);
                view.update(cx, |view, cx| view.fail(INVALID_FRAME, cx));
            }
            Ok(frame) => self.draw_scope(Some(Arc::new(frame)), cx),
            Err(error) => {
                self.message = format!("Scopes: {error}");
                view.update(cx, |view, cx| view.fail(READ_FAILED, cx));
            }
        }
        false
    }

    fn draw_scope(&mut self, frame: Option<Arc<ScopeFrame>>, cx: &mut Context<Self>) {
        if frame.is_some() {
            self.scopes.latest = frame;
        }
        let (Some(frame), Some(view)) = (self.scopes.latest.clone(), self.scopes.view.clone())
        else {
            return;
        };
        let kind = view.read(cx).kind;
        let colors = self.host.borrow().tokens.colors.clone();
        cx.spawn(async move |_, cx| {
            let (image, values) = cx
                .background_spawn(async move {
                    let counts = counts(&frame);
                    (raster(&counts, kind, &colors), reading(&counts, kind))
                })
                .await;
            view.update(cx, |view, cx| {
                if view.kind == kind {
                    view.show(image, values, cx);
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
