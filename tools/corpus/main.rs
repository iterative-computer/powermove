//! Private corpus worker. Only the Python driver's allowlisted results are public.
use anyhow::{Result, ensure};
use pm_engine_service::{EngineService, MediaCatalog, Output, ViewerConfig, jobs::*};
use pm_kernel::{KernelActor, KernelConfig, LibraryStore, library::StoreKey, session::*};
use pm_mcp::transport::ToolConnection;
use pm_native_migrate::{
    indexeddb,
    snapshot::{BlobCopy, Snapshot},
};
use pm_protocol::{
    clock::TransportCommand,
    color::{OutputColor, Precision},
    common::{FrameRate, PlaybackRate, SessionId, Size, TickRange, Ticks},
    priority::*,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "../../apps/native/crates/app/src/mcp.rs"]
mod app_mcp;
// Only referenced by the unused stdio launcher in the app's shared MCP adapter.
mod session {
    pub fn stable_id(path: &std::path::Path) -> String {
        format!(
            "corpus-{}",
            &blake3::hash(path.as_os_str().as_encoded_bytes()).to_hex()[..16]
        )
    }
}
mod media;
mod rotation;
mod rotation_pixels;
use media::project_assets;

const FLOOR: u64 = 6 * 1024 * 1024 * 1024;
fn space(root: &Path) -> Result<()> {
    ensure!(fs2::available_space(root)? >= FLOOR, "disk_floor");
    Ok(())
}
fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64).unwrap()
}
fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write(path: &Path, v: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(v)?;
    ensure!(
        fs2::available_space(path.parent().unwrap())?.saturating_sub(bytes.len() as u64) >= FLOOR,
        "disk_floor"
    );
    fs::write(path, bytes)?;
    Ok(())
}
fn anon(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex()[..16].to_owned()
}

