//! Window ▸ Panels ▸ Captions (S087) and the caption workflows (S093) over
//! pm-services `surfaces::captions`: the cue list of the captions layer
//! selected anywhere in the editor (else the one last chosen, else the
//! first), search, select-and-seek, inline edit, Add at Playhead, Split,
//! Delete, Import (SRT/WebVTT picker or drop) and Export Captions….
//! Every edit is one kernel transaction with today's Undo names.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_captions::{CaptionCue, CaptionFormat};
use pm_kernel::commands::captions::{
    AddCaptions, CaptionCommand, CaptionEdit, CaptionFormat as WireFormat, CueSource,
};
use pm_kernel::{Call, Caller, Command, Reply, Request, SelectionCall, WriteBody};
use pm_services::surfaces::captions::{self as backing, Layer, Selection, SelectionMode};
use pm_shell::MenuEntry;
use pm_shell::text::{TextEvent, TextField, TextKind};
use serde_json::{Value, json};
use std::{path::PathBuf, rc::Rc};

const CAPTION_EXTENSIONS: [&str; 2] = ["srt", "vtt"];

/// A file that imports as captions rather than media.
pub fn is_caption_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| CAPTION_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// "Interview.en.srt" → ("Interview", Some("en")); "clip.mov.srt" names its
/// video, not a language (today's `captionFileInfo`).
pub fn file_info(file_name: &str) -> (String, Option<String>) {
    let lower = file_name.to_ascii_lowercase();
    let base = if lower.ends_with(".srt") || lower.ends_with(".vtt") {
        &file_name[..file_name.len() - 4]
    } else {
        file_name
    };
    if let Some((name, ext)) = base.rsplit_once('.')
        && (2..=5).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && pm_builtins::media_import::extensions().contains(&ext.to_ascii_lowercase().as_str())
        && !name.is_empty()
    {
        return (name.into(), None);
    }
    if let Some((name, tag)) = base.rsplit_once('.')
        && !name.is_empty()
        && language_tag(tag)
    {
        return (name.into(), Some(tag.into()));
    }
    let name = if base.is_empty() { "Captions" } else { base };
    (name.into(), None)
}
/// `[a-z]{2,3}(-[A-Za-z0-9]{2,8})?`.
fn language_tag(tag: &str) -> bool {
    let (head, rest) = tag
        .split_once('-')
        .map_or((tag, None), |(h, r)| (h, Some(r)));
    (2..=3).contains(&head.len())
        && head.bytes().all(|b| b.is_ascii_lowercase())
        && rest.is_none_or(|r| {
            (2..=8).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}
/// Today's `PM.tc`: `HH:MM:SS:FF` at the composition rate.
pub fn timecode(seconds: f64, fps: f64) -> String {
    let fps = if fps.is_finite() && fps > 0. {
        fps
    } else {
        30.
    };
    let whole = fps.round().max(1.) as i64;
    let frames = (seconds.max(0.) * fps + 1e-6).floor() as i64;
    let (f, s) = (frames % whole, frames / whole);
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60, f)
}
/// Writes beside the target, then renames over it: a reader never sees half a file.
pub fn write_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let temp = path.with_file_name(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}
/// UTF-8 first; a file that is not valid UTF-8 is almost always Windows-1252.
pub fn decode_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text
            .strip_prefix('\u{feff}')
            .map(str::to_owned)
            .unwrap_or(text),
        Err(e) => e.into_bytes().iter().map(|b| windows_1252(*b)).collect(),
    }
}
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// The open composition's captions layers and what the panel draws them with.
#[derive(Clone)]
pub struct Doc {
    pub layers: Rc<[Layer]>,
    pub names: Rc<[String]>,
    /// Layer ids in `layers` order that are switched on (sidecars skip the rest).
    pub on: Rc<[bool]>,
    pub languages: Rc<[Option<String>]>,
    pub fps: f64,
    pub duration: f64,
    pub work: Option<[f64; 2]>,
    pub project: String,
}
fn wire(node: &pm_model::Node) -> Option<Value> {
    serde_json::from_str(&pm_model::json::to_canonical_string(&node.to_value())).ok()
}
/// Reads the published open composition (`None` without a document).
pub fn read(published: &pm_kernel::Published) -> Option<Doc> {
    let root = published.snapshot.root()?;
    let open = published.snapshot.open_comp();
    let comp = wire(pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root))?;
    let mut layers = vec![];
    let mut names = vec![];
    let mut on = vec![];
    let mut languages = vec![];
    for layer in comp["layers"].as_array().into_iter().flatten() {
        if layer["type"] != "captions" {
            continue;
        }
        let cues: Vec<CaptionCue> =
            serde_json::from_value(layer["d"]["cues"].clone()).unwrap_or_default();
        layers.push(Layer {
            id: layer["id"].as_str().unwrap_or_default().into(),
            from: layer["from"].as_f64().unwrap_or(0.),
            locked: layer["lock"] == true,
            cues,
        });
        names.push(layer["name"].as_str().unwrap_or("Captions").to_owned());
        on.push(layer["on"] != false);
        languages.push(layer["d"]["language"].as_str().map(str::to_owned));
    }
    let work = comp["work"]
        .as_array()
        .and_then(|w| Some([w.first()?.as_f64()?, w.get(1)?.as_f64()?]));
    Some(Doc {
        layers: layers.into(),
        names: names.into(),
        on: on.into(),
        languages: languages.into(),
        fps: comp["fps"].as_f64().filter(|f| *f > 0.).unwrap_or(30.),
        duration: comp["dur"].as_f64().unwrap_or(0.),
        work,
        project: root
            .get("name")
            .and_then(crate::node_string)
            .unwrap_or("Captions")
            .to_owned(),
    })
}

