//! Generate Captions (S090; today's `captions/generate.ts` and its command):
//! the speech of the selected audio and video clips, transcribed on this Mac,
//! placed on the composition through each clip's trim, speed and time
//! remapping and split into readable cues, added as one captions layer in one
//! Undo step. Runs off main as a Background Tasks row with Cancel; the agent's
//! `generate_captions` runs the same [`generate`].
use crate::{
    session::Controller,
    transcription::{self, Failure},
};
use gpui::Context;
use pm_captions::{
    CaptionWord,
    generate::{ClipTiming, SegmentOptions, map_words_to_composition, segment_words},
};
use pm_kernel::Published;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// One clip to caption, read from the open composition.
#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub index: usize,
    pub asset: String,
    pub from: f64,
    pub dur: f64,
    /// Constant-rate: `trim` and `speed` give its timing without sampling.
    pub linear: Option<(f64, f64)>,
}
fn animated(v: &Value) -> bool {
    v.is_object()
        && (v["kf"].as_array().is_some_and(|k| !k.is_empty())
            || v["expr"].as_str().is_some_and(|e| !e.is_empty()))
}
fn number(v: &Value, fallback: f64) -> f64 {
    if v.is_object() { &v["v"] } else { v }
        .as_f64()
        .filter(|n| n.is_finite())
        .unwrap_or(fallback)
}
/// The captionable clips among `ids` (audio or video with media).
pub fn targets(comp: &Value, ids: &[String]) -> Vec<Target> {
    comp["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, l)| {
            ids.iter().any(|id| l["id"] == id.as_str())
                && matches!(l["type"].as_str(), Some("audio" | "video"))
                && l["d"]["asset"].as_str().is_some_and(|a| !a.is_empty())
        })
        .map(|(index, l)| {
            let d = &l["d"];
            let remapped = d["timeRemap"] == true && d["sourceTime"].is_object();
            let varying = animated(&d["speed"]) || remapped;
            let speed = number(&d["speed"], 1.);
            Target {
                name: l["name"].as_str().unwrap_or("Clip").into(),
                index,
                asset: d["asset"].as_str().unwrap_or_default().into(),
                from: l["from"].as_f64().unwrap_or(0.),
                dur: l["dur"].as_f64().unwrap_or(0.).max(0.),
                linear: (!varying).then(|| (number(&d["trim"], 0.), speed)),
            }
        })
        .collect()
}
/// The open composition as wire JSON and its id.
pub fn open_comp(published: &Published) -> Option<(String, Value)> {
    let root = published.snapshot.root()?;
    let id = published.snapshot.open_comp();
    let node = pm_kernel::model::comp_node(root, id.as_deref()).unwrap_or(root);
    let comp: Value =
        serde_json::from_str(&pm_model::json::to_canonical_string(&node.to_value())).ok()?;
    Some((published.snapshot.render_comp(), comp))
}
/// The clip's composition → source map: two samples when constant, else the
/// kernel's `sourceTime` at 120 per second (today's sampling rate).
pub fn clip_timing(published: &Published, target: &Target) -> Result<ClipTiming, Failure> {
    let comp = published
        .snapshot
        .open_comp()
        .unwrap_or_else(|| published.snapshot.render_comp());
    if let Some((trim, speed)) = target.linear {
        return Ok(ClipTiming::linear(target.from, target.dur, trim, speed));
    }
    let steps = ((target.dur * 120.).ceil() as usize).clamp(1, 120 * 60 * 60);
    let mut mirror = pm_kernel::QueryMirror::new(published);
    let queries = mirror.queries();
    let mut samples = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = target.from + target.dur * i as f64 / steps as f64;
        let request = json!({"op":"sourceTime","comp":comp,"open":comp,"layer":target.index,"t":t,"retiming":"legacy"});
        let value = pm_model::json::parse(&request.to_string())
            .ok()
            .and_then(|r| queries.query(&r).ok())
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                Failure::Failed(format!("Couldn’t read how “{}” is timed.", target.name))
            })?;
        samples.push(value);
    }
    Ok(ClipTiming {
        from: target.from,
        dur: target.dur,
        samples,
    })
}
/// A transcript's words; a segment without word times counts as one word.
fn words(transcript: &transcription::Transcript) -> Vec<CaptionWord> {
    transcript
        .segments
        .iter()
        .flat_map(|s| {
            if s.words.is_empty() {
                vec![CaptionWord {
                    text: s.text.trim().into(),
                    start: s.start,
                    end: s.end,
                }]
            } else {
                s.words
                    .iter()
                    .map(|w| CaptionWord {
                        text: w.text.trim().into(),
                        start: w.start,
                        end: w.end,
                    })
                    .collect()
            }
        })
        .filter(|w| !w.text.is_empty())
        .collect()
}
#[derive(Clone)]
pub enum Outcome {
    /// Cues in composition seconds, the transcript language and a layer name.
    Captions {
        cues: Value,
        count: usize,
        language: Option<String>,
        name: String,
    },
    /// No speech in any clip.
    Empty,
}
/// Media originals by asset id, for the clips' sources.
pub fn originals(
    published: &Published,
    profile: &Path,
) -> std::collections::BTreeMap<String, PathBuf> {
    crate::media::for_session(
        &published.snapshot,
        &published.media,
        published.path.as_deref(),
        &crate::media::cache_root(profile),
    )
    .map(|r| r.catalog.originals.into_iter().collect())
    .unwrap_or_default()
}
/// Transcribes each target's visible source span and segments the words.
/// Blocking: run it off main. `progress` gets 0…1 for the whole job.
pub fn generate(
    published: &Published,
    targets: &[Target],
    profile: &Path,
    cancel: &Arc<AtomicBool>,
    progress: Arc<dyn Fn(f64) + Send + Sync>,
) -> Result<Outcome, Failure> {
    let originals = originals(published, profile);
    let mut heard = vec![];
    let mut language = None;
    for (i, target) in targets.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err(Failure::Cancelled);
        }
        let clip = clip_timing(published, target)?;
        let [start, end] = clip.visible_source_range();
        let path = originals.get(&target.asset).cloned().ok_or_else(|| {
            Failure::Failed(format!(
                "The media of “{}” is offline. Relink it first.",
                target.name
            ))
        })?;
        let total = targets.len() as f64;
        let report = progress.clone();
        let transcript = transcription::transcribe(
            transcription::Request {
                path,
                start: (start > 0.).then_some(start),
                end: (end > start).then_some(end),
                language: None,
                word_timestamps: true,
            },
            cancel.clone(),
            move |p| report((i as f64 + p.clamp(0., 1.)) / total),
        )?;
        if cancel.load(Ordering::Acquire) {
            return Err(Failure::Cancelled);
        }
        language = language.or(transcript.language.clone());
        heard.extend(map_words_to_composition(&words(&transcript), &clip));
    }
    if heard.is_empty() {
        return Ok(Outcome::Empty);
    }
    heard.sort_by(|a, b| a.start.total_cmp(&b.start));
    let cues = segment_words(&heard, SegmentOptions::default(), &mut || {
        pm_kernel::new_id("cue")
    });
    let name = match targets {
        [one] => format!("{} Captions", one.name),
        _ => "Captions".into(),
    };
    Ok(Outcome::Captions {
        count: cues.len(),
        cues: serde_json::to_value(&cues).unwrap_or_default(),
        language,
        name,
    })
}

