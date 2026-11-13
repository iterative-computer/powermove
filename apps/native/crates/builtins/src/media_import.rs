//! Import Media. Probing and byte retention run on the caller's worker. The
//! kernel admits `RetainedAsset` receipts and any placed layers as one
//! transaction: one Undo step, and no asset without its layer.
use crate::{Builtins, RetainedAsset};
use pm_kernel::{
    Caller, Command, KernelError, LayerCommand, Published, Reply, Request, SelectionCall,
    TransactionError, media_archive::Archive,
};
use pm_model::{Map, Value};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// `shared/media-formats.ts`. Formats the engine decodes natively need no proxy.
const IMAGES: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "svg", "tif", "tiff", "heic", "heif",
];
const AUDIO: &[&str] = &[
    "wav", "mp3", "m4a", "aac", "ogg", "oga", "opus", "weba", "flac", "aif", "aiff",
];
const VIDEO: &[&str] = &[
    "mp4", "mov", "m4v", "webm", "mkv", "avi", "wmv", "asf", "flv", "mpg", "mpeg", "m2v", "mts",
    "m2ts", "ts", "3gp", "3g2", "mxf", "ogv", "dv", "vob",
];
/// Every extension Import Media accepts, for the open panel's filter.
pub fn extensions() -> Vec<&'static str> {
    [IMAGES, AUDIO, VIDEO].concat()
}

/// An HTML `accept` list (`image/*`, `.png`, `video/mp4`) as open-panel
/// extensions; empty when any file is allowed.
pub fn accept_extensions(accept: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for token in accept.split(',').map(|t| t.trim().to_ascii_lowercase()) {
        let listed: &[&str] = match token.as_str() {
            "" => continue,
            "image/*" => IMAGES,
            "video/*" => VIDEO,
            "audio/*" => AUDIO,
            "image/jpeg" => &["jpg", "jpeg"],
            "image/svg+xml" => &["svg"],
            "image/tiff" => &["tif", "tiff"],
            "video/quicktime" => &["mov"],
            "video/x-matroska" => &["mkv"],
            "audio/mpeg" => &["mp3"],
            "audio/mp4" => &["m4a"],
            "audio/wav" | "audio/x-wav" => &["wav"],
            "application/json" => &["json"],
            "text/plain" => &["txt"],
            other => {
                match other.strip_prefix('.').or_else(|| other.split('/').nth(1)) {
                    Some(extension) if !extension.is_empty() && extension != "*" => {
                        out.push(extension.into())
                    }
                    // `*/*` or an unknown wildcard: anything goes.
                    _ => return vec![],
                }
                continue;
            }
        };
        out.extend(listed.iter().map(|e| e.to_string()));
    }
    out.dedup();
    out
}
fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}
fn kind(path: &Path) -> Option<&'static str> {
    let extension = extension(path);
    let has = |list: &[&str]| list.contains(&extension.as_str());
    [(IMAGES, "image"), (VIDEO, "video"), (AUDIO, "audio")]
        .into_iter()
        .find_map(|(list, kind)| has(list).then_some(kind))
}
fn mime(path: &Path, kind: &str) -> String {
    let extension = extension(path);
    match (kind, extension.as_str()) {
        ("image", "jpg" | "jpeg") => "image/jpeg".into(),
        ("image", "svg") => "image/svg+xml".into(),
        ("image", "tif") => "image/tiff".into(),
        ("video", "mov") => "video/quicktime".into(),
        ("video", "mkv") => "video/x-matroska".into(),
        ("audio", "mp3") => "audio/mpeg".into(),
        ("audio", "m4a") => "audio/mp4".into(),
        ("audio", "wav") => "audio/wav".into(),
        _ => format!("{kind}/{extension}"),
    }
}

/// The kind Import Media gives a file and the type its bytes are kept as.
pub fn classify(path: &Path) -> Option<(&'static str, String)> {
    kind(path).map(|kind| (kind, mime(path, kind)))
}

/// Folders expand to their media, recursively and in name order.
pub fn expand(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = vec![];
    for path in paths {
        if path.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|p| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .collect();
            entries.sort();
            out.extend(expand(entries).into_iter().filter(|p| kind(p).is_some()));
        } else {
            out.push(path);
        }
    }
    out
}

/// File ▸ Import Folder…: each chosen folder becomes a Project folder with
/// its subfolders, in name order. A folder with no media anywhere inside is
/// left out. Answers the folders (parents first) and every media file with
/// the folder it goes in; a loose file goes to the destination.
pub fn folder_tree(
    paths: Vec<PathBuf>,
) -> (
    Vec<pm_panel_media::NewFolder>,
    Vec<(PathBuf, Option<String>)>,
) {
    fn walk(
        paths: Vec<PathBuf>,
        parent: Option<&str>,
        folders: &mut Vec<pm_panel_media::NewFolder>,
        files: &mut Vec<(PathBuf, Option<String>)>,
    ) {
        for path in paths {
            if !path.is_dir() {
                files.push((path, parent.map(str::to_owned)));
                continue;
            }
            let mut entries: Vec<_> = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|p| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                        && (p.is_dir() || kind(p).is_some())
                })
                .collect();
            // A folder's own files come before its subfolders, each by name.
            entries.sort_by_key(|p| (p.is_dir(), p.clone()));
            let (at, had) = (folders.len(), files.len());
            let id = pm_kernel::new_id("f");
            folders.push(pm_panel_media::NewFolder {
                id: id.clone(),
                name: path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                parent: parent.map(str::to_owned),
            });
            walk(entries, Some(&id), folders, files);
            if files.len() == had {
                folders.truncate(at);
            }
        }
    }
    let (mut folders, mut files) = (vec![], vec![]);
    walk(paths, None, &mut folders, &mut files);
    (folders, files)
}

/// `assetIdentity`: what the Project panel and layers read from an asset,
/// and the file's audio streams. Lengths are measured as an asset with
/// `timing` reads the file.
fn metadata(
    path: &Path,
    kind: &str,
    size: u64,
    timing: pm_media::clip::ClipTiming,
) -> Result<(Value, Vec<pm_protocol::media::AudioStreamFacts>), String> {
    let facts = pm_media::facts::media_facts_as(path, None, timing).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("The file has no name")?;
    let [w, h] = facts.display_size.unwrap_or([0, 0]);
    let (channels, sample_rate) = facts
        .audio
        .as_ref()
        .map_or((0, 0), |a| (a.channels, a.sample_rate));
    let mut asset = json!({
        "name": name,
        "kind": kind,
        "size": size,
        "dur": facts.duration.unwrap_or(0.),
        "w": w,
        "h": h,
        "channels": channels,
        "sampleRate": sample_rate,
        "playbackProxy": false,
        "playbackProxyVersion": 0,
        "persisted": true,
        "sourcePath": path.to_string_lossy(),
    });
    if kind == "video" {
        asset["hasAudio"] = json!(facts.audio.is_some());
        asset["audioDur"] = json!(facts.audio_duration.unwrap_or(0.));
    }
    // A file with several audio streams (camera and recorder tracks) lists
    // them all, each on: every one is heard until it is switched off.
    if let Some(streams) = pm_media::facts::asset_audio_streams(&facts.audio_streams) {
        asset["audioStreams"] = streams;
    }
    if kind == "image" {
        asset["format"] = json!(extension(path));
    }
    Ok((crate::data::model(&asset)?, facts.audio_streams))
}

