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
/// The stills the still path draws (`pm_media::image::read`), EXR, DPX and
/// Photoshop documents at their depth.
const IMAGES: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "svg", "tif", "tiff", "heic",
    "heif", "exr", "dpx", "tga", "psd",
];
const AUDIO: &[&str] = &[
    "wav", "mp3", "m4a", "aac", "ogg", "oga", "opus", "weba", "flac", "aif", "aiff", "aifc", "caf",
];
/// Camera raw: named when refused (no raw development in the engine).
const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef",
    "srw", "x3f", "3fr", "iiq", "erf", "mos", "braw", "r3d", "ari",
];
const VIDEO: &[&str] = &[
    "mp4", "mov", "m4v", "webm", "mkv", "avi", "wmv", "asf", "flv", "mpg", "mpeg", "m2v", "mts",
    "m2ts", "ts", "3gp", "3g2", "mxf", "ogv", "dv", "vob",
];
/// What Import Media says of a file it cannot read (a drop of one says
/// why in the status line: "notes.txt · Not a video, …").
const UNSUPPORTED: &str = "Not a video, audio or image file Powermove can read";
/// What Import Media says of a camera raw file.
const RAW: &str =
    "Camera raw files can't be imported yet · export TIFF or EXR from your raw developer";
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
        ("image", "exr") => "image/x-exr".into(),
        ("image", "dpx") => "image/x-dpx".into(),
        ("image", "tga") => "image/x-tga".into(),
        ("image", "psd") => "image/vnd.adobe.photoshop".into(),
        ("audio", "aif" | "aifc") => "audio/aiff".into(),
        ("audio", "caf") => "audio/x-caf".into(),
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
    /// Where the placed layers join the stack (0: the top, the default),
    /// each next file below the one before (a drop between timeline rows).
    pub place_index: Option<usize>,
    /// An extension's `assets.import`: 3D models (glb, glTF, OBJ) are media too.
    pub models: bool,
    /// The layer definition an extension asked its asset to place as.
    pub layer_definition: Option<String>,
    /// Import Image Sequence: `paths` are the frames of one clip.
    pub sequence: Option<SequenceImport>,
    /// The placed layer's name and fitted size, instead of the file's.
    pub placed_as: Option<PlacedAs>,
    /// Scratch holding generated `paths`, removed once the import is done.
    pub staged: Option<tempfile::TempDir>,
    /// What the status bar says once it lands, instead of the summary.
    pub note: Option<String>,
    /// Layer ▸ Add 3D Layer ▸ Import 3D Model…: each imported model becomes
    /// a 3D layer once its asset lands.
    pub add_3d: bool,
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
/// reported (a missing number shows as Media Offline, never another frame).
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
/// The type a sequence's retained manifest is kept as.
pub const SEQUENCE_MIME: &str = "application/x-powermove-image-sequence";
/// Chosen frames as a sequence read in place: a manifest in scratch
/// (removed when dropped) that the asset retains as its original, and the
/// asset record. Frames are never transcoded: each is read as a still at
/// its own depth when it is drawn.
pub struct ChosenSequence {
    _dir: tempfile::TempDir,
    /// The manifest to retain as the asset's original ([`SEQUENCE_MIME`]).
    pub path: PathBuf,
    /// The asset record.
    pub asset: serde_json::Value,
    /// Numbers in the range with no file, as the status line says them.
    pub missing: Option<String>,
}