fn value(v: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(v)).unwrap()
}
fn extensions(v: &Value) -> bool {
    static BUILTINS: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    let builtins = BUILTINS.get_or_init(|| {
        pm_render::all_builtin_definitions()
            .definitions
            .into_iter()
            .map(|d| d.id.as_str().to_owned())
            .collect()
    });
    match v {
        Value::Object(m) => {
            let dynamic_effect = m.get("fx").and_then(Value::as_array).is_some_and(|fx| {
                fx.iter().any(|f| {
                    f["type"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty() && !builtins.contains(id))
                })
            });
            let dynamic_transition = ["transitionIn", "transitionOut"].iter().any(|k| {
                m.get(*k)
                    .and_then(|t| t.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty() && id != "none" && !builtins.contains(id))
            });
            dynamic_effect
                || dynamic_transition
                || m.get("ty")
                    .or_else(|| m.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|s| {
                        (s == "extension"
                            && m.get("d")
                                .or_else(|| m.get("c"))
                                .and_then(|d| d.get("definition"))
                                .and_then(Value::as_str)
                                .is_none_or(|id| !builtins.contains(id)))
                            || s.starts_with("ext.")
                            || s.starts_with("ext:")
                    })
                || m.get("extensionId")
                    .or_else(|| m.get("extension"))
                    .is_some_and(|x| x.as_str().is_some_and(|s| !s.is_empty()))
                || m.iter().any(|(k, x)| k != "history" && extensions(x))
        }
        Value::Array(a) => a.iter().any(extensions),
        _ => false,
    }
}
fn discover_paths(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for k in ["sourcePath", "path"] {
                if let Some(s) = m.get(k).and_then(Value::as_str) {
                    if Path::new(s).is_absolute() {
                        out.insert(s.into());
                    }
                }
            }
            for x in m.values() {
                discover_paths(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                discover_paths(x, out);
            }
        }
        _ => {}
    }
}
fn inventory(root: &Path) -> Result<()> {
    let store = LibraryStore::open(&root.join("profile"));
    let mut ids = BTreeSet::new();
    for k in store.keys()? {
        if let StoreKey::Dynamic { prefix, id } = k
            && prefix == "project"
        {
            ids.insert(id);
        }
    }
    for row in store.projects()?.into_iter().chain(store.trash()?) {
        ids.insert(row.id);
    }
    if let Some(v) = store.read("autosave")? {
        if let Some(id) = v
            .get("proj")
            .unwrap_or(&v)
            .get("id")
            .and_then(pm_model::Value::as_str)
        {
            ids.insert(id.into());
        }
    }
    let mut rows = Vec::new();
    let mut paths = BTreeSet::new();
    let mut linked = BTreeSet::new();
    for id in ids {
        let mut row = json!({"id":anon(&format!("library:{id}")),"library":id});
        match store.load(&id) {
            Ok(Some(p)) => {
                row["extension"] = json!(extensions(&value(&p.project)));
                row["assets"] = project_assets(&value(&p.project));
                discover_paths(&value(&p.project), &mut linked);
                if let Some(s) = p.session {
                    let mut associations = Vec::new();
                    for k in ["file", "embeddedMedia"] {
                        if let Some(p) = s
                            .get(k)
                            .and_then(|x| x.get("path"))
                            .and_then(pm_model::Value::as_str)
                        {
                            paths.insert(p.to_owned());
                            associations.push(p.to_owned());
                        }
                    }
                    row["associated"] = json!(associations);
                }
            }
            _ => row["inventory_open_failed"] = json!(true),
        }
        rows.push(row);
    }
    write(
        &root.join("inventory.json"),
        &json!({"projects":rows,"associated_files":paths,"linked_files":linked}),
    )
}
fn snapshot(db: &Path, blobs: &Path, target: &Path) -> Result<()> {
    let s = Snapshot::capture(
        db,
        blobs.is_dir().then_some(blobs),
        target,
        BlobCopy::Deferred { min_free: FLOOR },
    )?;
    let records = indexeddb::read(&s)?.records;
    let mut list = Vec::new();
    for r in records {
        if !r.poster {
            list.push(json!({"key":r.key,"revision":r.revision(),"path":r.payload_path(&s)?,"snapshot":target,"database":r.database,"number":r.payload().number,"size":r.payload().size,"value":r.value}));
        }
    }
    write(&target.join("records.json"), &json!(list))?;
    println!(
        "{}",
        json!({"records":list.len(),"blobs":s.blob_counts().2,"bytes":s.blob_counts().3,"complete":s.complete()})
    );
    Ok(())
}
fn result(root: &Path, stage: &str, state: &str) -> Result<()> {
    let p = root.join("result.json");
    let mut v = if p.exists() { read(&p)? } else { json!({}) };
    v[stage] = json!(state);
    write(&p, &v)
}
fn attempt(root: &Path, stage: &str, f: impl FnOnce() -> Result<()>) -> Result<bool> {
    result(root, "running", stage)?;
    match f() {
        Ok(()) => {
            result(root, stage, "pass")?;
            Ok(true)
        }
        Err(e) => {
            fs::write(root.join(format!("{stage}.private.log")), format!("{e:#}"))?;
            result(root, stage, "fail")?;
            Ok(false)
        }
    }
}
fn invoke(c: &mut app_mcp::Connection, name: &str, args: Value) -> Result<Value> {
    let v = c.invoke(name, args)?;
    ensure!(v["isError"] != true, "MCP rejected: {v}");
    Ok(v)
}
fn wait(
    v: &pm_engine_service::Viewer,
    time: Option<Ticks>,
    after: Option<f64>,
) -> Result<pm_engine_service::FrameReport> {
    let start = Instant::now();
    loop {
        let s = v.status();
        ensure!(s.error.is_none(), "viewer: {:?}", s.error);
        if let Some(f) = s.frame
            && f.gpu_ok
            && time.is_none_or(|t| f.time == t)
            && after.is_none_or(|a| f.completion_host_seconds.is_some_and(|x| x > a))
        {
            return Ok(f);
        }
        ensure!(start.elapsed() < Duration::from_secs(15), "frame_timeout");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn scrub(
    v: &pm_engine_service::Viewer,
    time: Ticks,
    frame_ticks: i64,
) -> Result<pm_engine_service::FrameReport> {
    let before = v.status().frame.and_then(|f| f.completion_host_seconds);
    let snapped = Ticks::new((time.get() / frame_ticks) * frame_ticks)?;
    v.scrub(snapped)?;
    wait(v, Some(snapped), before)
}
fn catalog(root: &Path, p: &pm_kernel::Published, work: &Path) -> Result<MediaCatalog> {
    let mut media = MediaCatalog::default();
    let doc = value(&p.snapshot.document.to_value().unwrap());
    for (i, m) in p.media.iter().enumerate() {
        space(work)?;
        ensure!(
            fs2::available_space(work)?.saturating_sub(m.length()) >= FLOOR,
            "embedded_media_disk_floor"
        );
        let dest = work.join(format!("embedded-{i}"));
        m.copy_to(&dest)?;
        let id = m.id.to_string();
        if media::proxy(&doc["assets"][&id]) {
            media.proxies.insert(id, dest);
        } else {
            media.originals.insert(id, dest);
        }
    }
    let map = read(&root.join("media.json"))?;
    if let Some(assets) = doc["assets"].as_object() {
        for (id, a) in assets {
            if media.originals.contains_key(id) || media.proxies.contains_key(id) {
                continue;
            }
            if let Some(binding) = media::binding(id, a, &map).map_err(anyhow::Error::msg)? {
                binding.verify(root).map_err(anyhow::Error::from_boxed)?;
                if binding.proxy {
                    media.proxies.insert(id.clone(), binding.path);
                } else {
                    media.originals.insert(id.clone(), binding.path);
                }
            }
        }
    }
    Ok(media)
}
fn project(
    root: &Path,
    spec: &Path,
    work: &Path,
    fonts: pm_node::fonts::render::RenderFonts,
) -> Result<()> {
    space(root)?;
    let row = read(spec)?;
    result(work, "running", "open")?;
    fs::create_dir_all(work.join("profile/store"))?;
    let profile = work.join("profile");
    let source = if let Some(id) = row["library"].as_str() {
        let lib = LibraryStore::open(&root.join("profile"))
            .load(id)?
            .ok_or_else(|| anyhow::anyhow!("missing_slot"))?;
        let mut metadata = lib.session.as_ref().map(value).unwrap_or(json!({}));
        let paths = read(&root.join("media.json"))?;
        for key in ["file", "embeddedMedia"] {
            if let Some(src) = metadata[key]["path"].as_str().map(str::to_owned) {
                if let Some(copy) = paths["linked"][&src].as_str() {
                    metadata[key]["path"] = json!(copy);
                } else {
                    metadata.as_object_mut().unwrap().remove(key);
                }
            }
        }
        // Only this project's private slot is writable. No full-profile compaction.
        write(
            &profile.join("store/project.corpus.json"),
            &value(&lib.project),
        )?;
        if let Some(h) = lib.history {
            write(
                &profile.join("store/projectHistory.corpus.json"),
                &value(&h),
            )?;
        }
        write(&profile.join("store/projectMeta.corpus.json"), &metadata)?;
        SessionSource::Library {
            profile: profile.clone(),
            project: "corpus".into(),
        }
    } else {
        SessionSource::File(PathBuf::from(row["file"].as_str().unwrap()))
    };
    let metadata_bytes = fs::read_dir(profile.join("store"))?.try_fold(0u64, |n, e| {
        Ok::<_, std::io::Error>(n.saturating_add(e?.metadata()?.len()))
    })?;
    ensure!(
        fs2::available_space(work)?.saturating_sub(metadata_bytes.saturating_mul(2)) >= FLOOR,
        "disk_floor: open checkpoint reserve required"
    );
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(profile.join("journals")),
        ..Default::default()
    });
    let (host, _) = app_mcp::Host::new(actor.handle(), profile.clone());
    app_mcp::attach(&host.kernel, &profile)?;
    if !attempt(work, "open", || {
        let reply = host
            .kernel
            .shell(ShellCall::PrepareOpen {
                id: "corpus".into(),
                source,
                electron_profile: None,
            })
            .wait()
            .ok_or_else(|| anyhow::anyhow!("kernel_stopped"))??;
        match reply {
            ShellReply::Opened {
                session, decision, ..
            } => {
                // The entire corpus is already an authorized private copy. Honor
                // kernel safe-copy decisions instead of requiring a writable base.
                if matches!(decision, pm_kernel::locks::OpenDecision::AsCopy(_)) {
                    result(work, "open_mode", "as-copy")?;
                }
                host.bind(Some(session));
                Ok(())
            }
            other => anyhow::bail!("kernel open requires recovery: {other:?}"),
        }
    })? {
        actor.shutdown();
        return Ok(());
    }
    let p = host.kernel.published("corpus").unwrap();
    let raw = value(&p.snapshot.document.to_value().unwrap());
    if extensions(&raw) {
        result(work, "extension", "p2")?;
        actor.shutdown();
        return Ok(());
    }
    // Private selected metadata for resolving catalog failures; not an audit
    // result. Omit names, embedded data and arbitrary asset fields.
    let mut descriptors = raw["assets"].clone();
    if let Some(assets) = descriptors.as_object_mut() {
        for asset in assets.values_mut() {
            if let Some(fields) = asset.as_object_mut() {
                fields.retain(|key, _| {
                    [
                        "id",
                        "kind",
                        "storageKey",
                        "fingerprint",
                        "revision",
                        "sourcePath",
                        "path",
                        "link",
                        "playbackProxy",
                        "playbackProxyVersion",
                        "contentHash",
                        "sourceHash",
                        "blake3",
                    ]
                    .contains(&key.as_str())
                });
                if let Some(link) = fields.get_mut("link").and_then(Value::as_object_mut) {
                    link.retain(|key, _| key == "path");
                }
            }
        }
    }
    if !descriptors.is_object() {
        descriptors = json!(null);
    }
    let used = project_assets(&raw)
        .as_object()
        .map(|a| a.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    write(
        &work.join("asset-request.private.json"),
        &json!({"assets":descriptors,"used":used}),
    )?;
    result(work, "running", "render")?;
    let comp = p
        .snapshot
        .open_comp()
        .or_else(|| raw["id"].as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("no_composition"))?;
    let media = catalog(root, &p, work)?;
    let canonical = if p.snapshot.open_comp().is_some() {
        raw["comps"][&comp].clone()
    } else {
        raw.clone()
    };
    let dur = canonical["dur"].as_f64().unwrap_or(1.).max(1. / 30.);
    let rate = canonical["fps"].as_f64().filter(|x| *x > 0.).unwrap_or(30.);
    let frame_rate =
        pm_time::Rate::new((rate * 1000.).round() as i64, 1000).unwrap_or(pm_time::Rate::FPS_30);
    let frame_ticks = frame_rate
        .exact_frame_ticks()
        .unwrap_or(pm_protocol::common::TICKS_PER_SECOND / 30);
    let size = Size::new(320, 180);
    let service = EngineService::start_with_fonts(pm_gpu::Gpu::new("corpus-metal")?, fonts)?;
    let viewer = service.viewer(ViewerConfig {
        composition: comp.clone(),
        snapshot: p.snapshot.clone(),
        media: media.clone(),
        output: Output {
            width: size.width,
            height: size.height,
            display: 0,
            headroom: 1.,
        },
        layer: None,
        rate: frame_rate,
        end: None,
        audio: false,
    })?;
    let times = [0., dur * 0.5, dur * 0.9];
    let render = attempt(work, "render", || {
        ensure!(
            pm_export_offline(&p.snapshot, &comp, &media.export_paths(false)).is_empty(),
            "offline_originals"
        );
        for t in times {
            scrub(&viewer, ticks(t), frame_ticks)?;
        }
        if row["synthetic_pixel_proof"] == true {
            let before = scrub(&viewer, Ticks::ZERO, frame_ticks)?;
            viewer.capture_next()?;
            let f = wait(&viewer, Some(Ticks::ZERO), before.completion_host_seconds)?;
            let pixels = f
                .pixels
                .ok_or_else(|| anyhow::anyhow!("missing_synthetic_readback"))?;
            write(&work.join("preview-pixels.json"), &json!(pixels))?;
        }
        Ok(())
    })?;
    if render {
        attempt(work, "play", || {
            let f = scrub(&viewer, Ticks::ZERO, frame_ticks)?;
            viewer.command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })?;
            let started = Instant::now();
            loop {
                let n = wait(&viewer, None, f.completion_host_seconds)?;
                if n.time > Ticks::ZERO {
                    break;
                }
                ensure!(
                    started.elapsed() < Duration::from_secs(15),
                    "playhead_did_not_advance"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            viewer.command(TransportCommand::Pause)?;
            Ok(())
        })?;
        attempt(work, "scrub", || {
            let mut hits = 0;
            let mut seed = u64::from_str_radix(row["id"].as_str().unwrap(), 16)?;
            for i in 0..8 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let t = if i % 2 == 0 {
                    dur * 0.5
                } else {
                    dur * ((seed >> 32) as f64 / u32::MAX as f64)
                };
                let f = scrub(&viewer, ticks(t), frame_ticks)?;
                hits += usize::from(f.cache_hit);
            }
            ensure!(hits > 0, "no_cached_seek_observed");
            Ok(())
        })?;
    }
    // Decode and evaluate audio without opening a speaker device.
    attempt(work, "audio", || {
        let original = media.export_paths(false);
        ensure!(
            pm_export_offline(&p.snapshot, &comp, &original).is_empty(),
            "offline_originals"
        );
        let cache = pm_audio::SourceCache::new(64 * 1024 * 1024, 2);
        cache.set_blocking_lengths(true);
        let evaluator =
            || pm_eval::Evaluator::new().with_expressions(Arc::new(pm_render::expr::PmExprEngine));
        let graph = pm_audio::MixGraph::build(
            &p.snapshot.document,
            &comp,
            &mut evaluator(),
            &cache,
            &original,
        )?;
        ensure!(graph.problems.is_empty(), "audio_graph_problems");
        let mut mixer = pm_audio::Mixer::new_legacy_export(
            pm_audio::MixFormat::stereo(48000),
            p.snapshot.document.clone(),
            Arc::new(graph),
            evaluator(),
            cache,
        );
        for t in times {
            let first = (t * 48000.).round() as i64;
            mixer.set_today_range(t, t + 0.1, first);
            let mut finite = true;
            let summary =
                pm_audio::offline::render(&mut mixer, first, first + 4800, 1024, |_, samples| {
                    finite &= samples.iter().all(|x| x.is_finite());
                    Ok(())
                })?;
            ensure!(finite, "nonfinite_audio");
            ensure!(summary.errors.is_empty(), "audio_decode_error");
        }
        Ok(())
    })?;
    viewer.retire();
    drop(service);
    let mut c = app_mcp::Connection::new(host.clone());
    let edit = attempt(work, "edit", || {
        invoke(
            &mut c,
            "start_session",
            json!({"prompt":"Corpus round trip"}),
        )?;
        invoke(
            &mut c,
            "apply_commands",
            json!({"commands":[{"type":"set_composition","patch":{"name":"Synthetic corpus edit"}}]}),
        )?;
        invoke(&mut c, "finish_session", json!({"commit":true}))?;
        let after = host.kernel.published("corpus").unwrap();
        ensure!(after.revision() > p.revision(), "edit_no_revision");
        Ok(())
    })?;
    let saved = work.join("roundtrip.pmv");
    if edit {
        attempt(work, "save", || {
            let published = host.kernel.published("corpus").unwrap();
            let metadata = pm_model::json::to_canonical_string(
                &published.snapshot.document.to_value().unwrap(),
            )
            .len() as u64;
            let history = fs::metadata(profile.join("store/projectHistory.corpus.json"))
                .map(|m| m.len())
                .unwrap_or(0);
            let needed = published
                .media
                .iter()
                .map(|m| m.length())
                .sum::<u64>()
                .saturating_add(metadata.saturating_mul(3))
                .saturating_add(history.saturating_mul(2))
                .saturating_add(16 * 1024 * 1024);
            ensure!(
                fs2::available_space(work)?.saturating_sub(needed) >= FLOOR,
                "disk_floor: roundtrip reserve required"
            );
            let handle = host.current().unwrap();
            let reply = host
                .kernel
                .shell(ShellCall::SaveAs {
                    session: handle,
                    path: saved.clone(),
                })
                .wait()
                .unwrap()?;
            ensure!(
                matches!(
                    reply,
                    ShellReply::Saved {
                        recovery_copy: None,
                        ..
                    }
                ),
                "save_refused"
            );
            let reopened = pm_kernel::open_project(&saved, Default::default())?;
            let after = host.kernel.published("corpus").unwrap();
            ensure!(
                reopened.document.to_value() == after.snapshot.document.to_value(),
                "roundtrip_mismatch"
            );
            Ok(())
        })?;
    }
    let snapshot = host.kernel.published("corpus").unwrap().snapshot.clone();
    for (stage, format, ext) in [
        ("mp4", ExportFormat::H264, "mp4"),
        (
            "prores",
            ExportFormat::ProRes {
                profile: ProResProfile::Standard,
            },
            "mov",
        ),
    ] {
        attempt(work, stage, || {
            ensure!(
                fs2::available_space(root)?.saturating_sub(32 * 1024 * 1024) >= FLOOR,
                "disk_floor: export reserve required"
            );
            let dest = work.join(format!("short.{ext}"));
            let settings = ExportSettings {
                format,
                alpha: AlphaOutput::None,
                quality: QualityTier::High,
                size,
                frame_rate: FrameRate::new(30, 1),
                range: TickRange {
                    start: Ticks::ZERO,
                    end: ticks(dur.min(0.1)),
                },
                audio: true,
                audio_index_playhead: None,
                motion_blur: false,
                color: OutputColor::Rec709Broadcast,
                precision: Precision::Float16,
                dither: false,
                draft_from_proxies: false,
                delivery: Default::default(),
            };
            let request = ExportRequest::new(
                "corpus-export".into(),
                &snapshot,
                JobSpec::Export {
                    session: SessionId::new(1).unwrap(),
                    composition: comp.as_str().into(),
                    settings,
                },
                media.clone(),
                dest.clone(),
            )?;
            let status = ExportHandle::start(&std::env::current_exe()?, request)?.wait()?;
            ensure!(status.state == JobState::Done, "export: {:?}", status.error);
            let decoded = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-xerror", "-i"])
                .arg(&dest)
                .args(["-f", "null", "-"])
                .output()?;
            ensure!(decoded.status.success(), "decode_failed");
            if row["synthetic_pixel_proof"] == true {
                let pixels = std::process::Command::new("ffmpeg")
                    .args(["-v", "error", "-i"])
                    .arg(&dest)
                    .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                    .output()?;
                ensure!(pixels.status.success(), "synthetic_pixel_decode_failed");
                ensure!(
                    pixels.stdout.len() == 320 * 180 * 3,
                    "synthetic_pixel_count"
                );
                fs::write(work.join(format!("{stage}-pixels.rgb")), pixels.stdout)?;
            }
            fs::remove_file(dest)?;
            Ok(())
        })?;
    }
    c.disconnect()?;
    drop(c);
    drop(host);
    actor.shutdown();
    result(work, "running", "done")?;
    Ok(())
}
// Keep engine offline discovery in the same path as export-child validation.
fn pm_export_offline(
    snapshot: &pm_kernel::Snapshot,
    comp: &str,
    media: &pm_export::render::MediaPaths,
) -> Vec<pm_export::render::OfflineClip> {
    pm_export::render::offline_media(&snapshot.document, comp, media)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--engine-job") {
        return pm_engine_service::jobs::run_child();
    }
    match args.get(1).map(String::as_str) {
        Some("rotation-proof") => rotation::proof(Path::new(&args[2])),
        Some("shader-info") => {
            let mut effects = pm_render::effects::Effects::new("corpus-diagnostics");
            let failures = effects.set_snapshot(&pm_render::all_builtin_definitions())?;
            write(Path::new(&args[2]), &serde_json::to_value(&failures)?)?;
            ensure!(
                failures.is_empty(),
                "shader_translation_failed; inspect private shader-info"
            );
            Ok(())
        }
        Some("snapshot") => snapshot(
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
        ),
        Some("blob") => Snapshot::open(Path::new(&args[2]))?.copy_blob(
            args[3].parse()?,
            args[4].parse()?,
            Path::new(&args[5]),
            FLOOR,
        ),
        Some("file-info") => {
            let p = pm_kernel::open_project(Path::new(&args[2]), Default::default())?;
            let v = value(&p.document.to_value().unwrap());
            write(
                Path::new(&args[3]),
                &json!({"assets":project_assets(&v),"extension":extensions(&v),"embedded":p.file.media.iter().map(|m|m.id.to_string()).collect::<Vec<_>>()}),
            )
        }
        Some("inventory") => inventory(Path::new(&args[2])),
        Some("project") => {
            // Font registry lives on main; all project/kernel/GPU work runs off main.
            let mut service = pm_node::fonts::FontService::new().map_err(anyhow::Error::msg)?;
            let broker = pm_node::fonts::render::FontBroker::new();
            broker.set_wake(Arc::new(|| {}));
            broker.set_generation(service.generation());
            let fonts = pm_node::fonts::render::RenderFonts::new(broker.clone());
            let root = PathBuf::from(&args[2]);
            let spec = PathBuf::from(&args[3]);
            let work = PathBuf::from(&args[4]);
            let worker = std::thread::spawn(move || project(&root, &spec, &work, fonts));
            while !worker.is_finished() {
                pm_node::fonts::render::serve(&mut service, broker.take_asks());
                std::thread::sleep(Duration::from_millis(2));
            }
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("project_worker_panicked"))?
        }
        _ => anyhow::bail!(
            "usage: pm-corpus snapshot DB BLOBS TARGET | inventory ROOT | project ROOT SPEC WORK"
        ),
    }
}

#[cfg(test)]
mod corpus_tests {
    use super::*;
    #[test]
    fn distinguishes_builtin_effects_from_runtime_dependencies() {
        assert!(!extensions(
            &json!({"layers":[{"type":"shape","fx":[{"type":"blur"}]}]})
        ));
        assert!(extensions(
            &json!({"layers":[{"type":"shape","fx":[{"type":"synthetic-runtime-effect"}]}]})
        ));
        assert!(extensions(
            &json!({"comps":{"other":{"layers":[{"type":"extension","d":{"definition":"synthetic"}}]}}})
        ));
        assert!(!extensions(
            &json!({"layers":[{"type":"extension","d":{"definition":"powermove.3d.studio-cube"}}]})
        ));
    }
    #[test]
    fn only_materializes_assets_referenced_by_compositions() {
        let v = json!({"assets":{"used":{"id":"used"},"unused":{"id":"unused"}},"comps":{"nested":{"layers":[{"d":{"asset":"used"}}]}}});
        assert_eq!(project_assets(&v), json!({"used":{"id":"used"}}));
    }
}