/// How an asset record reads its file's times (its `timing` member).
pub fn asset_timing(record: Option<&serde_json::Value>) -> pm_media::clip::ClipTiming {
    pm_media::clip::ClipTiming::of_asset(record.and_then(|r| r["timing"].as_f64()))
}

/// A relinked asset's `audioStreams`: the old member reconciled with the new
/// file (`pm_media::facts::relinked_audio_streams`). The new bytes are the
/// old ones when their hashes agree; without retained old bytes that cannot
/// be told.
pub fn relink_streams(
    asset: &mut serde_json::Value,
    old: Option<&serde_json::Value>,
    streams: &[pm_protocol::media::AudioStreamFacts],
    previous: Option<&pm_kernel::media_archive::Original>,
    original: &pm_kernel::media_archive::Original,
) {
    let same = previous.map(|previous| previous.hash() == original.hash());
    let Some(asset) = asset.as_object_mut() else {
        return;
    };
    match pm_media::facts::relinked_audio_streams(old, streams, same) {
        Some(list) => asset.insert("audioStreams".into(), list),
        None => asset.remove("audioStreams"),
    };
}

/// What the probe proves before any byte is retained.
fn decodes(path: &Path, kind: &str) -> Result<(), String> {
    crate::delivery::probe_media(path, kind)
}

#[derive(Debug, Default)]
pub struct ImportRequest {
    pub paths: Vec<PathBuf>,
    /// Destination Project folder (`None`: the root).
    pub folder: Option<String>,
    /// Relink: exactly one file replaces this asset's bytes.
    pub replace: Option<String>,
    /// `placement`: a layer per file at this time. `None`: Project panel only.
    pub place_at: Option<f64>,
    /// An extension's `assets.import`: 3D models (glb, glTF, OBJ) are media too.
    pub models: bool,
    /// The layer definition an extension asked its asset to place as.
    pub layer_definition: Option<String>,
    /// Import Image Sequence: `paths` are the frames, encoded into one clip.
    pub sequence: Option<SequenceImport>,
    /// The placed layer's name and fitted size, instead of the file's.
    pub placed_as: Option<PlacedAs>,
    /// Scratch holding generated `paths`, removed once the import is done.
    pub staged: Option<tempfile::TempDir>,
    /// What the status bar says once it lands, instead of the summary.
    pub note: Option<String>,
    /// Import Folder: chosen folders stay folders in the Project panel.
    pub keep_folders: bool,
}
/// Figma's image fallback: the layer is named for the selection and fitted
/// into the selection's size, keeping the image's aspect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacedAs {
    pub name: String,
    pub fit: Option<(f64, f64)>,
}
/// File ▸ Import Image Sequence…'s choice, with the line its progress reads from.
#[derive(Debug, Default)]
pub struct SequenceImport {
    pub fps: f64,
    pub progress: Arc<Mutex<String>>,
}
/// `chooseSequence`'s ordering, in today's words: frame-number order, gaps
/// skipped (the available images play consecutively).
pub fn order_sequence(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    use pm_media::image::sequence::ImageSequence;
    if paths.len() < 2 {
        return Err("Choose at least 2 numbered image frames".into());
    }
    match ImageSequence::from_paths(paths) {
        Ok(sequence) => Ok(sequence.frames.into_iter().map(|(_, path)| path).collect()),
        Err(error) => {
            let error = error.to_string();
            Err(match error.split("duplicate frame number ").nth(1) {
                Some(number) => format!("Duplicate frame number {number}"),
                None => "Choose images with the same name and extension, ending in frame \
                    numbers (for example frame_0001.png)"
                    .into(),
            })
        }
    }
}
/// `<prefix> sequence.webm`, the clip's name in the Project panel.
pub fn sequence_name(first: &Path) -> String {
    let name = first
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    let prefix = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let prefix = prefix.trim_end_matches(['-', '_', '.', ' ']);
    format!(
        "{} sequence.webm",
        if prefix.is_empty() { "Image" } else { prefix }
    )
}
/// The frames encoded into a scratch WebM (removed when dropped), and the
/// asset facts today's importer adds to it.
struct EncodedSequence {
    _dir: tempfile::TempDir,
    path: PathBuf,
    fps: f64,
    frames: u64,
}
fn encode_sequence(
    paths: &[PathBuf],
    sequence: &SequenceImport,
    cancel: &AtomicBool,
) -> Result<EncodedSequence, String> {
    use pm_export::stills::{Stage, encode_still_sequence};
    let frames = order_sequence(paths)?;
    let dir = tempfile::Builder::new()
        .prefix("powermove-image-sequence-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let path = dir.path().join(sequence_name(&frames[0]));
    let say = |line: String| {
        if let Ok(mut progress) = sequence.progress.lock() {
            *progress = line;
        }
    };
    let encoded = encode_still_sequence(&frames, sequence.fps, &path, cancel, &mut |stage| {
        say(match stage {
            Stage::Checking { done, total } => format!("Checking frames · {done} of {total}"),
            Stage::Creating { done, total } => {
                format!("Creating sequence · {done} of {total} frames")
            }
        })
    })
    .map_err(|error| match error {
        pm_export::Error::InvalidFrame(why) | pm_export::Error::InvalidSpec(why) => why,
        pm_export::Error::Io(why) => why,
        pm_export::Error::Cancelled => "Import cancelled".into(),
        other => format!("Could not create the image sequence: {other}"),
    })?;
    say("Importing image sequence…".into());
    Ok(EncodedSequence {
        _dir: dir,
        path,
        fps: sequence.fps,
        frames: encoded.frames,
    })
}
/// A 3D model's type, for imports that take models.
fn model_mime(path: &Path) -> Option<&'static str> {
    match extension(path).as_str() {
        "glb" => Some("model/gltf-binary"),
        "gltf" => Some("model/gltf+json"),
        "obj" => Some("model/obj"),
        _ => None,
    }
}
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub imported: Vec<String>,
    /// The imported (or replaced) assets' ids.
    pub assets: Vec<String>,
    /// File name and reason, in the panel's words.
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
}
impl Outcome {
    pub fn summary(&self) -> String {
        let mut parts = vec![];
        match self.imported.len() {
            0 => {}
            1 => parts.push(format!("Imported {}", self.imported[0])),
            n => parts.push(format!("Imported {n} files")),
        }
        parts.extend(
            self.failed
                .iter()
                .map(|(file, why)| format!("{file} · {why}")),
        );
        if self.cancelled {
            parts.push("Import cancelled".into());
        }
        parts.join(" · ")
    }
}
const SWITCHED: &str =
    "Import stopped because you switched projects · import again in the intended project";
