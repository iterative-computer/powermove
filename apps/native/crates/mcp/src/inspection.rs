//! Read-only kernel evaluation: source timing, 3D digests and structural lint.
use crate::media_tools::LayerTiming;
use crate::{Automation, Error, Result, json_value, model};
use pm_kernel::DocumentStore;
use serde_json::{Value, json};
use std::collections::HashSet;
fn round(n: f64) -> f64 {
    (n * 1000.).round() / 1000.
}
fn animated(v: &Value) -> bool {
    v["kf"].as_array().is_some_and(|a| !a.is_empty())
        || v["expr"].as_str().is_some_and(|s| !s.is_empty())
}
pub struct Evaluation {
    pub project: Value,
    store: DocumentStore,
    comp: String,
}
impl Evaluation {
    pub fn new(p: &Value) -> Self {
        let store = DocumentStore::new_untitled(model(p));
        let comp = store
            .open_comp()
            .unwrap_or_else(|| format!("@root:{}", p["id"].as_str().unwrap_or("")));
        Self {
            project: p.clone(),
            store,
            comp,
        }
    }
    pub fn query(&mut self, op: &str, index: usize, time: f64, extra: Value) -> Result<Value> {
        let mut request = json!({"op":op,"comp":self.comp,"open":self.comp,"layer":index,"t":time});
        if let Some(o) = extra.as_object() {
            for (k, v) in o {
                request[k] = v.clone();
            }
        }
        Ok(json_value(
            &self
                .store
                .queries()
                .query(&model(&request))
                .map_err(Error::kernel)?,
        ))
    }
    pub fn timing(&mut self, index: usize, limit: Option<f64>) -> Result<LayerTiming> {
        let l = self.project["layers"][index].clone();
        let from = l["from"].as_f64().unwrap_or(0.);
        let duration = l["dur"].as_f64().unwrap_or(0.).max(0.);
        let content = self.query("resolveContent", index, from, json!({}))?;
        let constant = !crate::model(&content["timeRemap"]).is_truthy()
            && !animated(&l["d"]["timeRemap"])
            && !animated(&l["d"]["speed"])
            && !animated(&l["d"]["trim"]);
        let count = if constant {
            2
        } else {
            ((duration * 20.).ceil() as usize + 1).clamp(2, 2000)
        };
        let mut samples = vec![];
        for i in 0..count {
            let t = from + duration * i as f64 / (count - 1) as f64;
            let source = self
                .query("sourceTime", index, t, json!({"retiming":"legacy"}))?
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| Error::invalid("Source timing is not finite"))?;
            samples.push([
                (t * 10000.).round() / 10000.,
                (source * 10000.).round() / 10000.,
            ]);
        }
        let source_start = samples
            .iter()
            .map(|p| p[1])
            .reduce(f64::min)
            .unwrap_or(0.)
            .max(0.);
        let source_end = samples
            .iter()
            .map(|p| p[1])
            .reduce(f64::max)
            .unwrap_or(0.)
            .min(limit.unwrap_or(f64::INFINITY));
        Ok(LayerTiming {
            id: l["id"].as_str().unwrap_or("").into(),
            name: l["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or(l["id"].as_str())
                .unwrap_or("")
                .into(),
            from,
            duration,
            constant,
            samples,
            source_start,
            source_end,
        })
    }
}
fn role(l: &Value) -> Option<&'static str> {
    match l["d"]["definition"].as_str() {
        Some("powermove.3d.object") => Some("object"),
        Some("powermove.3d.light") => Some("light"),
        Some("powermove.3d.camera") => Some("camera"),
        _ => None,
    }
}
pub fn scene(host: &Automation, id: &str, args: &Value) -> Result<Value> {
    let p = host.read_document(id)?;
    let mut ev = Evaluation::new(&p);
    let time = host.playhead(id);
    let mut layers = vec![];
    let mut channels = vec![];
    for (index, l) in p["layers"].as_array().into_iter().flatten().enumerate() {
        let Some(role) = role(l) else { continue };
        layers.push(json!({"id":l["id"],"name":l["name"],"role":role,"parent":l["parent"],"visible":l["on"],"locked":l["lock"],"from":l["from"],"duration":l["dur"],"content":l["d"]["data"]}));
        for (path, prop) in crate::scene3d::properties(l) {
            let value = ev.query("evP", index, time, json!({"key":path,"prop":prop}))?;
            channels.push(json!({"layerId":l["id"],"path":path,"value":value,"keyframeCount":prop["kf"].as_array().map_or(0,Vec::len)}));
        }
    }
    let scene = crate::scene3d::evaluated_scene(&p, time, &mut ev)?;
    let assets = p["assets"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, a)| ["model", "image"].contains(&a["kind"].as_str().unwrap_or("")))
        .map(|(_, a)| {
            let mut out = json!({"id":a["id"],"name":a["name"],"kind":a["kind"]});
            if let Some(format) = a.get("format") {
                out["format"] = format.clone();
            }
            out
        })
        .collect::<Vec<_>>();
    let mut result = json!({"layerId":args["target"].as_str().filter(|s|!s.is_empty()),"scene":scene,"layers":layers,"channels":channels,"assets":assets,"revision":p["revision"]});
    let issues=p["layers"].as_array().into_iter().flatten().filter(|l|l["native3dUpgradeIssue"].is_string()).map(|l|json!({"layerId":l["id"],"message":l["native3dUpgradeIssue"],"diagnostic":l["native3dUpgradeDiagnostic"],"sourceRetained":l["native3dArchive"].get("source").is_some()})).collect::<Vec<_>>();
    if !issues.is_empty() {
        result["upgradeIssues"] = json!(issues);
    }
    if let Some(issue) = p.get("native3dHistoryIssue") {
        result["historyUpgradeIssue"] = issue.clone();
    }
    Ok(result)
}
fn issue(code: &str, severity: &str, l: &Value, message: String) -> Value {
    json!({"code":code,"severity":severity,"layerId":l["id"],"message":message})
}
fn static_on(l: &Value) -> Option<bool> {
    if l["on"].is_object() {
        if animated(&l["on"]) {
            None
        } else {
            Some(crate::model(&l["on"]["v"]).is_truthy())
        }
    } else {
        Some(l["on"] != false)
    }
}
fn groups<'a>(l: &'a Value, layers: &'a [Value]) -> Vec<&'a Value> {
    let mut out = vec![];
    let mut at = l["group"].as_str();
    let mut seen = HashSet::new();
    while let Some(id) = at {
        if !seen.insert(id) {
            break;
        }
        let Some(g) = layers.iter().find(|l| l["id"] == id) else {
            break;
        };
        out.push(g);
        at = g["group"].as_str();
    }
    out
}
pub fn check(host: &Automation, id: &str) -> Result<Value> {
    // The kernel's id for the open project (layer loops below shadow `id`).
    let project = id;
    let p = host.read_document(id)?;
    let mut ev = Evaluation::new(&p);
    let layers = p["layers"]
        .as_array()
        .ok_or_else(|| Error::invalid("Missing layers"))?;
    let fps = p["fps"].as_f64().unwrap_or(30.).max(1.);
    let frame = 1. / fps;
    let duration = p["dur"].as_f64().unwrap_or(0.).max(0.);
    let mut span = [
        p["work"][0].as_f64().unwrap_or(0.).clamp(0., duration),
        p["work"][1]
            .as_f64()
            .unwrap_or(duration)
            .clamp(0., duration),
    ];
    if span[1] <= span[0] {
        span = [0., duration];
    }
    let mut issues = vec![];
    let mut unusable = HashSet::new();
    let registry = host.media_registry.as_deref();
    let asset_name = |asset: &str| {
        p["assets"][asset]["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(asset)
            .to_owned()
    };
    for l in layers {
        if !["video", "audio", "image"].contains(&l["type"].as_str().unwrap_or("")) {
            continue;
        }
        let name = l["name"].as_str().unwrap_or("");
        let id = l["id"].as_str().unwrap_or("");
        let Some(asset) = l["d"]["asset"].as_str().filter(|s| !s.is_empty()) else {
            unusable.insert(id);
            issues.push(issue(
                "no-media",
                "warning",
                l,
                format!("“{name}” has no media attached, so it draws nothing."),
            ));
            continue;
        };
        if !p["assets"][asset].is_object() {
            unusable.insert(id);
            issues.push(issue(
                "missing-media",
                "error",
                l,
                format!("“{name}” points at media {asset}, which is not in the project."),
            ));
            continue;
        }
        if let Some(registry) = registry {
            let runtime = registry.state(project, asset);
            if runtime.available || runtime.loading {
                continue;
            }
            unusable.insert(id);
            if let Some(error) = runtime.error {
                issues.push(issue(
                    "media-failed",
                    "error",
                    l,
                    format!("“{}” failed to load: {error}", asset_name(asset)),
                ));
            } else if runtime.cloud {
                let provider = runtime.cloud_provider.as_deref().unwrap_or("undefined");
                issues.push(issue("media-in-cloud","warning",l,format!("“{}” is in {provider} and not downloaded, so “{name}” is blank until it is.",asset_name(asset))));
            } else {
                issues.push(issue(
                    "media-offline",
                    "error",
                    l,
                    format!(
                        "“{}” is offline; “{name}” is blank until the file is relinked.",
                        asset_name(asset)
                    ),
                ));
            }
        }
    }
    let mut inside = vec![];
    for (index, l) in layers.iter().enumerate() {
        let from = l["from"].as_f64().unwrap_or(0.);
        let dur = l["dur"].as_f64().unwrap_or(0.);
        let name = l["name"].as_str().unwrap_or("");
        let id = l["id"].as_str().unwrap_or("");
        if dur <= 0. {
            issues.push(issue(
                "zero-duration",
                "warning",
                l,
                format!("“{name}” has no duration, so it never plays."),
            ));
            continue;
        }
        if from >= duration - 1e-6 || from + dur <= 1e-6 {
            issues.push(issue(
                "outside-composition",
                "warning",
                l,
                format!(
                    "“{name}” plays {}s–{}s, entirely outside the {}s composition.",
                    round(from),
                    round(from + dur),
                    round(duration)
                ),
            ));
            continue;
        }
        inside.push((index, l));
        if ["video", "audio"].contains(&l["type"].as_str().unwrap_or("")) && !unusable.contains(id)
        {
            let asset = l["d"]["asset"].as_str().unwrap_or("");
            if p["assets"][asset]["imageSequence"].is_object()
                || p["assets"][asset]["imageSequence"] == true
            {
                continue;
            }
            let runtime = registry
                .map(|r| r.state(project, asset))
                .unwrap_or_default();
            let Some(length) = runtime
                .duration
                .or(p["assets"][asset]["dur"].as_f64())
                .filter(|d| *d > 0.)
            else {
                continue;
            };
            let timing = ev.timing(index, Some(length))?;
            if !timing.constant {
                continue;
            }
            let last = timing.samples[1][1];
            let over = last - length;
            if over > frame * 1.5 {
                let starts = from
                    + ((length - timing.samples[0][1])
                        / ((last - timing.samples[0][1]) / dur).max(1e-9))
                    .max(0.);
                let mut out = issue(
                    "past-media-end",
                    "warning",
                    l,
                    format!(
                        "“{name}” runs {}s past the end of “{}”; nothing new plays after {}s.",
                        round(over),
                        asset_name(asset),
                        round(starts)
                    ),
                );
                out["ranges"] = json!([[round(starts), round(from + dur)]]);
                issues.push(out);
            }
        }
    }
    let matte = layers
        .iter()
        .filter_map(|l| l["matteSource"].as_str())
        .collect::<HashSet<_>>();
    let visual = inside
        .iter()
        .filter(|(_, l)| {
            !["audio", "null", "group", "adjustment"].contains(&l["type"].as_str().unwrap_or(""))
                && !matte.contains(l["id"].as_str().unwrap_or(""))
        })
        .copied()
        .collect::<Vec<_>>();
    let nothing = |l: &Value| {
        unusable.contains(l["id"].as_str().unwrap_or(""))
            || l["type"] == "text" && l["d"]["text"].as_str().is_some_and(|s| s.trim().is_empty())
            || l["type"] == "precomp"
                && l["d"]["comp"]
                    .as_str()
                    .is_none_or(|c| !p["comps"][c].is_object())
    };
    for (_, l) in &visual {
        if l["type"] == "precomp" && nothing(l) {
            issues.push(issue(
                "missing-composition",
                "error",
                l,
                format!(
                    "“{}” nests a composition that does not exist.",
                    l["name"].as_str().unwrap_or("")
                ),
            ));
        }
    }
    let frames = (((span[1] - span[0]) * fps - 1e-6).ceil() as usize).max(1);
    let stride = (frames.saturating_mul(visual.len().max(1)))
        .div_ceil(400000)
        .max(1);
    let mut active = HashSet::new();
    let mut visible = HashSet::new();
    let mut gaps = vec![];
    let mut gap = None;
    for index in (0..frames).step_by(stride) {
        let time = span[0] + index as f64 * frame;
        let mut covered = false;
        for (i, l) in &visual {
            if ev.query("active", *i, time, json!({}))? != true {
                continue;
            }
            active.insert(l["id"].as_str().unwrap_or(""));
            if ev
                .query("worldOpacity", *i, time, json!({}))?
                .as_f64()
                .unwrap_or(0.)
                <= 0.001
            {
                continue;
            }
            visible.insert(l["id"].as_str().unwrap_or(""));
            if !nothing(l) {
                covered = true;
            }
        }
        if !covered && gap.is_none() {
            gap = Some(time)
        }
        if covered && let Some(start) = gap.take() {
            gaps.push([round(start), round(time)]);
        }
    }
    if let Some(start) = gap {
        gaps.push([round(start), round(span[1])]);
    }
    if !gaps.is_empty() {
        let total = gaps.iter().map(|g| g[1] - g[0]).sum::<f64>();
        let fill = &p["backgroundFill"];
        let background = if fill["type"] == "none" {
            "transparent frames".into()
        } else if fill["type"] == "solid" {
            format!(
                "only the background ({})",
                fill["stops"][0]["color"]
                    .as_str()
                    .or(p["bg"].as_str())
                    .unwrap_or("undefined")
            )
        } else if fill.is_null() {
            format!(
                "only the background ({})",
                p["bg"].as_str().unwrap_or("#000000").to_uppercase()
            )
        } else {
            "only the background".into()
        };
        let all = gaps.len() == 1 && gaps[0][0] <= span[0] + 1e-6 && gaps[0][1] >= span[1] - 1e-6;
        let message = if all {
            format!(
                "No layer is visible anywhere in {}s–{}s; every frame shows {background}.",
                round(span[0]),
                round(span[1])
            )
        } else {
            format!(
                "No layer is visible in {} {} totaling {}s; those frames show {background}.",
                gaps.len(),
                if gaps.len() == 1 { "span" } else { "spans" },
                round(total)
            )
        };
        issues.insert(0,json!({"code":if all{"no-visuals"}else{"empty-frames"},"severity":"error","ranges":gaps.iter().take(40).collect::<Vec<_>>(),"message":message}));
    }
    for (_, l) in &visual {
        let id = l["id"].as_str().unwrap_or("");
        if visible.contains(id) || nothing(l) {
            continue;
        }
        let from = l["from"].as_f64().unwrap_or(0.);
        let dur = l["dur"].as_f64().unwrap_or(0.);
        let name = l["name"].as_str().unwrap_or("");
        if !active.contains(id) {
            if static_on(l) != Some(false)
                && !groups(l, layers)
                    .iter()
                    .any(|g| static_on(g) == Some(false))
                && (from >= span[1] || from + dur <= span[0])
            {
                issues.push(issue(
                    "outside-work-area",
                    "warning",
                    l,
                    format!(
                        "“{name}” plays {}s–{}s, outside the checked work area {}s–{}s.",
                        round(from),
                        round(from + dur),
                        round(span[0]),
                        round(span[1])
                    ),
                ));
            }
            continue;
        }
        issues.push(issue("never-visible","warning",l,format!("“{name}” is fully transparent wherever it plays in {}s–{}s (its opacity or its group's opacity is 0), so it never shows.",round(span[0]),round(span[1]))));
    }
    if let Some(registry) = registry {
        for (index, l) in &inside {
            if l["type"] != "audio" && !(l["type"] == "video" && l["d"]["embeddedAudio"] == true)
                || unusable.contains(l["id"].as_str().unwrap_or(""))
            {
                continue;
            }
            let runtime = registry.state(project, l["d"]["asset"].as_str().unwrap_or(""));
            let Some(length) = runtime
                .audio_duration
                .or(runtime.duration)
                .filter(|d| *d > 0.)
            else {
                continue;
            };
            if runtime.peaks.is_empty() {
                continue;
            }
            let gain = if l["type"] == "audio" {
                let g = &l["d"]["gain"];
                if g.is_object() {
                    std::iter::once(g["v"].as_f64().unwrap_or(1.))
                        .chain(
                            g["kf"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|k| k["v"].as_f64()),
                        )
                        .reduce(f64::max)
                        .unwrap_or(1.)
                } else {
                    g.as_f64().unwrap_or(1.)
                }
            } else {
                1.
            };
            let timing = ev.timing(*index, Some(length))?;
            let min = timing
                .samples
                .iter()
                .map(|p| p[1])
                .reduce(f64::min)
                .unwrap_or(0.);
            let max = timing
                .samples
                .iter()
                .map(|p| p[1])
                .reduce(f64::max)
                .unwrap_or(0.);
            let start = (min / length * runtime.peaks.len() as f64).floor().max(0.) as usize;
            let stop = (max / length * runtime.peaks.len() as f64).ceil().max(0.) as usize;
            let peak = runtime
                .peaks
                .iter()
                .skip(start)
                .take(stop.saturating_sub(start))
                .copied()
                .reduce(f64::max)
                .unwrap_or(0.);
            let level = peak * gain;
            if level > 1.001 {
                issues.push(issue("audio-clipping","warning",l,format!("“{}” peaks at +{:.1} dBFS with gain {}; it will clip. Lower its gain to {} or less.",l["name"].as_str().unwrap_or(""),20.*level.log10(),round(gain),round((gain/level*1000.).floor()/1000.))));
            }
        }
    }
    let mut out = json!({"ok":issues.is_empty(),"span":[round(span[0]),round(span[1])],"fps":fps,"sampledEveryFrames":stride,"layers":layers.len(),"mediaChecked":registry.is_some(),"issues":issues.iter().take(60).collect::<Vec<_>>()});
    if issues.len() > 60 {
        out["truncated"] = json!(issues.len() - 60);
    }
    Ok(out)
}