/// An image sequence chosen as File ▸ Import Image Sequence chooses it, for
/// a host that imports it another way (the agent's `import_media`): `path`
/// is the sequence's folder or one of its files (its numbering is read
/// there), `fps` the sheet's Frame Rate (1 to 240). The same checks and
/// refusals as the sheet. `None`: cancelled.
pub fn choose_image_sequence(
    path: &Path,
    fps: f64,
    cancel: &AtomicBool,
) -> Result<Option<ChosenSequence>, String> {
    if !fps.is_finite() || !(1.0..=240.0).contains(&fps) {
        return Err("Choose a frame rate from 1 to 240".into());
    }
    let frame = if path.is_dir() {
        // The folder's first numbered still names the sequence.
        let mut stills: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|entry| entry.path())
            .filter(|p| kind(p) == Some("image"))
            .collect();
        stills.sort();
        stills
            .into_iter()
            .find(|p| matches!(pm_media::image::sequence::detect(p), Ok(Some(_))))
            .ok_or("This folder has no numbered image frames")?
    } else {
        path.to_path_buf()
    };
    let frames: Vec<PathBuf> = pm_media::image::sequence::detect(&frame)
        .map_err(|e| e.to_string())?
        .ok_or("Choose at least 2 numbered image frames")?
        .frames
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    let sequence = SequenceImport {
        fps,
        progress: Default::default(),
    };
    choose_sequence(&frames, &sequence, cancel)
}
fn choose_sequence(
    paths: &[PathBuf],
    sequence: &SequenceImport,
    cancel: &AtomicBool,
) -> Result<Option<ChosenSequence>, String> {
    use pm_media::image::sequence::{ImageSequence, MAX_FRAMES, Manifest};
    order_sequence(paths)?;
    let chosen = ImageSequence::from_paths(paths).map_err(|e| e.to_string())?;
    if chosen.last().saturating_sub(chosen.first()) >= MAX_FRAMES as i64 {
        return Err(format!(
            "These frames are numbered {} to {}, too far apart to be one sequence. Choose frames numbered closer together.",
            chosen.first(),
            chosen.last()
        ));
    }
    // 23.976 is 24000/1001: the clip's length and frames stay on the grid.
    let rate = pm_time::SUPPORTED_RATES
        .into_iter()
        .find(|r| (r.to_f64() - sequence.fps).abs() < 1e-3)
        .or_else(|| pm_time::Rate::new((sequence.fps * 1000.0).round() as i64, 1000).ok())
        .ok_or("Choose a frame rate above 0")?;
    let fps = rate.to_f64();
    let manifest = Manifest::of(&chosen, rate).map_err(|_| {
        "Choose frames numbered the same way (for example frame_0001.png)".to_owned()
    })?;
    if let Ok(mut progress) = sequence.progress.lock() {
        *progress = "Importing image sequence…".into();
    }
    // The first frame proves the format reads; it gives the clip its size.
    let first = &chosen.frames[0].1;
    decodes(first, "image")?;
    // Every frame is drawn at that size: one of another size is refused now
    // (read from each file's header, not decoded).
    let size_of = |path: &Path| pm_media::image::imageio::pixel_size(path).ok();
    if let Some(expected) = size_of(first) {
        for (_, path) in &chosen.frames[1..] {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            if let Some((w, h)) = size_of(path).filter(|size| *size != expected) {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                return Err(format!(
                    "{name} is {w}×{h}, but the sequence’s first frame is {}×{}. Render every frame at one size.",
                    expected.0, expected.1
                ));
            }
        }
    }
    let facts =
        pm_media::facts::media_facts_as(first, None, pm_media::clip::ClipTiming::Presentation)
            .map_err(|e| e.to_string())?;
    let [w, h] = facts.display_size.unwrap_or([0, 0]);
    let size: u64 = chosen
        .frames
        .iter()
        .filter_map(|(_, p)| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    let frames = manifest.frames();
    let asset = json!({
        "name": manifest.name(),
        "kind": "video",
        "size": size,
        "dur": frames as f64 / fps,
        "w": w,
        "h": h,
        "channels": 0,
        "sampleRate": 0,
        "playbackProxy": false,
        "playbackProxyVersion": 0,
        "persisted": true,
        "hasAudio": false,
        "audioDur": 0,
        "format": extension(first),
        "fps": fps,
        "imageSequence": {"fps": fps, "frames": frames, "first": manifest.first, "last": manifest.last},
    });
    let Some(missing) = manifest.missing_until(cancel) else {
        return Ok(None);
    };
    let missing = (!missing.is_empty()).then(|| {
        let ranges: Vec<String> = missing
            .iter()
            .map(|(a, b)| {
                if a == b {
                    a.to_string()
                } else {
                    format!("{a}–{b}")
                }
            })
            .collect();
        let single = missing.len() == 1 && missing[0].0 == missing[0].1;
        format!(
            "{} {} missing",
            if single { "frame" } else { "frames" },
            ranges.join(", ")
        )
    });
    let dir = tempfile::Builder::new()
        .prefix("powermove-image-sequence-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let path = dir.path().join("sequence.json");
    std::fs::write(&path, manifest.to_bytes()).map_err(|e| e.to_string())?;
    Ok(Some(ChosenSequence {
        _dir: dir,
        path,
        asset,
        missing,
    }))
}
/// A sequence asset relinked by choosing one of its files in their new
/// folder: every file numbered as that one is the sequence again, at the
/// rate it was imported at.
struct RelinkedSequence {
    frames: Vec<PathBuf>,
    sequence: SequenceImport,
    /// The first and last number the asset had, when it says.
    was: Option<(i64, i64)>,
}
impl RelinkedSequence {
    /// The status line's word on a range that changed.
    fn range_note(&self, asset: &serde_json::Value) -> Option<String> {
        let number = |key: &str| asset["imageSequence"][key].as_i64();
        let now = (number("first")?, number("last")?);
        let was = self.was.filter(|was| *was != now)?;
        Some(format!(
            "frames {}–{}, were {}–{}",
            now.0, now.1, was.0, was.1
        ))
    }
}
/// `paths` (one numbered still) as the new place of the sequence `record`
/// is, or None when the asset is not a sequence or the file is not one of a
/// numbered set (it then replaces the asset as any file does).
fn relinked_sequence(
    record: Option<&serde_json::Value>,
    paths: &[PathBuf],
) -> Option<RelinkedSequence> {
    let numbers = record?.get("imageSequence")?;
    let [frame] = paths else {
        return None;
    };
    if kind(frame) != Some("image") {
        return None;
    }
    let found = pm_media::image::sequence::detect(frame).ok()??;
    let fps = numbers["fps"]
        .as_f64()
        .or_else(|| record?["fps"].as_f64())
        .filter(|fps| fps.is_finite() && *fps > 0.0)
        .unwrap_or(30.0);
    Some(RelinkedSequence {
        frames: found.frames.into_iter().map(|(_, path)| path).collect(),
        sequence: SequenceImport {
            fps,
            progress: Default::default(),
        },
        was: numbers["first"].as_i64().zip(numbers["last"].as_i64()),
    })
}
/// A 3D model's type, for imports that take models.
fn model_mime(path: &Path) -> Option<&'static str> {
    match extension(path).as_str() {
        "glb" => Some("model/gltf-binary"),
        "gltf" => Some("model/gltf+json"),
        "obj" => Some("model/obj"),
        "fbx" => Some("model/fbx"),
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
    /// What else a person should know (frames missing from a sequence).
    pub notes: Vec<String>,
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
        parts.extend(self.notes.iter().cloned());
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
        // A relink retains the old bytes and reads which of the old file's
        // audio streams were heard and how the asset reads its file's times.
        let mut timing = pm_media::clip::ClipTiming::Presentation;
        let mut relinked = None;
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
                if request.sequence.is_none() {
                    relinked = relinked_sequence(record.as_ref(), &request.paths);
                }
                let streams = record.and_then(|record| record.get("audioStreams").cloned());
                (previous, streams)
            }
            None => (None, None),
        };
        let chosen = match (&request.sequence, &relinked) {
            (Some(sequence), _) => Some(choose_sequence(&request.paths, sequence, cancel)?),
            (None, Some(relinked)) => Some(choose_sequence(
                &relinked.frames,
                &relinked.sequence,
                cancel,
            )?),
            (None, None) => None,
        };
        let sequence = match chosen {
            Some(None) => {
                outcome.cancelled = true;
                return Ok(outcome);
            }
            Some(Some(sequence)) => Some(sequence),
            None => None,
        };
        let (mut folders, mut folder_of) = (vec![], std::collections::HashMap::new());
        let paths = match &sequence {
            Some(sequence) => vec![sequence.path.clone()],
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
        for path in paths {
            if cancel.load(Ordering::Acquire) {
                outcome.cancelled = true;
                break;
            }
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let model = request.models.then(|| model_mime(&path)).flatten();
            let kind = match &sequence {
                Some(_) => Some("video"),
                None => kind(&path).or(model.map(|_| "model")),
            };
            let Some(kind) = kind else {
                let raw = RAW_EXTENSIONS.contains(&extension(&path).as_str());
                outcome
                    .failed
                    .push((name, if raw { RAW } else { UNSUPPORTED }.into()));
                continue;
            };
            let type_ = match (&sequence, model) {
                (Some(_), _) => SEQUENCE_MIME.to_owned(),
                (None, Some(model)) => model.to_owned(),
                (None, None) => mime(&path, kind),
            };
            // A sequence's first frame was read when it was chosen.
            let retained = (if model.is_some() || sequence.is_some() {
                Ok(())
            } else {
                decodes(&path, kind)
            })
            .and_then(|()| {
                let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                let (mut asset, streams) = if let Some(sequence) = &sequence {
                    (sequence.asset.clone(), vec![])
                } else {
                    match model {
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
                    }
                };
                // A new clip's times address the file's presentation. A
                // relinked asset keeps the reading its edits were made with:
                // the merge leaves its `timing` member (or its absence) alone.
                if request.replace.is_none()
                    && sequence.is_none()
                    && matches!(kind, "video" | "audio")
                {
                    asset["timing"] = json!(pm_media::clip::ClipTiming::imported());
                }
                if let Some(definition) = &request.layer_definition {
                    asset["layerDefinition"] = json!(definition);
                }
                if let Some(folder) = folder_of.get(&path) {
                    asset["folder"] = json!(folder);
                }
                // Generated in scratch: no source file to relink to.
                if request.staged.is_some()
                    && let Some(asset) = asset.as_object_mut()
                {
                    asset.remove("sourcePath");
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
        let into = pm_panel_media::Destination {
            folder: request.folder.clone(),
            new_folders: folders,
        };
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
                    let items: Vec<(&str, &Value)> =
                        items.iter().map(|i| (i.id.as_str(), &i.metadata)).collect();
                    placement(
                        &published,
                        &items,
                        at,
                        request.place_index,
                        request.placed_as.as_ref(),
                    )
                    .ok_or(SWITCHED)?
                }
                None => (vec![], vec![]),
            };
            let reply = self
                .admit_media(
                    &stamp,
                    items.clone(),
                    &into,
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
        if let (Some(relinked), Some(sequence)) = (&relinked, &sequence)
            && let Some(note) = relinked.range_note(&sequence.asset)
        {
            outcome.notes.push(note);
        }
        outcome.notes.extend(sequence.and_then(|s| s.missing));
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

/// Project items placed on the timeline (dragged from the Project panel):
/// a layer per media asset as an import places it, a precomposition per
/// composition, at `at` from `index` in the stack. New layer ids and the
/// commands adding them; `Err` says in a sentence why nothing can be placed.
pub fn place_items(
    published: &Published,
    assets: &[String],
    compositions: &[String],
    at: f64,
    index: Option<usize>,
) -> Result<(Vec<String>, Vec<Command>), String> {
    let root = published.snapshot.root().ok_or("Open a project first")?;
    let records = root
        .get("assets")
        .map(|a| a.to_value())
        .unwrap_or(Value::Null);
    let items: Vec<(&str, &Value)> = assets
        .iter()
        .filter_map(|id| Some((id.as_str(), records.get(id)?)))
        .collect();
    let (mut layers, mut commands) =
        placement(published, &items, at, index, None).ok_or("Open a project first")?;
    let open = published.snapshot.open_comp();
    let comps = root
        .get("comps")
        .map(|c| c.to_value())
        .unwrap_or(Value::Null);
    for (n, id) in compositions.iter().enumerate() {
        if open.as_deref() == Some(id.as_str()) {
            return Err("A composition can't be placed inside itself".into());
        }
        let comp = comps
            .get(id)
            .ok_or("That composition is no longer in the project")?;
        let number =
            |key: &str, default: f64| comp.get(key).and_then(Value::as_f64).unwrap_or(default);
        let mut content = Map::new();
        content.insert("comp", Value::from(id.as_str()));
        content.insert("w", Value::from(number("w", 1920.)));
        content.insert("h", Value::from(number("h", 1080.)));
        let layer = pm_kernel::commands::new_id("L");
        layers.push(layer.clone());
        commands.push(
            LayerCommand::Add(Box::new(pm_kernel::commands::layers::AddLayer {
                id: layer,
                layer_type: "precomp".into(),
                comp: None,
                name: comp.get("name").and_then(Value::as_str).map(str::to_owned),
                from: None,
                duration: Some(number("dur", 1.)),
                content,
                properties: Map::new(),
                color: None,
                index: index.map(|i| i + items.len() + n),
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
    if layers.is_empty() {
        return Err("Nothing here can be placed on the timeline".into());
    }
    Ok((layers, commands))
}

/// `PM.commandForAsset` for each asset: new layer ids and the commands
/// adding them at `at`, from stack `index` down. `None`: no project to
/// place in.
fn placement(
    published: &Published,
    items: &[(&str, &Value)],
    at: f64,
    index: Option<usize>,
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
    for (n, &(id, asset)) in items.iter().enumerate() {
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
                index: index.map(|i| i + n),
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

    /// Project items dragged onto the timeline: each a layer at the time
    /// under the pointer, joining the stack where the pointer is, first item
    /// on top, in one Undo step; a composition nests; the open composition
    /// is refused in a sentence.
    #[test]
    fn project_items_are_placed_at_a_time_and_a_place_in_the_stack() {
        let f = fixture();
        let a = write(&f.root, "A.png", PNG);
        let b = write(&f.root, "B.png", PNG);
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![a, b],
                    // An existing layer first, then the two stills only in
                    // the Project panel.
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.imported, ["A.png", "B.png"], "{outcome:?}");
        let kernel = f.actor.handle();
        let commit = |commands: Vec<Command>| {
            let call = pm_kernel::Call::Transact {
                label: Some("Add media".into()),
                expected_revision: None,
                body: pm_kernel::WriteBody::Commands(commands),
            };
            let reply = kernel
                .call("p", Request::new(Caller::native(), call))
                .wait()
                .unwrap()
                .unwrap();
            assert!(!matches!(reply.reply, Reply::Throw(_)), "{:?}", reply.reply);
        };
        let solid = pm_kernel::commands::layers::AddLayer {
            id: "S".into(),
            layer_type: "solid".into(),
            comp: None,
            name: Some("Solid".into()),
            from: None,
            duration: Some(10.),
            content: Map::new(),
            properties: Map::new(),
            color: None,
            index: None,
            parent: None,
            blend: None,
            motion_blur: None,
            visible: None,
            shy: None,
            collapsed: None,
            time: 0.,
        };
        commit(vec![LayerCommand::Add(Box::new(solid)).into()]);
        let published = kernel.published("p").unwrap();
        let (placed, commands) =
            place_items(&published, &outcome.assets, &[], 1.5, Some(1)).unwrap();
        assert_eq!(placed.len(), 2);
        commit(commands);
        let published = kernel.published("p").unwrap();
        let open = published.snapshot.open_comp();
        let layers =
            pm_kernel::model::comp_node(published.snapshot.root().unwrap(), open.as_deref())
                .unwrap()
                .to_value()
                .get("layers")
                .unwrap()
                .as_array()
                .unwrap()
                .clone();
        let ids: Vec<&str> = layers
            .iter()
            .map(|l| l.get("id").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["S", placed[0].as_str(), placed[1].as_str()],
            "under the solid, A above B"
        );
        for layer in &layers[1..] {
            assert_eq!(layer.get("from").unwrap().as_f64(), Some(1.5));
        }
        assert_eq!(
            place_items(&published, &[], open.as_slice(), 0., None).unwrap_err(),
            "A composition can't be placed inside itself"
        );
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

    /// A 16-bit RGBA PNG of `w`×`h` samples `pixel(x, y)`.
    fn png16(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u16; 4]) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = !0u32;
            for b in bytes {
                c ^= u32::from(*b);
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xedb8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let start = out.len();
            out.extend(kind);
            out.extend(data);
            let sum = crc(&out[start..]);
            out.extend(sum.to_be_bytes());
        }
        let mut raw = vec![];
        for y in 0..h {
            raw.push(0);
            for x in 0..w {
                for v in pixel(x, y) {
                    raw.extend(v.to_be_bytes());
                }
            }
        }
        let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::fast());
        std::io::Write::write_all(&mut z, &raw).unwrap();
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = vec![];
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([16, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z.finish().unwrap());
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// Frame `number`'s samples: a red ramp far finer than 8 bits, green
    /// coding the frame number, and rows of full, partial and no alpha.
    fn coded(number: u32, x: u32, y: u32) -> [u16; 4] {
        let alpha = [65535, 40000, 0, 65535][y as usize];
        [
            30000 + x as u16 * 37,
            number as u16 * 9000,
            65535 - x as u16 * 500,
            alpha,
        ]
    }

    #[test]
    fn an_image_sequence_is_read_in_place_at_its_depth_with_its_alpha() {
        use pm_protocol::color::Precision;
        use pm_protocol::common::{Size, Ticks};
        use pm_render::{MapResolver, RenderRequest, Renderer};
        for (depth, precision) in [(16, Precision::Float16), (32, Precision::Float32)] {
            let (f, _session) = opened(&format!(
                r##"{{"id":"p","name":"Film","compId":"c","w":64,"h":4,"fps":23.976,"dur":10,"bg":"#000000","colorDepth":{depth},"assets":{{}},"comps":{{}},"layers":[]}}"##,
            ));
            // Numbers 1 to 6 at 23.976 with 4 never rendered.
            let frames: Vec<_> = [6, 1, 3, 2, 5]
                .iter()
                .map(|&n| {
                    write(
                        &f.root,
                        &format!("shot.{n:04}.png"),
                        &png16(64, 4, |x, y| coded(n, x, y)),
                    )
                })
                .collect();
            let progress = Arc::new(Mutex::new(String::new()));
            let outcome = f
                .runtime
                .import_media(
                    document(&f),
                    &f.root,
                    ImportRequest {
                        paths: frames.clone(),
                        place_at: Some(0.),
                        sequence: Some(SequenceImport {
                            fps: 23.976,
                            progress: progress.clone(),
                        }),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert_eq!(outcome.imported, ["shot.[0001-0006].png"], "{outcome:?}");
            assert_eq!(
                outcome.summary(),
                "Imported shot.[0001-0006].png · frame 4 missing"
            );
            let published = f.actor.handle().published("p").unwrap();
            let root = published.snapshot.root().unwrap().to_value();
            let id = pm_kernel::asset_order::keys(&root).remove(0);
            let asset = root.get("assets").unwrap().get(&id).unwrap();
            let field = |key: &str| asset.get(key).and_then(Value::as_f64);
            assert_eq!(asset.get("kind").unwrap().as_str(), Some("video"));
            assert_eq!((field("w"), field("h")), (Some(64.), Some(4.)));
            assert_eq!(asset.get("hasAudio").unwrap().as_bool(), Some(false));
            let sequence = asset.get("imageSequence").unwrap();
            let number = |key: &str| sequence.get(key).and_then(Value::as_f64);
            assert_eq!(
                (number("first"), number("last"), number("frames")),
                (Some(1.), Some(6.), Some(6.))
            );
            assert!((field("dur").unwrap() - 6. * 1001. / 24000.).abs() < 1e-9);
            // The asset retains the reference, not the pixels.
            let manifest = f.root.join("retained.json");
            published
                .media
                .get(&id)
                .unwrap()
                .copy_to(&manifest)
                .unwrap();
            assert!(std::fs::metadata(&manifest).unwrap().len() < 4096);
            // The Project panel reads it as ready media, not a failed video.
            assert_eq!(crate::delivery::probe_media(&manifest, "video"), Ok(()));

            // A frame rendered into the hole later is picked up; one deleted
            // after import is missing, never another frame.
            write(
                &f.root,
                "shot.0004.png",
                &png16(64, 4, |x, y| coded(4, x, y)),
            );
            std::fs::remove_file(f.root.join("shot.0005.png")).unwrap();
            let mut resolver = MapResolver::default();
            resolver.paths.insert(id.clone(), manifest);
            let document = &published.snapshot.document;
            let comp = published.snapshot.open_comp().unwrap();
            {
                let mut renderer = Renderer::headless().unwrap();
                // At 16 bits a sample is within half a half-float step; an
                // 8-bit read would be off by up to 1/510.
                let tolerance = if precision == Precision::Float16 {
                    3e-4
                } else {
                    2e-6
                };
                for k in 0..6u32 {
                    // Composition frame k starts at k × 1001/24000 s exactly.
                    let time = Ticks::new(i64::from(k) * 29_429_400).unwrap();
                    // No background: the frame keeps the files' alpha.
                    let mut request = RenderRequest::sized(Size::new(64, 4), precision);
                    request.background = false;
                    let frame = renderer
                        .render(document, &comp, time, &request, &resolver)
                        .unwrap();
                    let shown = k + 1;
                    if shown == 5 {
                        assert_eq!(frame.failures.len(), 1, "{:?}", frame.failures);
                        assert!(
                            frame.failures[0].message.contains(
                                "frame 5 of the image sequence is missing: shot.0005.png"
                            ),
                            "{:?}",
                            frame.failures
                        );
                        continue;
                    }
                    assert!(
                        frame.failures.is_empty(),
                        "{precision:?} frame {k}: {:?}",
                        frame.failures
                    );
                    let pixels = renderer.read_rgba(&frame).unwrap();
                    let mut reds = std::collections::BTreeSet::new();
                    for y in 0..4 {
                        for x in 0..64 {
                            let want = coded(shown, x, y).map(|v| f32::from(v) / 65535.);
                            let at = ((y * 64 + x) * 4) as usize;
                            let got = &pixels[at..at + 4];
                            // Premultiplied, as the frame is read back.
                            let want = [
                                want[0] * want[3],
                                want[1] * want[3],
                                want[2] * want[3],
                                want[3],
                            ];
                            for c in 0..4 {
                                assert!(
                                    (got[c] - want[c]).abs() <= tolerance,
                                    "{precision:?} comp frame {k} (file {shown}) pixel {x},{y} channel {c}: {} vs {}",
                                    got[c],
                                    want[c]
                                );
                            }
                            if y == 0 {
                                reds.insert(got[0].to_bits());
                            }
                        }
                    }
                    // 64 reds within nine 8-bit codes stay apart.
                    let apart = if precision == Precision::Float16 {
                        40
                    } else {
                        64
                    };
                    assert!(
                        reds.len() >= apart,
                        "{precision:?}: {} distinct reds",
                        reds.len()
                    );
                }
            }
        }
    }

    /// A sequence whose folder was renamed is relinked by choosing one of its
    /// files there (Replace): the numbering and range are read again from
    /// the new folder, a changed range and missing numbers are reported, and
    /// its layers show the new folder's files.
    #[test]
    fn a_sparse_or_mixed_size_sequence_is_refused_in_plain_words() {
        let (f, _session) = opened(
            r##"{"id":"p","name":"Film","compId":"c","w":64,"h":4,"fps":24,"dur":10,"bg":"#000000","colorDepth":16,"assets":{},"comps":{},"layers":[]}"##,
        );
        let import = |request: ImportRequest| {
            f.runtime
                .import_media(document(&f), &f.root, request, &AtomicBool::new(false))
        };
        let sequence = || {
            Some(SequenceImport {
                fps: 24.,
                progress: Default::default(),
            })
        };
        // Two files a billion numbers apart: refused at once, with no look
        // at the numbers between.
        let sparse = f.root.join("sparse");
        std::fs::create_dir(&sparse).unwrap();
        let far: Vec<_> = [1, 1_000_000_001]
            .iter()
            .map(|n| {
                write(
                    &sparse,
                    &format!("shot.{n:010}.png"),
                    &png16(64, 4, |x, y| coded(1, x, y)),
                )
            })
            .collect();
        let started = std::time::Instant::now();
        let refused = import(ImportRequest {
            paths: far,
            sequence: sequence(),
            ..Default::default()
        })
        .unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(
            refused,
            "These frames are numbered 1 to 1000000001, too far apart to be one sequence. Choose frames numbered closer together."
        );
        // A frame rendered at another size.
        let renders = f.root.join("renders");
        std::fs::create_dir(&renders).unwrap();
        let mut frames: Vec<_> = (1..=3)
            .map(|n| {
                write(
                    &renders,
                    &format!("shot.{n:04}.png"),
                    &png16(64, 4, |x, y| coded(n, x, y)),
                )
            })
            .collect();
        let odd = write(
            &renders,
            "shot.0004.png",
            &png16(32, 4, |x, y| coded(4, x, y)),
        );
        frames.push(odd.clone());
        let words = "shot.0004.png is 32×4, but the sequence’s first frame is 64×4. Render every frame at one size.";
        let refused = import(ImportRequest {
            paths: frames.clone(),
            sequence: sequence(),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(refused, words);
        // Imported without it, then relinked to a folder holding it.
        let imported = import(ImportRequest {
            paths: frames[..3].to_vec(),
            sequence: sequence(),
            ..Default::default()
        })
        .unwrap();
        let refused = import(ImportRequest {
            paths: vec![frames[0].clone()],
            replace: Some(imported.assets[0].clone()),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(refused, words);
    }

    #[test]
    fn a_sequence_relinked_by_one_of_its_files_reads_its_numbering_again() {
        use pm_protocol::color::Precision;
        use pm_protocol::common::{Size, Ticks};
        use pm_render::{MapResolver, RenderRequest, Renderer};
        let (f, _session) = opened(
            r##"{"id":"p","name":"Film","compId":"c","w":64,"h":4,"fps":24,"dur":10,"bg":"#000000","colorDepth":16,"assets":{},"comps":{},"layers":[]}"##,
        );
        let renders = f.root.join("renders");
        std::fs::create_dir(&renders).unwrap();
        let frames: Vec<_> = (1..=6)
            .map(|n| {
                write(
                    &renders,
                    &format!("shot.{n:04}.png"),
                    &png16(64, 4, |x, y| coded(n, x, y)),
                )
            })
            .collect();
        let import = |request: ImportRequest| {
            f.runtime
                .import_media(document(&f), &f.root, request, &AtomicBool::new(false))
                .unwrap()
        };
        let imported = import(ImportRequest {
            paths: frames,
            place_at: Some(0.),
            sequence: Some(SequenceImport {
                fps: 24.,
                progress: Default::default(),
            }),
            ..Default::default()
        });
        let id = imported.assets[0].clone();
        // The folder is renamed; a frame is rendered past the end, one is lost.
        let moved = f.root.join("renders v2");
        std::fs::rename(&renders, &moved).unwrap();
        write(
            &moved,
            "shot.0007.png",
            &png16(64, 4, |x, y| coded(7, x, y)),
        );
        std::fs::remove_file(moved.join("shot.0003.png")).unwrap();
        let relinked = import(ImportRequest {
            paths: vec![moved.join("shot.0005.png")],
            replace: Some(id.clone()),
            ..Default::default()
        });
        assert_eq!(
            relinked.summary(),
            "Imported shot.[0001-0007].png · frames 1–7, were 1–6 · frame 3 missing"
        );
        assert_eq!(relinked.assets, std::slice::from_ref(&id));
        let published = f.actor.handle().published("p").unwrap();
        let root = published.snapshot.root().unwrap().to_value();
        let asset = root.get("assets").unwrap().get(&id).unwrap();
        let sequence = asset.get("imageSequence").unwrap();
        let number = |key: &str| sequence.get(key).and_then(Value::as_f64);
        assert_eq!(
            (
                number("first"),
                number("last"),
                number("frames"),
                number("fps")
            ),
            (Some(1.), Some(7.), Some(7.), Some(24.))
        );
        let manifest = f.root.join("retained.json");
        published
            .media
            .get(&id)
            .unwrap()
            .copy_to(&manifest)
            .unwrap();
        let read = pm_media::image::sequence::Manifest::read(&manifest)
            .unwrap()
            .unwrap();
        assert_eq!(read.dir, moved);
        let mut resolver = MapResolver::default();
        resolver.paths.insert(id.clone(), manifest);
        let comp = published.snapshot.open_comp().unwrap();
        let mut renderer = Renderer::headless().unwrap();
        let mut request = RenderRequest::sized(Size::new(64, 4), Precision::Float16);
        request.background = false;
        let at = |frame: i64| Ticks::new(frame * Ticks::from_whole_seconds(1).get() / 24).unwrap();
        // The placed layer keeps its length (six frames): its fifth shows
        // file 5, read from the new folder (the old one is gone).
        let fifth = renderer
            .render(
                &published.snapshot.document,
                &comp,
                at(4),
                &request,
                &resolver,
            )
            .unwrap();
        assert!(fifth.failures.is_empty(), "{:?}", fifth.failures);
        let green = renderer.read_rgba(&fifth).unwrap()[1];
        assert!((green - f32::from(coded(5, 0, 0)[1]) / 65535.).abs() < 3e-4);
        let lost = renderer
            .render(
                &published.snapshot.document,
                &comp,
                at(2),
                &request,
                &resolver,
            )
            .unwrap();
        assert!(
            lost.failures[0]
                .message
                .contains("frame 3 of the image sequence is missing: shot.0003.png"),
            "{:?}",
            lost.failures
        );
    }

    #[test]
    fn stills_and_sound_a_pro_receives_import_and_draw_at_their_depth() {
        use pm_protocol::color::Precision;
        use pm_protocol::common::{Size, Ticks};
        use pm_render::{MapResolver, RenderRequest, Renderer};
        let scratch = tempfile::tempdir().unwrap();
        let make = |name: &str, args: &[&str]| {
            let path = scratch.path().join(name);
            let status = std::process::Command::new(ffmpeg())
                .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
                .args(args)
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success(), "{name}");
            path
        };
        let color = "color=c=0x336699:s=16x16";
        let one = ["-frames:v", "1"];
        // (file, decoded bits, float, scene-linear samples)
        let stills = [
            (
                make("deep.tif", &[color, one[0], one[1], "-pix_fmt", "rgba64le"]),
                16,
                false,
                false,
            ),
            (
                make(
                    "half.exr",
                    &[
                        color,
                        one[0],
                        one[1],
                        "-pix_fmt",
                        "gbrapf32le",
                        "-format",
                        "half",
                    ],
                ),
                16,
                true,
                true,
            ),
            (
                make(
                    "float.exr",
                    &[
                        color,
                        one[0],
                        one[1],
                        "-pix_fmt",
                        "gbrpf32le",
                        "-format",
                        "float",
                    ],
                ),
                32,
                true,
                true,
            ),
        ];
        let srgb = |v: f32| {
            if v <= 0.003_130_8 {
                v * 12.92
            } else {
                1.055 * v.powf(1. / 2.4) - 0.055
            }
        };
        // Imported into a 32-bit project and drawn alone: the centre pixel.
        let drawn = |path: &PathBuf| -> Vec<f32> {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let (f, _session) = opened(
                r##"{"id":"p","name":"Film","compId":"c","w":16,"h":16,"fps":30,"dur":1,"bg":"#000000","colorDepth":32,"assets":{},"comps":{},"layers":[]}"##,
            );
            let outcome = f
                .runtime
                .import_media(
                    document(&f),
                    &f.root,
                    ImportRequest {
                        paths: vec![path.clone()],
                        place_at: Some(0.),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert_eq!(outcome.imported, std::slice::from_ref(&name), "{outcome:?}");
            let published = f.actor.handle().published("p").unwrap();
            let root = published.snapshot.root().unwrap().to_value();
            let id = pm_kernel::asset_order::keys(&root).remove(0);
            let retained = f.root.join(&name);
            published
                .media
                .get(&id)
                .unwrap()
                .copy_to(&retained)
                .unwrap();
            let mut resolver = MapResolver::default();
            resolver.paths.insert(id, retained);
            let mut renderer = Renderer::headless().unwrap();
            let frame = renderer
                .render(
                    &published.snapshot.document,
                    &published.snapshot.open_comp().unwrap(),
                    Ticks::ZERO,
                    &RenderRequest::sized(Size::new(16, 16), Precision::Float32),
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{name}: {:?}", frame.failures);
            renderer.read_rgba(&frame).unwrap()[(8 * 16 + 8) * 4..][..4].to_vec()
        };
        for (path, bits, float, linear) in &stills {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let still = pm_media::image::read(path, &pm_media::pool::FramePool::new(0)).unwrap();
            assert_eq!((still.bit_depth, still.float), (*bits, *float), "{name}");
            let pixel = drawn(path);
            for (got, code) in pixel.iter().zip([0x33, 0x66, 0x99]) {
                let v = code as f32 / 255.;
                // Scene-linear files hold the code value as light.
                let want = if *linear { srgb(v) } else { v };
                assert!((got - want).abs() < 0.01, "{name}: {pixel:?}");
            }
        }
        // Film scans and design files: DPX at 10, 12 and 16 bits, TGA with
        // alpha, a 16-bit Photoshop document, each from exact samples and
        // drawn finer than an 8-bit step (each code is further than the
        // tolerance from one).
        let codes = [202u16, 410, 618].map(|c| c << 6 | c >> 4);
        let raw = scratch.path().join("codes.rgb48");
        std::fs::write(
            &raw,
            codes
                .iter()
                .flat_map(|c| c.to_le_bytes())
                .collect::<Vec<_>>()
                .repeat(256),
        )
        .unwrap();
        let from_codes = |name: &str, args: &[&str]| {
            let path = scratch.path().join(name);
            let status = std::process::Command::new(ffmpeg())
                .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb48le"])
                .args(["-s", "16x16", "-i"])
                .arg(&raw)
                .args(["-frames:v", "1"])
                .args(args)
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success(), "{name}");
            path
        };
        let mut psd = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
        psd.extend([16u32, 16].iter().flat_map(|v| v.to_be_bytes()));
        psd.extend([0, 16, 0, 3]);
        psd.extend([0; 14]);
        for c in codes {
            psd.extend(c.to_be_bytes().repeat(256));
        }
        let document_path = scratch.path().join("plate.psd");
        std::fs::write(&document_path, psd).unwrap();
        let unit = |c: u16, bits: u32| f32::from(c >> (16 - bits)) / ((1u32 << bits) - 1) as f32;
        let deep = [
            (from_codes("scan10.dpx", &["-pix_fmt", "gbrp10le"]), 10),
            (from_codes("scan12.dpx", &["-pix_fmt", "gbrp12le"]), 12),
            (from_codes("scan16.dpx", &["-pix_fmt", "rgb48le"]), 16),
            (document_path, 16),
        ];
        for (path, bits) in &deep {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let pixel = drawn(path);
            for (got, code) in pixel.iter().zip(codes) {
                let want = unit(code, *bits);
                assert!((got - want).abs() < 1e-4, "{name}: {pixel:?} vs {want}");
            }
        }
        // An 8-bit TGA with straight alpha at one half, top row first,
        // drawn over the project's black.
        let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 16, 0, 32, 0x28];
        let [r, g, b] = codes.map(|c| (c >> 8) as u8);
        tga.extend([b, g, r, 128].repeat(256));
        let matte = scratch.path().join("matte.tga");
        std::fs::write(&matte, tga).unwrap();
        let pixel = drawn(&matte);
        for (got, code) in pixel.iter().zip(codes) {
            let want = unit(code, 8) * 128. / 255.;
            assert!((got - want).abs() < 1e-4, "matte.tga: {pixel:?} vs {want}");
        }
        let tone = ["sine=f=440:d=1:sample_rate=48000"];
        for (name, codec) in [("take.caf", "pcm_s24le"), ("take.aifc", "pcm_f32be")] {
            let format = if name.ends_with("caf") { "caf" } else { "aiff" };
            let path = make(name, &[tone[0], "-c:a", codec, "-f", format]);
            let f = fixture();
            let outcome = f
                .runtime
                .import_media(
                    document(&f),
                    &f.root,
                    ImportRequest {
                        paths: vec![path],
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert_eq!(outcome.imported, [name], "{outcome:?}");
            let root = f
                .actor
                .handle()
                .published("p")
                .unwrap()
                .snapshot
                .root()
                .unwrap()
                .to_value();
            let asset = root
                .get("assets")
                .unwrap()
                .get(&pm_kernel::asset_order::keys(&root)[0])
                .unwrap()
                .clone();
            let field = |key: &str| asset.get(key).and_then(Value::as_f64);
            assert_eq!(asset.get("kind").unwrap().as_str(), Some("audio"), "{name}");
            assert_eq!(
                (field("channels"), field("sampleRate")),
                (Some(1.), Some(48000.)),
                "{name}"
            );
            assert!(
                (field("dur").unwrap() - 1.).abs() < 0.01,
                "{name}: {asset:?}"
            );
        }
        // Camera raw is named, not "unsupported".
        let f = fixture();
        let raw = write(&f.root, "A001.dng", b"II*\0");
        let outcome = f
            .runtime
            .import_media(
                document(&f),
                &f.root,
                ImportRequest {
                    paths: vec![raw],
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(outcome.failed, [("A001.dng".to_owned(), RAW.to_owned())]);
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
        assert_eq!(outcome.failed[0], ("notes.txt".into(), UNSUPPORTED.into()));
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