fn model(value: Value) -> pm_model::Value {
    pm_model::json::parse(&value.to_string()).unwrap_or(pm_model::Value::Null)
}
fn edit(layer: &str, edit: CaptionEdit) -> Command {
    CaptionCommand::Edit {
        layer_id: layer.into(),
        edit,
    }
    .into()
}
fn wire_format(format: CaptionFormat) -> WireFormat {
    match format {
        CaptionFormat::Srt => WireFormat::Srt,
        CaptionFormat::Vtt => WireFormat::Vtt,
    }
}
/// `add_captions` for a parsed file at `at` (composition seconds).
pub fn import_command(
    text: String,
    format: CaptionFormat,
    at: f64,
    name: String,
    language: Option<String>,
) -> (String, Command) {
    let id = pm_kernel::new_id("L");
    let command = CaptionCommand::Add(Box::new(AddCaptions {
        id: id.clone(),
        name: Some(name),
        source: CueSource::Text {
            text,
            format: wire_format(format),
            offset: at,
        },
        style: None,
        language,
        from: Some(at),
        duration: None,
        index: None,
        comp: None,
    }))
    .into();
    (id, command)
}
/// `add_captions` from cues in composition seconds (Generate, an empty layer).
pub fn cues_command(name: &str, cues: Value, language: Option<String>) -> (String, Command) {
    let id = pm_kernel::new_id("L");
    let command = CaptionCommand::Add(Box::new(AddCaptions {
        id: id.clone(),
        name: Some(name.into()),
        source: CueSource::Cues(model(cues)),
        style: None,
        language,
        from: Some(0.),
        duration: None,
        index: None,
        comp: None,
    }))
    .into();
    (id, command)
}
/// Add Caption at Playhead's cue: two seconds, cut at the next cue, refused
/// over a showing cue or with less than a frame of room.
pub fn add_span(layer: &Layer, at: f64, fps: f64) -> Result<(f64, f64), &'static str> {
    let local = at - layer.from;
    if pm_captions::cue_index_at(&layer.cues, local).is_some() {
        return Err("A caption is already showing here");
    }
    let next = layer.cues.iter().find(|c| c.start > local);
    let end = (at + 2.).min(next.map_or(f64::INFINITY, |c| c.start + layer.from));
    if end - at < 1. / fps.max(1.) {
        return Err("There is no room for a caption here");
    }
    Ok((at, end))
}
/// Text for one layer's file: composition time, optionally rebased to a range.
pub fn sidecar_text(
    layer: &Layer,
    format: CaptionFormat,
    range: Option<[f64; 2]>,
) -> Result<String, &'static str> {
    backing::export(layer, format, range).map_err(|_| "Couldn’t write the caption file.")
}
/// A file-name piece for one of several layers (today's sidecar slug).
pub fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    let out: String = out.chars().take(28).collect();
    let out = out.trim_end_matches(['-', '_']);
    if out.is_empty() {
        "captions".into()
    } else {
        out.into()
    }
}
/// Today's delivery sidecars: one per switched-on captions layer with cues,
/// `(suffix, text)`; several are told apart by language or name.
pub fn sidecars(
    doc: &Doc,
    format: CaptionFormat,
    range: [f64; 2],
) -> Vec<(Option<String>, String)> {
    let chosen: Vec<usize> = (0..doc.layers.len())
        .filter(|&i| doc.on[i] && !doc.layers[i].cues.is_empty())
        .collect();
    let mut used = std::collections::BTreeSet::new();
    chosen
        .iter()
        .filter_map(|&i| {
            let suffix = (chosen.len() > 1).then(|| {
                let base = slug(doc.languages[i].as_deref().unwrap_or(&doc.names[i]));
                let mut suffix = base.clone();
                let mut n = 2;
                while !used.insert(suffix.to_lowercase()) {
                    suffix = format!("{base}-{n}");
                    n += 1;
                }
                suffix
            });
            let text = backing::export(&doc.layers[i], format, Some(range)).ok()?;
            Some((suffix, text))
        })
        .collect()
}