const EDIT_OPEN: &str = "Finish the current edit, then import again";
/// Concurrent edits move the revision; the receipts stay valid across retries.
const ADMIT_ATTEMPTS: usize = 8;
/// An open edit or drag refuses admission. Bytes are already retained, so wait
/// it out on this worker: 10 seconds, in steps.
const EDIT_WAITS: usize = 100;
const EDIT_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

impl Builtins {
    /// Blocks the caller for probing, copying and the actor's reply. Run it on
    /// a worker only. Nothing is admitted unless `document` is still open.
    pub fn import_media(
        &self,
        document: u64,
        profile: &Path,
        request: ImportRequest,
        cancel: &AtomicBool,
    ) -> Result<Outcome, String> {
        let project = self.broker.project.clone();
        let archive = Archive::open(profile).map_err(|e| e.to_string())?;
        let mut outcome = Outcome::default();
        let mut items = vec![];
        let encoded = match &request.sequence {
            Some(sequence) => match encode_sequence(&request.paths, sequence, cancel) {
                Ok(encoded) => Some(encoded),
                Err(_) if cancel.load(Ordering::Acquire) => {
                    outcome.cancelled = true;
                    return Ok(outcome);
                }
                Err(error) => return Err(error),
            },
            None => None,
        };
        let (mut folders, mut folder_of) = (vec![], std::collections::HashMap::new());
        let paths = match &encoded {
            Some(encoded) => vec![encoded.path.clone()],
            None if request.keep_folders && request.replace.is_none() => {
                let (made, files) = folder_tree(request.paths);
                folders = made;
                files
                    .into_iter()
                    .map(|(path, folder)| {
                        if let Some(folder) = folder {
                            folder_of.insert(path.clone(), folder);
                        }
                        path
                    })
                    .collect()
            }
            None => expand(request.paths),
        };
        // A relink retains the old bytes and reads which of the old file's
        // audio streams were heard and how the asset reads its file's times.
        let mut timing = pm_media::clip::ClipTiming::Presentation;
        let (previous, old_streams) = match &request.replace {
            Some(id) => {
                let published = self.broker.kernel.published(&project).ok_or(SWITCHED)?;
                let entry = published.media.get(id).cloned();
                let previous = entry
                    .map(|entry| archive.retain_entry(&entry))
                    .transpose()
                    .map_err(|e| e.to_string())?;
                let record = published
                    .snapshot
                    .root()
                    .and_then(|root| root.get("assets")?.get(id))
                    .map(|record| crate::data::wire(&record.to_value()));
                timing = asset_timing(record.as_ref());
                let streams = record.and_then(|record| record.get("audioStreams").cloned());
                (previous, streams)
            }
            None => (None, None),
        };
        for path in paths {
            if cancel.load(Ordering::Acquire) {
                outcome.cancelled = true;
                break;
            }
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let model = request.models.then(|| model_mime(&path)).flatten();
            let Some(kind) = kind(&path).or(model.map(|_| "model")) else {
                outcome.failed.push((name, "Unsupported file".into()));
                continue;
            };
            let type_ = model.map_or_else(|| mime(&path, kind), str::to_owned);
            let retained = (if model.is_some() {
                Ok(())
            } else {
                decodes(&path, kind)
            })
            .and_then(|()| {
                let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                let (mut asset, streams) = match model {
                    // A model has no media facts; the scene system reads its bytes.
                    Some(_) => (
                        json!({"name":name,"kind":"model","size":size,"persisted":true,
                            "sourcePath":path.to_string_lossy()}),
                        vec![],
                    ),
                    None => {
                        let (asset, streams) = metadata(&path, kind, size, timing)?;
                        (crate::data::wire(&asset), streams)
                    }
                };
                // A new clip's times address the file's presentation. A
                // relinked asset keeps the reading its edits were made with:
                // the merge leaves its `timing` member (or its absence) alone.
                if request.replace.is_none() && matches!(kind, "video" | "audio") {
                    asset["timing"] = json!(pm_media::clip::ClipTiming::imported());
                }
                if let Some(definition) = &request.layer_definition {
                    asset["layerDefinition"] = json!(definition);
                }
                if let Some(folder) = folder_of.get(&path) {
                    asset["folder"] = json!(folder);
                }
                // Generated in scratch: no source file to relink to.
                if (encoded.is_some() || request.staged.is_some())
                    && let Some(asset) = asset.as_object_mut()
                {
                    asset.remove("sourcePath");
                }
                if let Some(encoded) = &encoded {
                    asset["fps"] = json!(encoded.fps);
                    asset["imageSequence"] = json!({"fps": encoded.fps, "frames": encoded.frames});
                }
                Ok((asset, streams))
            })
            .and_then(|(mut asset, streams)| {
                let original = archive
                    .retain_path(&path, &type_, cancel)
                    .map_err(|e| e.to_string())?;
                if request.replace.is_some() {
                    relink_streams(
                        &mut asset,
                        old_streams.as_ref(),
                        &streams,
                        previous.as_ref(),
                        &original,
                    );
                }
                Ok(RetainedAsset {
                    id: pm_kernel::new_id("a"),
                    metadata: crate::data::model(&asset)?,
                    original,
                    previous: previous.clone(),
                })
            });
            match retained {
                Ok(item) => items.push(item),
                Err(error) => outcome.failed.push((name, error)),
            }
        }
        if items.is_empty() {
            return Ok(outcome);
        }
        // A folder whose every file was refused is not made.
        let mut used: std::collections::HashSet<String> = items
            .iter()
            .filter_map(|item| Some(item.metadata.get("folder")?.as_str()?.to_owned()))
            .collect();
        for folder in folders.iter().rev() {
            if used.contains(&folder.id)
                && let Some(parent) = &folder.parent
            {
                used.insert(parent.clone());
            }
        }
        folders.retain(|folder| used.contains(&folder.id));
        let names: Vec<String> = items
            .iter()
            .filter_map(|item| Some(item.metadata.get("name")?.as_str()?.to_owned()))
            .collect();
        let assets: Vec<String> = match &request.replace {
            Some(id) => vec![id.clone()],
            None => items.iter().map(|item| item.id.to_string()).collect(),
        };
        let (mut attempt, mut waits) = (0, 0);
        let layers = loop {
            attempt += 1;
            let published = self.broker.kernel.published(&project).ok_or(SWITCHED)?;
            if published.snapshot.doc_id != document {
                return Err(SWITCHED.into());
            }
            let stamp = pm_panel_media::Catalog::published(&published)
                .map_err(|_| "The project's media could not be read")?
                .stamp;
            let (layers, place) = match request.place_at.filter(|_| request.replace.is_none()) {
                Some(at) => {
                    placement(&published, &items, at, request.placed_as.as_ref()).ok_or(SWITCHED)?
                }
                None => (vec![], vec![]),
            };
            let reply = self
                .admit_media(
                    &stamp,
                    items.clone(),
                    request.folder.clone(),
                    &folders,
                    request.replace.clone(),
                    place,
                    Caller::native(),
                )?
                .wait()
                .ok_or("Project closed")?;
            let stale = self
                .broker
                .kernel
                .published(&project)
                .is_some_and(|p| p.revision() != stamp.revision);
            match reply.map(|ack| ack.reply) {
                Ok(Reply::Throw(error)) if edit_open(&error) => {
                    waits += 1;
                    if cancel.load(Ordering::Acquire) {
                        outcome.cancelled = true;
                        return Ok(outcome);
                    }
                    if waits >= EDIT_WAITS {
                        return Err(EDIT_OPEN.into());
                    }
                    attempt -= 1;
                    std::thread::sleep(EDIT_WAIT);
                }
                Ok(Reply::Throw(_)) if stale && attempt < ADMIT_ATTEMPTS => {}
                Ok(Reply::Throw(error)) => return Err(error.to_string()),
                Ok(_) => break layers,
                Err(_) if stale && attempt < ADMIT_ATTEMPTS => {}
                Err(error) => return Err(error.to_string()),
            }
        };
        if !layers.is_empty() {
            // The import landed; a refused selection does not undo that.
            let selection = SelectionCall::Layers {
                ids: layers,
                add: false,
            };
            let request = Request::new(Caller::native(), selection);
            let _ = self.broker.kernel.call(&project, request).wait();
        }
        outcome.imported = names;
        outcome.assets = assets;
        Ok(outcome)
    }
}