impl Controller {
    /// Generate Captions on `ids` (the selection when `None`).
    pub fn generate_captions(&mut self, ids: Option<Vec<String>>, cx: &mut Context<Self>) {
        let Some(published) = self.published.clone() else {
            return;
        };
        let ids = ids.unwrap_or_else(|| published.selection.layers().to_vec());
        let Some((_, comp)) = open_comp(&published) else {
            return;
        };
        let targets = targets(&comp, &ids);
        if targets.is_empty() {
            return self.note("Select an audio or video clip to caption.".into(), cx);
        }
        if self.captions.generating.is_some() {
            return self.note("Captions are already being generated.".into(), cx);
        }
        let Some(store) = transcription::store() else {
            return self.note("Transcription isn’t available here.".into(), cx);
        };
        if store.model_for(true).is_none() {
            let reason = if store.active().is_some() {
                "Captions need a speech model that times each word."
            } else {
                "Captions need a speech model."
            };
            return self.model_sheet(reason, ids, cx);
        }
        let total = targets.iter().map(|t| t.dur).sum::<f64>().ceil().max(1.) as u64;
        let title = match targets.as_slice() {
            [one] => one.name.clone(),
            _ => format!("{} clips", targets.len()),
        };
        let task = match crate::export_coordinator::AgentExport::begin(
            pm_promedia::tasks::TaskSpec {
                title,
                kind: pm_promedia::tasks::TaskKind::Captions,
                unit: pm_promedia::tasks::ProgressUnit::Seconds,
                priority: 0,
                total,
            },
            &published.project,
        ) {
            Ok(task) => Arc::new(task),
            Err(e) => return self.note(e, cx),
        };
        let cancel = task.cancel.clone();
        self.captions.generating = Some(cancel.clone());
        cx.notify();
        let profile = self.profile.clone();
        let doc = published.snapshot.doc_id;
        let row = task.clone();
        let job = cx.background_executor().spawn(async move {
            let progress: Arc<dyn Fn(f64) + Send + Sync> =
                Arc::new(move |p| row.progress((p * total as f64) as u64, total));
            generate(&published, &targets, &profile, &cancel, progress)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let task = Arc::try_unwrap(task).ok();
            let _ = this.update(cx, |v, cx| {
                v.captions.generating = None;
                let finish = |result: Result<(), String>| {
                    if let Some(task) = task {
                        task.finish(result);
                    }
                };
                if v.published.as_ref().map(|p| p.snapshot.doc_id) != Some(doc) {
                    finish(Err("The project changed before captions were ready.".into()));
                    return v.note("The project changed before captions were ready.".into(), cx);
                }
                match result {
                    Ok(Outcome::Captions {
                        cues,
                        count,
                        language,
                        name,
                    }) => {
                        finish(Ok(()));
                        let (id, command) =
                            crate::captions_panel::cues_command(&name, cues, language);
                        v.captions_apply(
                            "Generate Captions",
                            vec![command],
                            move |v, cx| {
                                v.captions_show_layer(&id, cx);
                                let noun = if count == 1 { "caption" } else { "captions" };
                                v.note(format!("Added {count} {noun}"), cx);
                            },
                            cx,
                        );
                    }
                    Ok(Outcome::Empty) => {
                        finish(Ok(()));
                        v.note("No speech was found in the selected clips.".into(), cx);
                    }
                    Err(Failure::Cancelled) => {
                        finish(Err("cancelled".into()));
                        v.note("Caption generation was cancelled.".into(), cx);
                    }
                    Err(e) => {
                        finish(Err(e.to_string()));
                        v.message = format!("Couldn’t generate captions. {e}");
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    /// S091 Download a Speech Model: the models that would let captions run,
    /// recommended first; Download keeps the sheet with its progress, and the
    /// generation continues once the model is ready.
    fn model_sheet(&mut self, reason: &str, ids: Vec<String>, cx: &mut Context<Self>) {
        use crate::dialogs::{Button, DialogId, Field, NativePiece, Surface, WindowId};
        use pm_services::surfaces::transcription::{ModelState, Status};
        let Some(store) = transcription::store() else {
            return;
        };
        let status: Status = match serde_json::from_value(store.status()) {
            Ok(status) => status,
            Err(_) => return self.note("Couldn’t read the transcription models.".into(), cx),
        };
        let mut offered: Vec<_> = status.offered(true).into_iter().cloned().collect();
        offered.sort_by_key(|m| !m.recommended);
        if offered.is_empty() {
            return self.note(reason.into(), cx);
        }
        let raw = store.status();
        let facts = |id: &str| {
            raw["models"]
                .as_array()
                .and_then(|m| m.iter().find(|m| m["id"] == id))
                .map(|m| {
                    let name = m["name"].as_str().unwrap_or(id);
                    let tag = if m["recommended"] == true {
                        " · Recommended"
                    } else {
                        ""
                    };
                    format!("{name}{tag} — {}", pm_builtins::speech::facts(m))
                })
                .unwrap_or_else(|| id.to_owned())
        };
        let mut field = Field::new(
            "model",
            "Speech Model",
            NativePiece::PopUpButton,
            offered.iter().map(|m| facts(&m.id)).collect(),
        );
        field.description = Some("Runs on this Mac. Your audio is never uploaded.".into());
        let project = self
            .session
            .as_ref()
            .map(|s| s.project.clone())
            .unwrap_or_default();
        let mut surface = Surface::sheet(
            DialogId(format!("speech-model:{project}")),
            WindowId(project),
            "Download a Speech Model",
            vec![field],
            "Download",
        );
        surface.line = Some(reason.into());
        surface.buttons = vec![Button::cancel(false), Button::action("Download")];
        let tokens = self.host.borrow().tokens.clone();
        let open = std::rc::Rc::new(crate::sheets::OpenSheet::present(
            crate::sheets::from_surface(&surface, &tokens),
            &self.host.borrow().window,
            None,
        ));
        let actions = open.actions.clone();
        let ids_after = ids.clone();
        cx.spawn(async move |this, cx| {
            let mut chosen: Option<String> = None;
            loop {
                // A download in progress: show it, and go on once it lands.
                if let Some(id) = &chosen {
                    let status = store.status();
                    let model = status["models"]
                        .as_array()
                        .and_then(|m| m.iter().find(|m| m["id"] == id.as_str()))
                        .cloned()
                        .unwrap_or_default();
                    match model["state"].as_str() {
                        Some("ready") => {
                            open.close();
                            let ids = ids_after.clone();
                            let _ = this.update(cx, |v, cx| v.generate_captions(Some(ids), cx));
                            return;
                        }
                        Some("error") => {
                            open.describe(
                                "model",
                                model["error"]
                                    .as_str()
                                    .unwrap_or("The download failed. Try again."),
                                tokens.colors.danger,
                            );
                            open.enable("apply", true);
                            chosen = None;
                        }
                        _ => {
                            let done = model["downloadedBytes"].as_u64().unwrap_or(0);
                            let size = model["size"].as_u64().unwrap_or(0);
                            open.describe(
                                "model",
                                &format!(
                                    "Downloading {} — {} of {}",
                                    model["name"].as_str().unwrap_or(""),
                                    pm_transcribe::store::format_bytes(done),
                                    pm_transcribe::store::format_bytes(size)
                                ),
                                tokens.colors.secondary,
                            );
                        }
                    }
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let Ok(action) = actions.try_recv() else {
                    if actions.is_closed() {
                        return;
                    }
                    continue;
                };
                match action.as_str() {
                    "cancel" => {
                        // The download keeps going; Settings › Transcription shows it.
                        return open.close();
                    }
                    "apply" if chosen.is_none() => {
                        let Some(model) = open.popup("model").and_then(|i| offered.get(i)) else {
                            continue;
                        };
                        let id = model.id.clone();
                        let ready = model.state == ModelState::Ready;
                        if !ready && let Err(e) = store.download(&id) {
                            open.describe("model", &e, tokens.colors.danger);
                            continue;
                        }
                        open.enable("apply", false);
                        chosen = Some(id);
                    }
                    _ => {}
                }
            }
        })
        .detach();
    }
    /// Settings › Transcription's Delete…: confirmed, then the files go.
    pub fn confirm_speech_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(store) = transcription::store() else {
            return;
        };
        let status = store.status();
        let Some(model) = status["models"]
            .as_array()
            .and_then(|m| m.iter().find(|m| m["id"] == id))
            .cloned()
        else {
            return;
        };
        let name = model["name"].as_str().unwrap_or(id).to_owned();
        let size = pm_transcribe::store::format_bytes(model["size"].as_u64().unwrap_or(0));
        use crate::alert::{Alert, Button};
        let alert = Alert::new(
            format!("Delete {name}?"),
            format!("This frees {size}. You can download it again at any time."),
            vec![
                Button::new("Cancel", false).default().cancel(),
                Button::new("Delete", true).destructive(),
            ],
        );
        let settings = self.builtins.windows.get("settings").copied();
        let window = settings
            .and_then(|h| {
                h.update(cx, |_, w, _| crate::platform::ns_window(w))
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| self.host.borrow().window.clone());
        let answer = alert.sheet(&window);
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            if answer.recv().await == Ok(Some(true)) {
                let removed = cx
                    .background_executor()
                    .spawn(async move { store.remove(&id) })
                    .await;
                if let Err(e) = removed {
                    let _ = this.update(cx, |v, cx| {
                        v.message = format!("Couldn’t delete {name}. {e}");
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_selected_clips_with_media_are_captionable() {
        let comp = json!({"layers":[
            {"id":"a","type":"video","name":"Interview","from":2,"dur":10,"d":{"asset":"m1","trim":3,"speed":{"v":2}}},
            {"id":"b","type":"text","name":"Title","from":0,"dur":5,"d":{}},
            {"id":"c","type":"audio","name":"VO","from":0,"dur":4,"d":{"asset":"m2","speed":{"v":1,"kf":[{"t":0,"v":1},{"t":2,"v":2}]}}},
            {"id":"d","type":"audio","name":"Empty","from":0,"dur":4,"d":{}}
        ]});
        let ids: Vec<String> = ["a", "b", "c", "d"].map(String::from).into();
        let found = targets(&comp, &ids);
        assert_eq!(
            found.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["Interview", "VO"]
        );
        assert_eq!(found[0].linear, Some((3., 2.)));
        assert_eq!(found[0].index, 0);
        assert_eq!(found[1].linear, None);
        assert!(targets(&comp, &["b".into()]).is_empty());
    }
}