struct Editing {
    layer: String,
    cue: String,
    original: String,
    field: Entity<TextField>,
    _events: Subscription,
}
#[derive(Default)]
pub struct State {
    cache: Option<(u64, u64, Doc)>,
    chosen: Option<String>,
    pub selection: Selection,
    query: String,
    search: Option<(Entity<TextField>, Subscription)>,
    editing: Option<Editing>,
    /// The keyboard cursor in the shown rows.
    cursor: Option<usize>,
    list_focused: bool,
    scroll: UniformListScrollHandle,
    followed: Option<usize>,
    focus: Option<FocusHandle>,
    pub generating: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

pub fn panel(
    v: &mut Controller,
    id: &str,
    bounds: pm_shell::layout::Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    (id == "captions").then(|| render(v, bounds, window, cx))
}

impl Controller {
    /// The open composition's captions layers at the current revision.
    pub fn captions_doc(&mut self) -> Option<Doc> {
        let published = self.published.clone()?;
        let stamp = (published.snapshot.doc_id, published.revision());
        if self
            .captions
            .cache
            .as_ref()
            .is_none_or(|c| (c.0, c.1) != stamp)
        {
            let doc = read(&published)?;
            self.captions.selection.prune(&doc.layers);
            self.captions.cache = Some((stamp.0, stamp.1, doc));
        }
        self.captions.cache.as_ref().map(|c| c.2.clone())
    }
    fn selected_layers(&self) -> Vec<String> {
        self.published
            .as_ref()
            .map(|p| p.selection.layers().to_vec())
            .unwrap_or_default()
    }
    /// S087: the captions layer selected anywhere, else the remembered one.
    fn captions_layer(&mut self) -> Option<Layer> {
        let doc = self.captions_doc()?;
        let selected = self.selected_layers();
        backing::choose_layer(&doc.layers, &selected, self.captions.chosen.as_deref()).cloned()
    }
    /// One edit as one Undo step; `then` runs once the kernel applied it.
    pub fn captions_apply(
        &mut self,
        label: &str,
        commands: Vec<Command>,
        then: impl FnOnce(&mut Controller, &mut Context<Controller>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = &self.session else { return };
        let revision = self.published.as_ref().map(|p| p.revision());
        let call = Call::Transact {
            label: Some(label.into()),
            expected_revision: revision,
            body: WriteBody::Commands(commands),
        };
        let pending = self
            .kernel
            .call(&session.project, Request::new(Caller::native(), call));
        let label = label.to_owned();
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Some(Ok(ack)) if !matches!(ack.reply, Reply::Throw(_)) => then(v, cx),
                    Some(Ok(ack)) => {
                        if let Reply::Throw(e) = ack.reply {
                            v.message = e.to_string();
                        }
                    }
                    _ => v.message = format!("Couldn’t {}", label.to_lowercase()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn select_only(&mut self, layer: &str, cx: &mut Context<Self>) {
        if self.selected_layers() != [layer.to_owned()] {
            self.call(
                SelectionCall::Layers {
                    ids: vec![layer.into()],
                    add: false,
                }
                .into(),
                cx,
            );
        }
    }
    /// Click (Shift extends, Command toggles) selects the cue, makes its layer
    /// the only selected one and moves the playhead to it.
    pub fn captions_select(&mut self, cue: &str, mode: SelectionMode, cx: &mut Context<Self>) {
        let Some(layer) = self.captions_layer() else {
            return;
        };
        self.captions_select_in(layer, cue, mode, cx);
    }
    fn captions_select_in(
        &mut self,
        layer: Layer,
        cue: &str,
        mode: SelectionMode,
        cx: &mut Context<Self>,
    ) {
        if let Some(time) = self.captions.selection.select(&layer, cue, mode) {
            self.captions.chosen = Some(layer.id.clone());
            self.select_only(&layer.id, cx);
            self.seek(time);
        }
        cx.notify();
    }
    /// Commands the panel, menus, palette and `--open` reach.
    pub fn captions_command(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match name {
            "addCaption" => self.add_caption(window, cx),
            "importCaptions" => self.pick_captions(cx),
            "exportCaptions" => self.export_captions_sheet(None, cx),
            "generateCaptions" => self.generate_captions(None, cx),
            "deleteCaptions" => return self.delete_cues(cx),
            "splitCaption" => return self.split_cue(cx),
            other if other.starts_with("captions.cue:") => {
                // The timeline's cue: `captions.cue:<verb>:<layer>\u{1f}<cue>`.
                let rest = &other["captions.cue:".len()..];
                let Some((verb, ids)) = rest.split_once(':') else {
                    return false;
                };
                let Some((layer, cue)) = ids.split_once('\u{1f}') else {
                    return false;
                };
                let mode = match verb {
                    "extend" => SelectionMode::Extend,
                    "toggle" => SelectionMode::Toggle,
                    _ => SelectionMode::Replace,
                };
                let Some(layer) = self
                    .captions_doc()
                    .and_then(|d| d.layers.iter().find(|l| l.id == layer).cloned())
                else {
                    return false;
                };
                self.captions_select_in(layer.clone(), cue, mode, cx);
                if verb == "edit" {
                    self.begin_edit(&layer, cue, window, cx);
                }
            }
            other => {
                let Some(id) = other.strip_prefix("captions.layer:") else {
                    return false;
                };
                self.captions.chosen = Some(id.into());
                self.select_only(id, cx);
                cx.notify();
            }
        }
        true
    }
    /// Shows a captions layer in the panel and makes it the selected layer.
    pub fn captions_show_layer(&mut self, layer: &str, cx: &mut Context<Self>) {
        self.captions.chosen = Some(layer.into());
        self.select_only(layer, cx);
        cx.notify();
    }
    fn delete_cues(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(layer) = self.captions_layer() else {
            return false;
        };
        let selected = self.selected_layers();
        let Some(stamp) = self.captions_stamp() else {
            return false;
        };
        if backing::delete_selected(stamp, &layer, &self.captions.selection, &selected).is_err() {
            return false;
        }
        let ids: Vec<String> = self.captions.selection.cues.iter().cloned().collect();
        let label = if ids.len() == 1 {
            "Delete Caption"
        } else {
            "Delete Captions"
        };
        self.captions_apply(
            label,
            vec![edit(&layer.id, CaptionEdit::Delete(ids))],
            |v, _| v.captions.selection.cues.clear(),
            cx,
        );
        true
    }
    fn split_cue(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(layer) = self.captions_layer() else {
            return false;
        };
        let time = self.time();
        let Some(stamp) = self.captions_stamp() else {
            return false;
        };
        let Ok(plan) = backing::split(stamp, &layer, &self.captions.selection, time) else {
            return false;
        };
        let id = plan.wire["id"].as_str().unwrap_or_default().to_owned();
        let tail = pm_kernel::new_id("cue");
        let command = edit(
            &layer.id,
            CaptionEdit::SplitWithId {
                id: id.clone(),
                at: time,
                tail_id: tail.clone(),
            },
        );
        let layer_id = layer.id.clone();
        self.captions_apply(
            "Split Caption",
            vec![command],
            move |v, _| {
                v.captions.selection.layer_id = Some(layer_id);
                v.captions.selection.cues = [id, tail].into_iter().collect();
            },
            cx,
        );
        true
    }
    fn captions_stamp(&self) -> Option<pm_services::surfaces::DocumentStamp> {
        let p = self.published.as_ref()?;
        Some(pm_services::surfaces::DocumentStamp {
            project: p.project.clone(),
            session: p.snapshot.doc_id,
            revision: p.revision(),
        })
    }
    /// A new cue at the playhead on the chosen layer (a new layer when there
    /// is none), then its editor.
    fn add_caption(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.captions_doc() else {
            return;
        };
        let at = self.time();
        let Some(layer) = self.captions_layer() else {
            // An empty layer from the composition start, then the cue on it.
            let (id, command) = cues_command("Captions", json!([]), None);
            let cue = pm_kernel::new_id("cue");
            let insert = edit(
                &id,
                CaptionEdit::Insert(model(
                    json!([{"id":cue,"start":at,"end":at + 2.,"text":"Caption"}]),
                )),
            );
            self.captions_apply(
                "Add Caption",
                vec![command, insert],
                move |v, cx| v.after_add(id, cue, cx),
                cx,
            );
            let _ = window;
            return;
        };
        if layer.locked {
            self.note("Unlock the captions layer to add a caption".into(), cx);
            return;
        }
        let (start, end) = match add_span(&layer, at, doc.fps) {
            Ok(span) => span,
            Err(message) => return self.note(message.into(), cx),
        };
        let cue = pm_kernel::new_id("cue");
        let command = edit(
            &layer.id,
            CaptionEdit::Insert(model(
                json!([{"id":cue,"start":start,"end":end,"text":"Caption"}]),
            )),
        );
        let id = layer.id.clone();
        self.captions_apply(
            "Add Caption",
            vec![command],
            move |v, cx| v.after_add(id, cue, cx),
            cx,
        );
    }
    fn after_add(&mut self, layer: String, cue: String, cx: &mut Context<Self>) {
        self.captions.chosen = Some(layer.clone());
        self.captions.selection.layer_id = Some(layer.clone());
        self.captions.selection.cues = [cue.clone()].into_iter().collect();
        self.select_only(&layer, cx);
        self.captions.pending_edit(layer, cue);
        cx.notify();
    }
    /// File ▸ Import Captions…: an open panel for `.srt` and `.vtt`.
    fn pick_captions(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let rx = crate::platform::import_paths(
            &self.host.borrow().window,
            false,
            false,
            &CAPTION_EXTENSIONS,
        );
        cx.spawn(async move |this, cx| {
            if let Ok(paths) = rx.recv().await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |v, cx| v.import_captions(path, 0., cx));
            }
        })
        .detach();
    }
    /// Reads, parses and adds one caption file as a new layer at `at`.
    pub fn import_captions(&mut self, path: PathBuf, at: f64, cx: &mut Context<Self>) {
        let Some(stamp) = self.captions_stamp() else {
            return;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let task = cx.background_executor().spawn(async move {
            let text = decode_text(std::fs::read(&path).ok()?);
            Some(backing::Import::parse(stamp, text, name))
        });
        cx.spawn(async move |this, cx| {
            let import = task.await;
            let _ = this.update(cx, |v, cx| {
                let Some(import) = import else {
                    return v.note("Couldn’t import captions.".into(), cx);
                };
                let Some(current) = v.captions_stamp() else {
                    return;
                };
                let at = at.max(0.);
                if import.command(&current, at).is_err() {
                    let message = if import.parsed.cues.is_empty() {
                        format!("No captions found in {}", import.name)
                    } else {
                        "Couldn’t import captions.".into()
                    };
                    v.message = message;
                    return cx.notify();
                }
                let count = import.parsed.cues.len();
                let skipped = import.parsed.skipped;
                let (name, language) = file_info(&import.name);
                let (id, command) =
                    import_command(import.original, import.parsed.format, at, name, language);
                v.captions_apply(
                    "Import Captions",
                    vec![command],
                    move |v, cx| {
                        v.captions.chosen = Some(id.clone());
                        v.select_only(&id, cx);
                        let noun = if count == 1 { "caption" } else { "captions" };
                        let skipped = if skipped > 0 {
                            format!(" · {skipped} unreadable skipped")
                        } else {
                            String::new()
                        };
                        v.note(format!("Imported {count} {noun}{skipped}"), cx);
                    },
                    cx,
                );
            });
        })
        .detach();
    }
    /// S093 Export Captions…: layer (with several), format and range (with a
    /// work area), then a save panel; the file is written off main.
    pub fn export_captions_sheet(&mut self, layer: Option<String>, cx: &mut Context<Self>) {
        use crate::dialogs::{DialogId, Field, NativePiece, Surface, WindowId};
        let Some(doc) = self.captions_doc() else {
            return;
        };
        if doc.layers.is_empty() {
            return self.note("There are no captions to export".into(), cx);
        }
        let selected = self.selected_layers();
        let target = layer
            .and_then(|id| doc.layers.iter().position(|l| l.id == id))
            .or_else(|| doc.layers.iter().position(|l| selected.contains(&l.id)))
            .unwrap_or(0);
        let mut fields = vec![];
        if doc.layers.len() > 1 {
            let mut field = Field::new(
                "layer",
                "Captions Layer",
                NativePiece::PopUpButton,
                doc.names.to_vec(),
            );
            field.selected = target;
            field.description = Some("Which captions to write.".into());
            fields.push(field);
        }
        fields.push(Field::new(
            "format",
            "Format",
            NativePiece::PopUpButton,
            vec!["SubRip (.srt)".into(), "WebVTT (.vtt)".into()],
        ));
        let work = doc
            .work
            .filter(|w| w[1] > w[0] && (w[0] > 0. || w[1] < doc.duration));
        if work.is_some() {
            let mut field = Field::new(
                "range",
                "Range",
                NativePiece::PopUpButton,
                vec!["Whole Composition".into(), "Work Area".into()],
            );
            field.description =
                Some("Times in the file start from the beginning of the range.".into());
            fields.push(field);
        }
        let project = self
            .session
            .as_ref()
            .map(|s| s.project.clone())
            .unwrap_or_default();
        let surface = Surface::sheet(
            DialogId(format!("captions-export:{project}")),
            WindowId(project),
            "Export Captions",
            fields,
            "Export…",
        );
        let tokens = self.host.borrow().tokens.clone();
        let open = Rc::new(crate::sheets::OpenSheet::present(
            crate::sheets::from_surface(&surface, &tokens),
            &self.host.borrow().window,
            None,
        ));
        let actions = open.actions.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                match action.as_str() {
                    "cancel" => return open.close(),
                    "apply" => {
                        let layer = open.popup("layer").unwrap_or(target);
                        let format = if open.popup("format") == Some(1) {
                            CaptionFormat::Vtt
                        } else {
                            CaptionFormat::Srt
                        };
                        let range = work.filter(|_| open.popup("range") == Some(1));
                        open.close();
                        let _ = this.update(cx, |v, cx| v.save_captions(layer, format, range, cx));
                        return;
                    }
                    _ => {}
                }
            }
        })
        .detach();
    }
    fn save_captions(
        &mut self,
        index: usize,
        format: CaptionFormat,
        range: Option<[f64; 2]>,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.captions_doc() else {
            return;
        };
        let Some(layer) = doc.layers.get(index).cloned() else {
            return;
        };
        let text = match sidecar_text(&layer, format, range) {
            Ok(text) => text,
            Err(e) => return self.note(e.into(), cx),
        };
        let extension = if format == CaptionFormat::Vtt {
            "vtt"
        } else {
            "srt"
        };
        let base = if doc.layers.len() > 1 {
            format!("{} - {}", doc.project, doc.names[index])
        } else {
            doc.project.clone()
        }
        .replace(['/', '\\', ':'], "-");
        let rx = crate::platform::export_destination(
            &self.host.borrow().window,
            false,
            &format!("{base}.{extension}"),
        );
        cx.spawn(async move |this, cx| {
            let Ok(Some(path)) = rx.recv().await else {
                return;
            };
            let written = cx
                .background_executor()
                .spawn(async move { write_file(&path, text.as_bytes()) })
                .await;
            let _ = this.update(cx, |v, cx| match written {
                Ok(()) => v.note(
                    format!(
                        "Exported {} captions",
                        if extension == "vtt" { "WebVTT" } else { "SRT" }
                    ),
                    cx,
                ),
                Err(_) => {
                    v.message = "Couldn’t write the caption file.".into();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// The panel's actions menu (header ⋯ in v1.2.0's panel).
    fn captions_menu(&mut self, at: Point<Pixels>) {
        let items = [
            MenuEntry::item("Add Caption at Playhead", "addCaption"),
            MenuEntry::separator(),
            MenuEntry::item("Import Captions…", "importCaptions"),
            MenuEntry::item("Generate Captions", "generateCaptions"),
            MenuEntry::item("Export Captions…", "exportCaptions"),
        ];
        self.host
            .borrow()
            .context_menu(&items, f32::from(at.x), f32::from(at.y));
    }
    fn begin_edit(
        &mut self,
        layer: &Layer,
        cue: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if layer.locked {
            return self.note("Unlock the captions layer to edit it".into(), cx);
        }
        let Some(found) = layer.cues.iter().find(|c| c.id == cue) else {
            return;
        };
        self.finish_edit(true, cx);
        let tokens = self.host.borrow().tokens.clone();
        let original = found.text.clone();
        let field = cx
            .new(|cx| TextField::new(TextKind::Composer, "Caption", &original, tokens, window, cx));
        let events = cx.subscribe(&field, |v, _, event: &TextEvent, cx| match event {
            TextEvent::Submitted(_) | TextEvent::Committed(_) => v.finish_edit(true, cx),
            TextEvent::Cancelled => v.finish_edit(false, cx),
            _ => {}
        });
        field.update(cx, |f, cx| f.begin_editing_selecting_all(cx));
        self.captions.editing = Some(Editing {
            layer: layer.id.clone(),
            cue: cue.into(),
            original,
            field,
            _events: events,
        });
        cx.notify();
    }
    /// Commits (or drops) the open editor; an unchanged text writes nothing.
    fn finish_edit(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(editing) = self.captions.editing.take() else {
            return;
        };
        let text = editing.field.read(cx).value().to_owned();
        cx.notify();
        if !commit || text == editing.original {
            return;
        }
        let Some(doc) = self.captions_doc() else {
            return;
        };
        let Some(layer) = doc.layers.iter().find(|l| l.id == editing.layer) else {
            return;
        };
        let Some(stamp) = self.captions_stamp() else {
            return;
        };
        let Ok(mut draft) = backing::CueDraft::begin(stamp.clone(), layer, &editing.cue) else {
            return;
        };
        // Another edit since the editor opened leaves the person's text unapplied.
        if layer
            .cues
            .iter()
            .find(|c| c.id == editing.cue)
            .map(|c| &c.text)
            != Some(&editing.original)
        {
            return self.note("The caption changed while you were editing it".into(), cx);
        }
        draft.text = text.clone();
        if draft.command(&stamp, layer).is_err() {
            return;
        }
        let command = edit(
            &layer.id,
            CaptionEdit::Update(model(json!([{"id":editing.cue,"text":text}]))),
        );
        self.captions_apply("Edit Caption", vec![command], |_, _| {}, cx);
    }
}
impl State {
    /// An added cue opens its editor on the next draw.
    fn pending_edit(&mut self, layer: String, cue: String) {
        self.selection.layer_id = Some(layer);
        self.selection.cues = [cue].into_iter().collect();
        self.cursor = None;
        self.followed = None;
        PENDING.with(|p| *p.borrow_mut() = true);
    }
}
thread_local! {
    static PENDING: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
}

fn render(
    v: &mut Controller,
    bounds: pm_shell::layout::Rect,
    window: &mut Window,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    let height = (bounds.h - m.header).max(0.);
    let this = cx.entity();
    let focus = v
        .captions
        .focus
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    let Some(doc) = v.captions_doc() else {
        return div().into_any_element();
    };
    let selected = v.selected_layers();
    let Some(layer) =
        backing::choose_layer(&doc.layers, &selected, v.captions.chosen.as_deref()).cloned()
    else {
        let button = |label: &'static str, command: &'static str| {
            div()
                .id(command)
                .role(Role::Button)
                .aria_label(label)
                .h(px(m.control))
                .px(px(m.inset))
                .flex()
                .items_center()
                .rounded(px(m.control_radius))
                .bg(rgb(c.field))
                .text_size(px(ty.reading))
                .text_color(rgb(c.text))
                .child(label)
                .on_click(cx.listener(move |v, _, window, cx| {
                    v.captions_command(command, window, cx);
                }))
        };
        return pm_shell::view::empty_state(
            &t,
            "No Captions",
            "Import an SRT or WebVTT file, or select a clip and generate captions from its speech.",
        )
        .h(px(height))
        .child(
            div()
                .mt(px(m.gap))
                .flex()
                .gap(px(m.gap))
                .child(button("Import…", "importCaptions"))
                .child(button("Generate", "generateCaptions")),
        )
        .into_any_element();
    };
    // An added cue opens its editor once the layer shows it.
    if PENDING.with(|p| std::mem::take(&mut *p.borrow_mut())) {
        if let Some(cue) = v.captions.selection.cues.iter().next().cloned()
            && layer.cues.iter().any(|c| c.id == cue)
        {
            v.begin_edit(&layer, &cue, window, cx);
        } else {
            PENDING.with(|p| *p.borrow_mut() = true);
        }
    }
    let time = v.time();
    let playing = v.viewer.as_ref().is_some_and(|viewer| {
        viewer
            .status()
            .clock
            .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
    });
    let shown: Rc<[(usize, CaptionCue)]> = backing::shown(&layer, &v.captions.query)
        .into_iter()
        .map(|(i, c)| (i, c.clone()))
        .collect();
    let under = pm_captions::cue_index_at(&layer.cues, time - layer.from);
    // Keep the spoken cue in view while playing, unless the person is busy.
    let follow = backing::follow(
        &layer,
        &v.captions.query,
        time,
        playing,
        v.captions.editing.is_some(),
        v.captions.list_focused && focus.is_focused(window),
    );
    if follow.is_some() && follow != v.captions.followed {
        v.captions
            .scroll
            .scroll_to_item(follow.unwrap(), ScrollStrategy::Nearest);
    }
    v.captions.followed = follow;
    if playing {
        // Redraw while playing so the mark follows the playhead.
        window.request_animation_frame();
    }
    let search = v
        .captions
        .search
        .get_or_insert_with(|| {
            let field = cx.new(|cx| {
                let mut f = TextField::new(
                    TextKind::Primary,
                    "Search captions",
                    "",
                    t.clone(),
                    window,
                    cx,
                );
                f.set_placeholder("Search captions");
                f
            });
            let events = cx.subscribe(&field, |v, _, event: &TextEvent, cx| match event {
                TextEvent::Changed(q) | TextEvent::Committed(q) => {
                    v.captions.query = q.clone();
                    v.captions.cursor = None;
                    cx.notify();
                }
                TextEvent::Cancelled => {
                    v.captions.query.clear();
                    if let Some((f, _)) = &v.captions.search {
                        f.update(cx, |f, cx| f.set_value("", cx));
                    }
                    cx.notify();
                }
                _ => {}
            });
            (field, events)
        })
        .0
        .clone();
    let selection: std::collections::BTreeSet<String> =
        if v.captions.selection.layer_id.as_deref() == Some(&layer.id) {
            v.captions.selection.cues.clone()
        } else {
            Default::default()
        };
    let fps = doc.fps;
    let wash = rgba(c.text << 8 | (t.translucency.selected * 255.).round() as u32);
    let row_h = m.row;
    let rows = {
        let shown = shown.clone();
        let layer = layer.clone();
        let this = this.clone();
        let tokens = t.clone();
        let cursor = v.captions.cursor;
        uniform_list("caption-cues", shown.len(), move |range, _, _| {
            let (c, m, ty) = (&tokens.colors, &tokens.metrics, &tokens.typography);
            range
                .map(|position| {
                    let (index, cue) = &shown[position];
                    let now = under == Some(*index);
                    let on = selection.contains(&cue.id);
                    let id = cue.id.clone();
                    let text = cue.text.replace('\n', " ");
                    let tc = timecode(layer.from + cue.start, fps);
                    let (this, layer) = (this.clone(), layer.clone());
                    div()
                        .id(SharedString::from(format!("cue-{}", cue.id)))
                        .role(Role::Row)
                        .aria_label(format!(
                            "{tc}, {text}{}",
                            if now { ", now playing" } else { "" }
                        ))
                        .h(px(row_h))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(m.gap))
                        .px(px(m.inset))
                        .relative()
                        .when(on, |d| d.bg(wash))
                        .when(cursor == Some(position), |d| {
                            d.border_1().border_color(rgb(c.focus))
                        })
                        .when(now, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(m.border * 2.))
                                    .bottom(px(m.border * 2.))
                                    .w(px(2.))
                                    .bg(rgb(c.accent)),
                            )
                        })
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(ty.body))
                                .text_color(rgb(c.secondary))
                                .font_features(FontFeatures(std::sync::Arc::new(vec![(
                                    "tnum".into(),
                                    1,
                                )])))
                                .child(tc),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(ty.body))
                                .text_color(rgb(c.text))
                                .when(now, |d| d.font_weight(FontWeight(ty.medium_weight)))
                                .child(text),
                        )
                        .on_click(move |e: &ClickEvent, window, cx| {
                            let modifiers = e.modifiers();
                            let mode = if modifiers.shift {
                                SelectionMode::Extend
                            } else if modifiers.platform {
                                SelectionMode::Toggle
                            } else {
                                SelectionMode::Replace
                            };
                            let double =
                                matches!(e, ClickEvent::Mouse(m) if m.down.click_count >= 2);
                            let (id, layer) = (id.clone(), layer.clone());
                            this.update(cx, |v, cx| {
                                v.captions.cursor = Some(position);
                                v.captions.list_focused = true;
                                if let Some(f) = &v.captions.focus {
                                    window.focus(f, cx);
                                }
                                if double {
                                    v.begin_edit(&layer, &id, window, cx);
                                } else {
                                    v.finish_edit(true, cx);
                                    v.captions_select(&id, mode, cx);
                                }
                            });
                        })
                })
                .collect()
        })
        .track_scroll(&v.captions.scroll)
        .flex_1()
    };
    // The editor sits over its row, growing down with its lines.
    let editor = v.captions.editing.as_ref().and_then(|e| {
        let position = shown.iter().position(|(_, c)| c.id == e.cue)?;
        let offset = f32::from(v.captions.scroll.0.borrow().base_handle.offset().y);
        Some(
            div()
                .absolute()
                .left(px(m.inset))
                .right(px(m.inset))
                .top(px(position as f32 * row_h + offset))
                .child(e.field.clone()),
        )
    });
    let list_body = if layer.cues.is_empty() {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(ty.reading))
            .text_color(rgb(c.secondary))
            .child("This captions layer is empty.")
            .into_any_element()
    } else if shown.is_empty() {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(ty.reading))
            .text_color(rgb(c.secondary))
            .child(format!("No captions match “{}”", v.captions.query.trim()))
            .into_any_element()
    } else {
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .relative()
            .child(rows)
            .children(editor)
            .into_any_element()
    };
    let chooser = (doc.layers.len() > 1).then(|| {
        let name = doc
            .layers
            .iter()
            .position(|l| l.id == layer.id)
            .map(|i| doc.names[i].clone())
            .unwrap_or_default();
        let entries: Vec<MenuEntry> = doc
            .layers
            .iter()
            .zip(doc.names.iter())
            .map(|(l, n)| {
                MenuEntry::item(n.clone(), format!("captions.layer:{}", l.id))
                    .checked(l.id == layer.id)
            })
            .collect();
        div()
            .id("captions-layer")
            .role(Role::ComboBox)
            .aria_label("Captions Layer")
            .mx(px(m.inset))
            .h(px(m.field))
            .px(px(m.gap))
            .flex()
            .items_center()
            .rounded(px(m.control_radius))
            .bg(rgb(c.field))
            .text_size(px(ty.body))
            .text_color(rgb(c.text))
            .child(div().flex_1().truncate().child(name))
            .child(pm_shell::symbol::symbol(
                "chevron.up.chevron.down",
                m.symbol_panel,
                pm_shell::symbol::Weight::Medium,
                c.secondary,
                window,
            ))
            .on_click(cx.listener(move |v, e: &ClickEvent, _, _| {
                let at = e.position();
                v.host
                    .borrow()
                    .context_menu(&entries, f32::from(at.x), f32::from(at.y));
            }))
    });
    let actions = div()
        .id("caption-actions")
        .role(Role::Button)
        .aria_label("Caption Actions")
        .tooltip(pm_shell::view::tooltip(&t, "Caption Actions"))
        .flex_none()
        .size(px(m.hit))
        .flex()
        .items_center()
        .justify_center()
        .child(pm_shell::symbol::symbol(
            "ellipsis.circle",
            m.symbol_panel,
            pm_shell::symbol::Weight::Medium,
            c.secondary,
            window,
        ))
        .on_click(cx.listener(|v, e: &ClickEvent, _, cx| {
            cx.stop_propagation();
            v.captions_menu(e.position());
        }));
    let shown_len = shown.len();
    let keyed = shown.clone();
    let key_layer = layer.clone();
    div()
        .id("captions")
        .track_focus(&focus)
        .key_context("CaptionsPanel")
        .w_full()
        .h(px(height))
        .flex()
        .flex_col()
        .gap(px(m.border * 2.))
        .pt(px(m.border * 2.))
        .on_key_down(cx.listener(move |v, e: &KeyDownEvent, window, cx| {
            if v.captions.editing.is_some() {
                return;
            }
            let cursor = v.captions.cursor;
            let key = e.keystroke.key.as_str();
            match key {
                "up" | "down" if shown_len > 0 => {
                    let next = match (cursor, key) {
                        (None, _) => 0,
                        (Some(i), "up") => i.saturating_sub(1),
                        (Some(i), _) => (i + 1).min(shown_len - 1),
                    };
                    v.captions.cursor = Some(next);
                    v.captions
                        .scroll
                        .scroll_to_item(next, ScrollStrategy::Nearest);
                    let mode = if e.keystroke.modifiers.shift {
                        SelectionMode::Extend
                    } else {
                        SelectionMode::Replace
                    };
                    let id = keyed[next].1.id.clone();
                    v.captions_select(&id, mode, cx);
                }
                "enter" => {
                    if let Some(i) = cursor.filter(|i| *i < shown_len) {
                        let id = keyed[i].1.id.clone();
                        v.begin_edit(&key_layer, &id, window, cx);
                    }
                }
                "backspace" | "delete" => {
                    if v.delete_cues(cx) {
                        cx.stop_propagation();
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|v, _, _, cx| v.finish_edit(true, cx)),
        )
        .children(chooser)
        .child(
            div()
                .mx(px(m.inset))
                .flex()
                .items_center()
                .gap(px(m.gap))
                .child(div().flex_1().child(search))
                .child(actions),
        )
        .child(list_body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layer() -> Layer {
        Layer {
            id: "L".into(),
            from: 10.,
            locked: false,
            cues: vec![
                CaptionCue {
                    id: "a".into(),
                    start: 0.,
                    end: 1.,
                    text: "One".into(),
                    words: None,
                },
                CaptionCue {
                    id: "b".into(),
                    start: 2.5,
                    end: 4.,
                    text: "Two".into(),
                    words: None,
                },
            ],
        }
    }
    #[test]
    fn caption_files_name_their_language_but_not_their_video() {
        assert_eq!(
            file_info("Interview.en.srt"),
            ("Interview".into(), Some("en".into()))
        );
        assert_eq!(
            file_info("Talk.pt-BR.vtt"),
            ("Talk".into(), Some("pt-BR".into()))
        );
        assert_eq!(file_info("clip.mov.srt"), ("clip".into(), None));
        assert_eq!(file_info("Plain.SRT"), ("Plain".into(), None));
        assert_eq!(file_info(".srt"), ("Captions".into(), None));
        assert!(is_caption_file(std::path::Path::new("/a/b.VTT")));
        assert!(!is_caption_file(std::path::Path::new("/a/b.txt")));
    }
    #[test]
    fn timecode_counts_whole_frames() {
        assert_eq!(timecode(1.5, 24.), "00:00:01:12");
        assert_eq!(timecode(3661.0, 30.), "01:01:01:00");
        assert_eq!(timecode(-1., 30.), "00:00:00:00");
        assert_eq!(timecode(0.999, 29.97), "00:00:00:29");
    }
    #[test]
    fn windows_1252_text_survives_import() {
        assert_eq!(decode_text(b"caf\xe9 \x93hi\x94".to_vec()), "café “hi”");
        assert_eq!(decode_text("\u{feff}ok".as_bytes().to_vec()), "ok");
    }
    #[test]
    fn add_at_playhead_takes_two_seconds_up_to_the_next_cue() {
        let l = layer();
        assert_eq!(add_span(&l, 11.2, 30.), Ok((11.2, 12.5)));
        assert_eq!(add_span(&l, 14.5, 30.), Ok((14.5, 16.5)));
        assert_eq!(
            add_span(&l, 10.5, 30.),
            Err("A caption is already showing here")
        );
        assert_eq!(
            add_span(&l, 12.49, 30.),
            Err("There is no room for a caption here")
        );
    }
    #[test]
    fn several_layers_write_one_named_sidecar_each() {
        let mut other = layer();
        other.id = "M".into();
        let doc = Doc {
            layers: vec![
                layer(),
                other.clone(),
                Layer {
                    cues: vec![],
                    ..other
                },
            ]
            .into(),
            names: vec!["English".into(), "English".into(), "Empty".into()].into(),
            on: vec![true, true, true].into(),
            languages: vec![Some("en".into()), None, None].into(),
            fps: 30.,
            duration: 20.,
            work: None,
            project: "Film".into(),
        };
        let files = sidecars(&doc, CaptionFormat::Srt, [0., 20.]);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0.as_deref(), Some("en"));
        assert_eq!(files[1].0.as_deref(), Some("English"));
        assert!(files[0].1.contains("00:00:10,000 --> 00:00:11,000"));
        let one = Doc {
            layers: vec![layer()].into(),
            names: vec!["Captions".into()].into(),
            on: vec![true].into(),
            languages: vec![None].into(),
            ..doc
        };
        let files = sidecars(&one, CaptionFormat::Vtt, [10., 20.]);
        assert_eq!(files[0].0, None);
        assert!(files[0].1.contains("00:00:00.000 --> 00:00:01.000"));
        assert_eq!(slug("Français (CA)"), "Fran-ais-CA");
    }
}
