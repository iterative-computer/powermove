//! Import Media. Probing and byte retention run on the caller's worker; admission
//! goes through the kernel actor with `RetainedAsset` receipts, one Undo step.
use crate::{Builtins, RetainedAsset};
use pm_kernel::{
    Call, Caller, Command, LayerCommand, Reply, Request, SelectionCall, WriteBody,
    media_archive::Archive,
};
use pm_model::{Map, Value};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
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

/// `assetIdentity`: what the Project panel and layers read from an asset.
fn metadata(path: &Path, kind: &str, size: u64) -> Result<Value, String> {
    let facts = pm_media::facts::media_facts(path, None).map_err(|e| e.to_string())?;
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
        asset["audioDur"] = json!(if facts.audio.is_some() {
            facts.duration.unwrap_or(0.)
        } else {
            0.
        });
    }
    if kind == "image" {
        asset["format"] = json!(extension(path));
    }
    crate::data::model(&asset)
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
}
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub imported: Vec<String>,
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
/// Concurrent edits move the revision; the receipts stay valid across retries.
const ADMIT_ATTEMPTS: usize = 8;

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
        let previous = match &request.replace {
            Some(id) => {
                let published = self.broker.kernel.published(&project).ok_or(SWITCHED)?;
                let entry = published.media.get(id).cloned();
                entry
                    .map(|entry| archive.retain_entry(&entry))
                    .transpose()
                    .map_err(|e| e.to_string())?
            }
            None => None,
        };
        for path in expand(request.paths) {
            if cancel.load(Ordering::Acquire) {
                outcome.cancelled = true;
                break;
            }
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let Some(kind) = kind(&path) else {
                outcome.failed.push((name, "Unsupported file".into()));
                continue;
            };
            let retained = decodes(&path, kind)
                .and_then(|()| {
                    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                    metadata(&path, kind, size)
                })
                .and_then(|metadata| {
                    let original = archive
                        .retain_path(&path, &mime(&path, kind), cancel)
                        .map_err(|e| e.to_string())?;
                    Ok(RetainedAsset {
                        id: pm_kernel::new_id("a"),
                        metadata,
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
        let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
        let names: Vec<String> = items
            .iter()
            .filter_map(|item| Some(item.metadata.get("name")?.as_str()?.to_owned()))
            .collect();
        let mut attempt = 0;
        loop {
            attempt += 1;
            let published = self.broker.kernel.published(&project).ok_or(SWITCHED)?;
            if published.snapshot.doc_id != document {
                return Err(SWITCHED.into());
            }
            let stamp = pm_panel_media::Catalog::published(&published)
                .map_err(|e| format!("{e:?}"))?
                .stamp;
            let reply = self
                .admit_media(
                    &stamp,
                    items.clone(),
                    request.folder.clone(),
                    request.replace.clone(),
                    Caller::native(),
                )?
                .wait()
                .ok_or("Project closed")?;
            let stale = self
                .broker
                .kernel
                .published(&project)
                .is_some_and(|p| p.revision() != stamp.revision);
            match reply {
                Ok(ack) if !matches!(ack.reply, Reply::Throw(_)) => break,
                _ if stale && attempt < ADMIT_ATTEMPTS => continue,
                Ok(ack) => return Err(format!("{:?}", ack.reply)),
                Err(error) => return Err(error.to_string()),
            }
        }
        if let Some(at) = request.place_at.filter(|_| request.replace.is_none()) {
            self.place_layers(&project, &ids, at)?;
        }
        outcome.imported = names;
        Ok(outcome)
    }

    /// `PM.commandForAsset` for each imported asset, as one Undo step.
    fn place_layers(&self, project: &str, ids: &[String], at: f64) -> Result<(), String> {
        let published = self.broker.kernel.published(project).ok_or(SWITCHED)?;
        let root = published.snapshot.root().ok_or(SWITCHED)?;
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
        for id in ids {
            let asset = root
                .get("assets")
                .and_then(|a| a.get(id))
                .ok_or("The media item is no longer in the project")?
                .to_value();
            let field = |key: &str| asset.get(key).and_then(Value::as_f64).filter(|n| *n > 0.);
            let kind = asset.get("kind").and_then(Value::as_str).unwrap_or("image");
            let mut content = Map::new();
            content.insert("asset", Value::from(id.as_str()));
            match kind {
                "audio" => {
                    for (key, value) in
                        [("trim", 0.), ("gain", 1.), ("fadeIn", 0.), ("fadeOut", 0.)]
                    {
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
                content.insert("w", Value::from(field("w").unwrap_or(w)));
                content.insert("h", Value::from(field("h").unwrap_or(h)));
            }
            let layer = pm_kernel::commands::new_id("L");
            layers.push(layer.clone());
            let name = asset.get("name").and_then(Value::as_str).map(str::to_owned);
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
        let label = match commands.len() {
            1 => "Import file".to_owned(),
            n => format!("Import {n} files"),
        };
        let call = |call: Call| {
            self.broker
                .kernel
                .call(project, Request::new(Caller::native(), call))
                .wait()
                .ok_or_else(|| SWITCHED.to_owned())?
                .map_err(|e| e.to_string())
        };
        let ack = call(Call::Transact {
            label: Some(label),
            body: WriteBody::Commands(commands),
            expected_revision: None,
        })?;
        if let Reply::Throw(error) = ack.reply {
            return Err(error.to_string());
        }
        call(
            SelectionCall::Layers {
                ids: layers,
                add: false,
            }
            .into(),
        )?;
        Ok(())
    }
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
        let project = pm_model::json::parse(
            r##"{"id":"p","name":"Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[]}"##,
        )
        .unwrap();
        kernel
            .shell(ShellCall::Open {
                id: "p".into(),
                source: SessionSource::New(project),
                electron_profile: None,
            })
            .wait()
            .unwrap()
            .unwrap();
        let runtime = Builtins::new(kernel, "p", || {}).unwrap();
        Fixture {
            _dir: dir,
            root,
            actor,
            runtime,
        }
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
}