/// What a transient edit refuses, as opposed to what retrying cannot fix.
fn edit_open(error: &KernelError) -> bool {
    matches!(
        error,
        KernelError::Guard(_) | KernelError::Transaction(TransactionError::GestureInProgress)
    )
}

/// `PM.commandForAsset` for each imported asset: new layer ids and the
/// commands adding them at `at`. `None`: no project to place in.
fn placement(
    published: &Published,
    items: &[RetainedAsset],
    at: f64,
    placed_as: Option<&PlacedAs>,
) -> Option<(Vec<String>, Vec<Command>)> {
    let root = published.snapshot.root()?;
    let open = published.snapshot.open_comp();
    let composition = pm_kernel::model::comp_node(root, open.as_deref()).unwrap_or(root);
    let number = |key: &str, default: f64| {
        composition
            .get(key)
            .and_then(|n| n.to_value().as_f64())
            .unwrap_or(default)
    };
    let (fps, dur, w, h) = (
        number("fps", 30.),
        number("dur", 1.),
        number("w", 1920.),
        number("h", 1080.),
    );
    let mut commands: Vec<Command> = vec![];
    let mut layers = vec![];
    for item in items {
        let (id, asset) = (item.id.as_str(), &item.metadata);
        let field = |key: &str| asset.get(key).and_then(Value::as_f64).filter(|n| *n > 0.);
        let kind = asset.get("kind").and_then(Value::as_str).unwrap_or("image");
        let mut content = Map::new();
        content.insert("asset", Value::from(id));
        match kind {
            "audio" => {
                for (key, value) in [("trim", 0.), ("gain", 1.), ("fadeIn", 0.), ("fadeOut", 0.)] {
                    content.insert(key, Value::from(value));
                }
            }
            "video" => {
                content.insert("trim", Value::from(0.));
                content.insert("speed", Value::from(1.));
                let audio = asset.get("hasAudio").cloned().unwrap_or(Value::Bool(false));
                content.insert("embeddedAudio", audio);
            }
            _ => {}
        }
        if kind != "audio" {
            let (mut cw, mut ch) = (field("w").unwrap_or(w), field("h").unwrap_or(h));
            if let Some((fw, fh)) = placed_as.and_then(|p| p.fit)
                && let (Some(aw), Some(ah)) = (field("w"), field("h"))
            {
                let scale = (fw / aw).min(fh / ah);
                (cw, ch) = (aw * scale, ah * scale);
            }
            content.insert("w", Value::from(cw));
            content.insert("h", Value::from(ch));
        }
        let layer = pm_kernel::commands::new_id("L");
        layers.push(layer.clone());
        let name = match placed_as {
            Some(placed) => Some(placed.name.clone()),
            None => asset.get("name").and_then(Value::as_str).map(str::to_owned),
        };
        commands.push(
            LayerCommand::Add(Box::new(pm_kernel::commands::layers::AddLayer {
                id: layer,
                layer_type: if kind == "audio" || kind == "video" {
                    kind
                } else {
                    "image"
                }
                .into(),
                comp: None,
                name,
                from: None,
                // The whole source, even past the composition end.
                duration: Some(field("dur").unwrap_or_else(|| (dur - at).max(1. / fps))),
                content,
                properties: Map::new(),
                color: None,
                index: None,
                parent: None,
                blend: None,
                motion_blur: None,
                visible: None,
                shy: None,
                collapsed: None,
                time: at,
            }))
            .into(),
        );
    }
    Some((layers, commands))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{KernelActor, KernelConfig, persistence::DiskFormat, session::*};

    /// A real 2×2 PNG, decoded by the same production probes.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72,
        0xb6, 0x0d, 0x24, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x84, 0x19, 0x60, 0x0c, 0x00, 0x47, 0xca, 0x07, 0xf9, 0x67, 0x59,
        0x6e, 0xb7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        actor: KernelActor,
        runtime: Builtins,
    }
    fn fixture() -> Fixture {
        opened(
            r##"{"id":"p","name":"Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
        )
        .0
    }
    /// A session on `project` and its handle.
    fn opened(project: &str) -> (Fixture, SessionHandle) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(root.join("journals")),
            ..Default::default()
        });
        let kernel = actor.handle();
        kernel
            .shell(ShellCall::AttachLibrary {
                profile: root.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        let project = pm_model::json::parse(project).unwrap();
        let reply = kernel
            .shell(ShellCall::Open {
                id: "p".into(),
                source: SessionSource::New(project),
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        let ShellReply::Opened { session, .. } = reply else {
            panic!("the project did not open");
        };
        let runtime = Builtins::new(kernel, "p", || {}).unwrap();
        let fixture = Fixture {
            _dir: dir,
            root,
            actor,
            runtime,
        };
        (fixture, session)
    }
    fn write(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
    fn document(f: &Fixture) -> u64 {
        f.actor.handle().published("p").unwrap().snapshot.doc_id
    }

    #[test]
    fn a_picked_image_is_retained_admitted_placed_and_survives_source_deletion() {
        let f = fixture();
        let source = write(&f.root, "Sky.png", PNG);
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![source.clone()],
                    place_at: Some(2.),
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.imported, ["Sky.png"]);
        assert!(outcome.failed.is_empty(), "{outcome:?}");
        assert_eq!(outcome.summary(), "Imported Sky.png");
        std::fs::remove_file(&source).unwrap();

        let kernel = f.actor.handle();
        let published = kernel.published("p").unwrap();
        let root = published.snapshot.root().unwrap().to_value();
        let ids = pm_kernel::asset_order::keys(&root);
        assert_eq!(ids.len(), 1);
        let asset = root.get("assets").unwrap().get(&ids[0]).unwrap();
        assert_eq!(asset.get("kind").unwrap().as_str(), Some("image"));
        assert_eq!(asset.get("w").unwrap().as_f64(), Some(2.));
        assert!(asset.get("originalVersion").is_some());
        let copied = f.root.join("copied.png");
        published
            .media
            .get(&ids[0])
            .unwrap()
            .copy_to(&copied)
            .unwrap();
        assert_eq!(
            std::fs::read(copied).unwrap(),
            PNG,
            "bytes outlive the source"
        );

        let open = published.snapshot.open_comp();
        let composition =
            pm_kernel::model::comp_node(published.snapshot.root().unwrap(), open.as_deref())
                .unwrap()
                .to_value();
        let layers = composition.get("layers").unwrap().as_array().unwrap();
        let layer = &layers[0];
        assert_eq!(layer.get("type").unwrap().as_str(), Some("image"));
        assert_eq!(layer.get("from").unwrap().as_f64(), Some(2.));
        assert_eq!(
            layer.get("d").unwrap().get("asset").unwrap().as_str(),
            Some(ids[0].as_str())
        );
        assert_eq!(
            published.selection.primary(),
            layer.get("id").and_then(Value::as_str)
        );
    }

    #[test]
    fn an_image_sequence_becomes_one_lossless_webm_clip_skipping_missing_numbers() {
        let f = fixture();
        // Frames 1, 2, 3, 5 (4 missing), chosen out of order with mixed padding.
        let frames: Vec<_> = ["shot_005.png", "shot_1.png", "shot_003.png", "shot_02.png"]
            .iter()
            .map(|name| write(&f.root, name, PNG))
            .collect();
        let progress = Arc::new(Mutex::new(String::new()));
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: frames.clone(),
                    place_at: Some(1.),
                    sequence: Some(SequenceImport {
                        fps: 12.,
                        progress: progress.clone(),
                    }),
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.imported, ["shot sequence.webm"], "{outcome:?}");
        assert_eq!(*progress.lock().unwrap(), "Importing image sequence…");
        for frame in &frames {
            std::fs::remove_file(frame).unwrap();
        }
        let published = f.actor.handle().published("p").unwrap();
        let root = published.snapshot.root().unwrap().to_value();
        let ids = pm_kernel::asset_order::keys(&root);
        assert_eq!(ids.len(), 1);
        let asset = root.get("assets").unwrap().get(&ids[0]).unwrap();
        let field = |key: &str| asset.get(key).and_then(Value::as_f64);
        assert_eq!(asset.get("kind").unwrap().as_str(), Some("video"));
        assert_eq!(field("w"), Some(2.));
        assert_eq!(field("fps"), Some(12.));
        let sequence = asset.get("imageSequence").unwrap();
        assert_eq!(sequence.get("fps").and_then(Value::as_f64), Some(12.));
        // Four images, not five numbers: missing numbers are skipped.
        assert_eq!(sequence.get("frames").and_then(Value::as_f64), Some(4.));
        assert!((field("dur").unwrap() - 4. / 12.).abs() < 0.01);
        assert!(asset.get("sourcePath").is_none());
        let copied = f.root.join("copied.webm");
        published
            .media
            .get(&ids[0])
            .unwrap()
            .copy_to(&copied)
            .unwrap();
        assert_eq!(&std::fs::read(&copied).unwrap()[..4], b"\x1a\x45\xdf\xa3");
        let open = published.snapshot.open_comp();
        let composition =
            pm_kernel::model::comp_node(published.snapshot.root().unwrap(), open.as_deref())
                .unwrap()
                .to_value();
        let layer = &composition.get("layers").unwrap().as_array().unwrap()[0];
        assert_eq!(layer.get("type").unwrap().as_str(), Some("video"));
        assert_eq!(layer.get("from").unwrap().as_f64(), Some(1.));
    }

    #[test]
    fn sequence_choices_read_in_todays_words() {
        let p = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
        assert_eq!(
            order_sequence(&p(&["/a/f_10.png", "/a/f_9.png"])).unwrap(),
            p(&["/a/f_9.png", "/a/f_10.png"])
        );
        assert_eq!(
            order_sequence(&p(&["/a/f_1.png"])).unwrap_err(),
            "Choose at least 2 numbered image frames"
        );
        assert_eq!(
            order_sequence(&p(&["/a/f_1.png", "/a/f_01.png"])).unwrap_err(),
            "Duplicate frame number 1"
        );
        assert!(
            order_sequence(&p(&["/a/f_1.png", "/a/g_2.png"]))
                .unwrap_err()
                .starts_with("Choose images with the same name and extension")
        );
        assert_eq!(
            sequence_name(Path::new("/a/render_v2-0001.png")),
            "render_v2 sequence.webm"
        );
        assert_eq!(
            sequence_name(Path::new("/a/0001.png")),
            "Image sequence.webm"
        );
    }

    /// One second of silence, 8 kHz mono.
    fn wav() -> Vec<u8> {
        let data = vec![0u8; 16_000];
        let mut out = b"RIFF".to_vec();
        out.extend((36 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend([1, 0, 1, 0]);
        out.extend(8_000u32.to_le_bytes());
        out.extend(16_000u32.to_le_bytes());
        out.extend([2, 0, 16, 0]);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    /// A new clip's times address its file's presentation (`timing`); a
    /// relinked asset keeps the reading its edits were made with, present or
    /// absent; and the member survives the project file.
    #[test]
    fn a_new_clip_is_stamped_with_its_timing_and_a_relink_never_changes_it() {
        let (f, session) = opened(
            r##"{"id":"p","name":"Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{"old":{"id":"old","name":"Old.wav","kind":"audio","dur":1}},"comps":{},"layers":[]}"##,
        );
        let import = |name: &str, replace: Option<&str>| {
            let outcome = f
                .runtime
                .import_media(
                    document(&f),
                    &f.root,
                    ImportRequest {
                        paths: vec![write(&f.root, name, &wav())],
                        replace: replace.map(str::to_owned),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert!(outcome.failed.is_empty(), "{outcome:?}");
        };
        let timings = |project: &str| -> Vec<(String, Option<f64>)> {
            let published = f.actor.handle().published(project).unwrap();
            let root = published.snapshot.root().unwrap().to_value();
            pm_kernel::asset_order::keys(&root)
                .into_iter()
                .map(|id| {
                    let asset = root.get("assets").unwrap().get(&id).unwrap();
                    let name = asset.get("name").unwrap().as_str().unwrap().to_owned();
                    (name, asset.get("timing").and_then(Value::as_f64))
                })
                .collect()
        };
        import("New.wav", None);
        let expected = vec![
            ("Old.wav".to_owned(), None),
            ("New.wav".to_owned(), Some(2.)),
        ];
        assert_eq!(timings("p"), expected);
        // Relink both: the older asset stays without the member, the new
        // one keeps it.
        let root = f.actor.handle().published("p").unwrap();
        let new = pm_kernel::asset_order::keys(&root.snapshot.root().unwrap().to_value())
            .into_iter()
            .find(|id| id != "old")
            .unwrap();
        import("Old again.wav", Some("old"));
        import("New again.wav", Some(&new));
        let relinked = vec![
            ("Old again.wav".to_owned(), None),
            ("New again.wav".to_owned(), Some(2.)),
        ];
        assert_eq!(timings("p"), relinked);
        // Saved and opened again, both records read the same.
        let file = f.root.join("Film.pmv");
        let kernel = f.actor.handle();
        let save = ShellCall::SaveAs {
            session: session.clone(),
            path: file.clone(),
            replace: false,
        };
        kernel.shell(save).wait().unwrap().unwrap();
        kernel
            .shell(ShellCall::Close(session))
            .wait()
            .unwrap()
            .unwrap();
        kernel
            .shell(ShellCall::Open {
                id: "p".into(),
                source: SessionSource::File(file),
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(timings("p"), relinked);
    }

    const RATE: usize = 48_000;
    /// Where each fixture stream's one click sits: two streams of a
    /// recorder file, a mono and a stereo replacement.
    const CLICKS: [usize; 4] = [4_800, 24_000, 14_400, 33_600];

    fn ffmpeg() -> PathBuf {
        std::env::var_os("PM_FFMPEG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var("HOME").unwrap())
                    .join("Library/Caches/powermove-engine/ffmpeg/out")
            })
            .join("bin/ffmpeg")
    }

    /// A one-second clip with a picture and one PCM stream per click
    /// (`(click, channels)`), each stream holding its one click.
    fn clip_with_streams(root: &Path, name: &str, streams: &[(usize, u32)]) -> PathBuf {
        let path = root.join(name);
        let mut command = std::process::Command::new(ffmpeg());
        command.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"]);
        command.args(["-i", "testsrc2=size=64x64:rate=25:duration=1"]);
        for &(at, channels) in streams {
            let layout = if channels == 1 { "mono" } else { "stereo" };
            command.args(["-f", "lavfi", "-i"]).arg(format!(
                "aevalsrc='0.9*eq(n\\,{at})':s={RATE}:d=1:c={layout}"
            ));
        }
        command.args(["-map", "0:v"]);
        for k in 0..streams.len() {
            command.args(["-map", &format!("{}:a", k + 1)]);
        }
        command.args(["-c:v", "mpeg4", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"]);
        assert!(command.arg(&path).status().unwrap().success());
        path
    }

    /// The clicks a person hears from `asset` as its only clip: the
    /// published record through the mix graph and offline PCM.
    fn heard(f: &Fixture, asset: &str) -> Vec<usize> {
        let published = f.actor.handle().published("p").unwrap();
        let mut root = crate::data::wire(&published.snapshot.root().unwrap().to_value());
        root["layers"] = json!([{"id":"V","type":"video","name":"V","from":0,"dur":1,"on":true,
            "solo":false,"group":null,"parent":null,"p":{},"fx":[],"masks":[],
            "d":{"asset":asset,"trim":0,"speed":1,"embeddedAudio":true}}]);
        let source = PathBuf::from(root["assets"][asset]["sourcePath"].as_str().unwrap());
        let doc = pm_model::Document::new(Some(crate::data::model(&root).unwrap()), 1);
        let cache = pm_audio::SourceCache::new(64 << 20, 0);
        let resolver = move |_: &str| Some(source.clone());
        let comp = root["compId"].as_str().unwrap().to_owned();
        let graph = pm_audio::MixGraph::build(
            &doc,
            &comp,
            &mut pm_eval::Evaluator::new(),
            &cache,
            &resolver,
        )
        .unwrap();
        let mut mixer = pm_audio::Mixer::new(
            pm_audio::MixFormat::stereo(RATE as u32),
            std::sync::Arc::new(doc),
            std::sync::Arc::new(graph),
            pm_eval::Evaluator::new(),
            cache.clone(),
        );
        let mut left = vec![];
        let summary = pm_audio::offline::render(&mut mixer, 0, RATE as i64, 4096, |_, inter| {
            left.extend(inter.iter().step_by(2).copied());
            Ok(())
        })
        .unwrap();
        cache.shutdown();
        assert!(summary.errors.is_empty(), "{:?}", summary.errors);
        let mut found = vec![];
        for (k, &at) in CLICKS.iter().enumerate() {
            if left[at.saturating_sub(2)..(at + 3).min(left.len())]
                .iter()
                .any(|v| v.abs() > 0.2)
            {
                found.push(k);
            }
        }
        let loud = left.iter().filter(|v| v.abs() > 0.2).count();
        assert!(loud <= found.len() * 2, "sound away from any click: {loud}");
        found
    }

    fn streams_of(f: &Fixture, asset: &str) -> Option<serde_json::Value> {
        let published = f.actor.handle().published("p").unwrap();
        let root = crate::data::wire(&published.snapshot.root().unwrap().to_value());
        root["assets"][asset].get("audioStreams").cloned()
    }

    /// Relinking an asset to its own file changes nothing a person hears:
    /// an older clip without a stream list keeps playing its one stream, and
    /// a stream the person switched off stays off. Replacing it with other
    /// footage reconciles the list to the new file: a stream with the same
    /// index and layout keeps its state, anything else plays as a fresh
    /// import would, and nothing listed names a stream the file lacks.
    #[test]
    fn a_relink_keeps_which_audio_streams_are_heard() {
        let entry = |index: u32, on: bool| {
            format!(r#"{{"index":{index},"channels":1,"sampleRate":48000,"on":{on}}}"#)
        };
        let project = format!(
            r##"{{"id":"p","name":"Film","compId":"c","w":640,"h":360,"fps":25,"dur":1,"bg":"#000000","comps":{{}},"layers":[],"assets":{{
                "older":{{"id":"older","name":"Older.mov","kind":"video","dur":1}},
                "muted":{{"id":"muted","name":"Muted.mov","kind":"video","dur":1,"audioStreams":[{},{}]}},
                "first_off":{{"id":"first_off","name":"First off.mov","kind":"video","dur":1,"audioStreams":[{},{}]}},
                "both":{{"id":"both","name":"Both.mov","kind":"video","dur":1,"audioStreams":[{},{}]}}}}}}"##,
            entry(1, true),
            entry(2, false),
            entry(1, false),
            entry(2, true),
            entry(1, true),
            entry(2, true),
        );
        let (f, _session) = opened(&project);
        let recorder =
            clip_with_streams(&f.root, "recorder.mov", &[(CLICKS[0], 1), (CLICKS[1], 1)]);
        let mono = clip_with_streams(&f.root, "mono.mov", &[(CLICKS[2], 1)]);
        let stereo = clip_with_streams(&f.root, "stereo.mov", &[(CLICKS[3], 2)]);
        let relink = |asset: &str, path: &Path| {
            let outcome = f
                .runtime
                .import_media(
                    document(&f),
                    &f.root,
                    ImportRequest {
                        paths: vec![path.to_owned()],
                        replace: Some(asset.into()),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert!(outcome.failed.is_empty(), "{outcome:?}");
        };
        // An older clip without a list: still the file's one best stream.
        relink("older", &recorder);
        assert_eq!(heard(&f, "older"), [0]);
        assert_eq!(streams_of(&f, "older"), None);
        // The stream the person switched off stays off.
        relink("muted", &recorder);
        assert_eq!(heard(&f, "muted"), [0]);
        // A one-stream replacement: stream 1 keeps its state (off); stream
        // 2 no longer exists and is no longer listed.
        relink("first_off", &mono);
        assert_eq!(heard(&f, "first_off"), Vec::<usize>::new());
        assert_eq!(
            streams_of(&f, "first_off"),
            Some(json!([{"index":1,"channels":1,"sampleRate":48000,"on":false}]))
        );
        // A stereo stream is not the mono one it replaces: it plays as the
        // file's one stream, without a list.
        relink("both", &stereo);
        assert_eq!(heard(&f, "both"), [3]);
        assert_eq!(streams_of(&f, "both"), None);
        // New footage known to differ (the old bytes were retained): a clip
        // that played one stream lists and plays all of the new file's.
        relink("both", &recorder);
        assert_eq!(heard(&f, "both"), [0, 1]);
        // The same bytes again: nothing changes.
        let copy = f.root.join("recorder copy.mov");
        std::fs::copy(&recorder, &copy).unwrap();
        relink("muted", &copy);
        assert_eq!(heard(&f, "muted"), [0]);
    }

    /// A relink records the clip's lengths as the asset reads its file: an
    /// older asset's frames sit at their own timestamps and its sound starts
    /// at the program's first sample; a new one reads the presentation.
    #[test]
    fn a_relink_measures_the_clip_as_its_asset_reads_it() {
        let (f, _session) = opened(
            r##"{"id":"p","name":"Film","compId":"c","w":640,"h":360,"fps":25,"dur":1,"bg":"#000000","comps":{},"layers":[],"assets":{
                "older":{"id":"older","name":"Older.mp4","kind":"video","dur":1},
                "new":{"id":"new","name":"New.mp4","kind":"video","dur":1,"timing":2}}}"##,
        );
        let run = |args: &[&str], name: &str| {
            let path = f.root.join(name);
            let status = std::process::Command::new(ffmpeg())
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(args)
                .args(["-c:v", "mpeg4", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"])
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
            path
        };
        let picture = [
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x64:rate=25:duration=0.8",
        ];
        // Twenty frames, the first 0.4 s into the file.
        let late_picture = run(
            &[&picture[..], &["-output_ts_offset", "0.4"]].concat(),
            "late picture.mp4",
        );
        // A second of picture from zero and a second of sound from 0.25 s.
        let late_sound = run(
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=25:duration=1",
                "-itsoffset",
                "0.25",
                "-f",
                "lavfi",
                "-i",
                "sine=r=48000:d=1",
                "-map",
                "0:v",
                "-map",
                "1:a",
            ],
            "late sound.mov",
        );
        let lengths = |asset: &str| {
            let published = f.actor.handle().published("p").unwrap();
            let root = crate::data::wire(&published.snapshot.root().unwrap().to_value());
            let record = &root["assets"][asset];
            (
                record["dur"].as_f64().unwrap(),
                record["audioDur"].as_f64().unwrap(),
            )
        };
        let relink = |asset: &str, path: &Path| {
            let outcome = f
                .runtime
                .import_media(
                    document(&f),
                    &f.root,
                    ImportRequest {
                        paths: vec![path.to_owned()],
                        replace: Some(asset.into()),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert!(outcome.failed.is_empty(), "{outcome:?}");
        };
        let close =
            |(a, b): (f64, f64), (c, d): (f64, f64)| (a - c).abs() < 1e-3 && (b - d).abs() < 1e-3;
        for (path, older, new) in [
            (&late_picture, (1.2, 0.), (0.8, 0.)),
            (&late_sound, (1.0, 1.0), (1.25, 1.25)),
        ] {
            relink("older", path);
            relink("new", path);
            assert!(
                close(lengths("older"), older),
                "{path:?}: {:?}",
                lengths("older")
            );
            assert!(close(lengths("new"), new), "{path:?}: {:?}", lengths("new"));
        }
    }

    #[test]
    fn one_undo_removes_the_layers_and_the_media_of_a_placed_import() {
        let f = fixture();
        let sources = vec![write(&f.root, "a.png", PNG), write(&f.root, "b.png", PNG)];
        f.runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: sources,
                    place_at: Some(0.),
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        let kernel = f.actor.handle();
        let state = || {
            let published = kernel.published("p").unwrap();
            let root = published.snapshot.root().unwrap().to_value();
            let open = published.snapshot.open_comp();
            let composition =
                pm_kernel::model::comp_node(published.snapshot.root().unwrap(), open.as_deref())
                    .unwrap()
                    .to_value();
            let layers = composition.get("layers").unwrap().as_array().unwrap().len();
            (pm_kernel::asset_order::keys(&root).len(), layers)
        };
        assert_eq!(state(), (2, 2));
        let published = kernel.published("p").unwrap();
        assert_eq!(
            published.history.undo_label.as_deref(),
            Some("Import 2 files")
        );
        let undo = Request::new(Caller::native(), pm_kernel::HistoryCall::Undo);
        kernel.call("p", undo).wait().unwrap().unwrap();
        assert_eq!(state(), (0, 0));
    }

    #[test]
    fn an_import_waits_for_an_open_edit_and_lands_when_it_closes() {
        let f = fixture();
        let source = write(&f.root, "a.png", PNG);
        let kernel = f.actor.handle();
        let begin = pm_kernel::HistoryCall::Begin {
            label: "Drag".into(),
        };
        let open = Request::new(Caller::native(), begin);
        kernel.call("p", open).wait().unwrap().unwrap();
        let commit = std::thread::spawn({
            let kernel = kernel.clone();
            move || {
                std::thread::sleep(EDIT_WAIT * 3);
                let call = pm_kernel::HistoryCall::Commit { label: None };
                let request = Request::new(Caller::native(), call);
                kernel.call("p", request).wait().unwrap().unwrap();
            }
        });
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![source],
                    place_at: Some(0.),
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        commit.join().unwrap();
        assert_eq!(outcome.imported, ["a.png"]);
        assert!(outcome.failed.is_empty());
    }

    #[test]
    fn a_cancel_stops_the_wait_for_an_open_edit() {
        let f = fixture();
        let source = write(&f.root, "a.png", PNG);
        let begin = pm_kernel::HistoryCall::Begin {
            label: "Drag".into(),
        };
        let open = Request::new(Caller::native(), begin);
        let kernel = f.actor.handle();
        kernel.call("p", open).wait().unwrap().unwrap();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let later = std::thread::spawn({
            let cancel = cancel.clone();
            move || {
                std::thread::sleep(EDIT_WAIT * 2);
                cancel.store(true, Ordering::Release);
            }
        });
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![source],
                    ..Default::default()
                },
                &cancel,
            )
            .unwrap();
        later.join().unwrap();
        assert!(outcome.cancelled && outcome.imported.is_empty());
    }

    #[test]
    fn a_kernel_or_probe_error_reads_as_words_not_as_debug_output() {
        assert_eq!(
            KernelError::Guard("Close the edit first".into()).to_string(),
            "Close the edit first"
        );
        let f = fixture();
        let picked = write(&f.root, "broken.png", b"not a png");
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![picked],
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        let why = &outcome.failed[0].1;
        for debug in ["Unreadable(", "Unsupported(", "Throw("] {
            assert!(!why.contains(debug), "{why}");
        }
    }

    #[test]
    fn several_files_keep_picked_order_and_bad_files_are_named_without_stopping_the_rest() {
        let f = fixture();
        let picked = vec![
            write(&f.root, "zebra.png", PNG),
            write(&f.root, "notes.txt", b"text"),
            write(&f.root, "broken.png", b"not a png"),
            write(&f.root, "apple.png", PNG),
        ];
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: picked,
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.imported, ["zebra.png", "apple.png"]);
        assert_eq!(outcome.failed.len(), 2);
        assert_eq!(
            outcome.failed[0],
            ("notes.txt".into(), "Unsupported file".into())
        );
        assert_eq!(outcome.failed[1].0, "broken.png");
        let published = f.actor.handle().published("p").unwrap();
        let root = published.snapshot.root().unwrap().to_value();
        let names: Vec<_> = pm_kernel::asset_order::keys(&root)
            .iter()
            .map(|id| {
                let asset = root.get("assets").unwrap().get(id).unwrap();
                asset.get("name").unwrap().as_str().unwrap().to_owned()
            })
            .collect();
        assert_eq!(names, ["zebra.png", "apple.png"], "Project panel order");
    }

    #[test]
    fn an_import_finishing_after_the_project_changed_admits_nothing() {
        let f = fixture();
        let source = write(&f.root, "Sky.png", PNG);
        let error = f
            .runtime
            .import_media(
                document(&f) + 1,
                &f.root,
                ImportRequest {
                    paths: vec![source],
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap_err();
        assert_eq!(error, SWITCHED);
        let published = f.actor.handle().published("p").unwrap();
        assert!(published.media.is_empty());
    }

    #[test]
    fn a_cancelled_import_stops_before_the_next_file() {
        let f = fixture();
        let source = write(&f.root, "Sky.png", PNG);
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![source],
                    ..Default::default()
                },
                &AtomicBool::new(true),
            )
            .unwrap();
        assert!(outcome.cancelled);
        assert!(outcome.imported.is_empty());
    }

    #[test]
    fn folders_expand_recursively_in_name_order_and_keep_media_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("shots/inner")).unwrap();
        for name in [
            "shots/b.mov",
            "shots/a.png",
            "shots/notes.txt",
            "shots/.hidden.png",
            "shots/inner/c.wav",
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        let found = expand(vec![root.join("shots")]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.png", "b.mov", "c.wav"]);
    }

    #[test]
    fn import_folder_keeps_folders_that_hold_media_and_says_where_each_file_goes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for folder in ["shots/inner", "shots/notes", "shots/deep/plates"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        for name in [
            "loose.png",
            "shots/b.mov",
            "shots/a.png",
            "shots/.hidden.png",
            "shots/inner/c.wav",
            "shots/notes/read.txt",
            "shots/deep/plates/d.png",
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        let (folders, files) = folder_tree(vec![root.join("shots"), root.join("loose.png")]);
        let name = |id: &Option<String>| {
            id.as_ref()
                .map(|id| folders.iter().find(|f| f.id == *id).unwrap().name.clone())
        };
        // Parents come first; the folder of text notes holds no media.
        let made: Vec<_> = folders
            .iter()
            .map(|f| (f.name.clone(), name(&f.parent)))
            .collect();
        assert_eq!(
            made,
            [
                ("shots".to_owned(), None),
                ("deep".to_owned(), Some("shots".to_owned())),
                ("plates".to_owned(), Some("deep".to_owned())),
                ("inner".to_owned(), Some("shots".to_owned())),
            ]
        );
        let placed: Vec<_> = files
            .iter()
            .map(|(p, folder)| {
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    name(folder),
                )
            })
            .collect();
        assert_eq!(
            placed,
            [
                ("a.png".to_owned(), Some("shots".to_owned())),
                ("b.mov".to_owned(), Some("shots".to_owned())),
                ("d.png".to_owned(), Some("plates".to_owned())),
                ("c.wav".to_owned(), Some("inner".to_owned())),
                ("loose.png".to_owned(), None),
            ]
        );
    }
}
