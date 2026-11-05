use pm_kernel::{Command as KernelCommand, MediaCommand, Origin};
use pm_mcp::media::{MediaRegistry, MediaState};
use pm_mcp::services::{ExportChild, FrozenProject, Image, MediaRequest, VideoExportRequest};
use pm_mcp::{Automation, CaptureRequest, EngineHook, Error, McpSession, Result};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
    id: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-wave2"));
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        let id = host
            .create_project(&json!({"name":"Synthetic wave 2","duration":20}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .into();
        Self {
            root,
            host: Arc::new(Mutex::new(host)),
            id,
        }
    }
    fn session(&self) -> McpSession {
        let mut s = McpSession::new(self.host.clone());
        ok(s.call("start_session", json!({"prompt":"Synthetic test"})));
        s
    }
    fn read(&self) -> Value {
        self.host.lock().unwrap().read_document(&self.id).unwrap()
    }
    fn commands(&self, commands: Vec<KernelCommand>) {
        let mut h = self.host.lock().unwrap();
        let r = h.revision(&self.id).unwrap();
        h.apply_commands(&self.id, commands, "Synthetic fixture", Origin::Mcp, r, 1)
            .unwrap();
    }
    fn media(&self) {
        self.commands(vec![MediaCommand::AddAsset{asset_id:"a".into(),asset:pm_model::json::parse(r#"{"id":"a","name":"Synthetic footage","kind":"video","dur":30,"w":640,"h":360,"persisted":true,"sourcePath":"/synthetic/footage.mp4","unknown":{"keep":true}}"#).unwrap()}.into()]);
    }
    fn seed_video(&self, s: &mut McpSession) {
        ok(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":"clip","layerType":"video","name":"Clip","from":2,"duration":6,"content":{"asset":"a","trim":{"v":1,"kf":[],"expr":null},"speed":{"v":1,"kf":[],"expr":null},"embeddedAudio":true}},{"type":"replace_keyframes","target":"clip","path":"opacity","keyframes":[{"time":0,"value":0},{"time":5,"value":100}]}]})));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn err(v: Value) -> Value {
    assert_eq!(v["isError"], true, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn oracle(input: Value) -> Value {
    let mut c = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/domain-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let r = c.wait_with_output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    serde_json::from_slice(&r.stdout).unwrap()
}
fn normalize_ids(before: &Value, after: &Value, result: &Value) -> (Value, Value) {
    fn ids(v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::Object(o) => {
                for (k, v) in o {
                    if ["id", "i"].contains(&k.as_str())
                        && let Some(s) = v.as_str()
                    {
                        out.insert(s.into());
                    }
                    ids(v, out)
                }
            }
            Value::Array(a) => {
                for v in a {
                    ids(v, out)
                }
            }
            _ => {}
        }
    }
    fn map_ids(v: &Value, known: &HashSet<String>, map: &mut HashMap<String, String>) {
        match v {
            Value::Object(o) => {
                for (k, v) in o {
                    if ["id", "i"].contains(&k.as_str())
                        && let Some(s) = v.as_str()
                        && !known.contains(s)
                    {
                        let next = format!("generated-{}", map.len());
                        map.entry(s.into()).or_insert(next);
                    }
                    map_ids(v, known, map)
                }
            }
            Value::Array(a) => {
                for v in a {
                    map_ids(v, known, map)
                }
            }
            _ => {}
        }
    }
    fn replace(v: &Value, map: &HashMap<String, String>) -> Value {
        match v {
            Value::String(s) => json!(map.get(s).unwrap_or(s)),
            Value::Array(a) => json!(a.iter().map(|v| replace(v, map)).collect::<Vec<_>>()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, v)| (k.clone(), replace(v, map)))
                    .collect(),
            ),
            _ => v.clone(),
        }
    }
    let mut known = HashSet::new();
    ids(before, &mut known);
    let mut map = HashMap::new();
    map_ids(after, &known, &mut map);
    (replace(after, &map), replace(result, &map))
}
#[test]
fn video_every_operation_matches_tagged_body_and_rolls_back() {
    let operations = [
        json!({"operation":"insert","assetId":"a","from":1,"duration":4,"sourceIn":2}),
        json!({"operation":"split","layerId":"clip","at":5}),
        json!({"operation":"trim","layerId":"clip","start":1,"end":9}),
        json!({"operation":"move","layerId":"clip","from":10}),
        json!({"operation":"slip","layerId":"clip","sourceIn":4}),
        json!({"operation":"separate_audio","layerId":"clip"}),
        json!({"operation":"remove","layerId":"clip"}),
    ];
    for args in operations {
        let f = Fixture::new();
        f.media();
        let mut seed = f.session();
        f.seed_video(&mut seed);
        ok(seed.call("finish_session", json!({"commit":true})));
        let before = f.read();
        let expected = oracle(json!({"tool":"edit_video","project":before,"arguments":args}));
        let mut s = f.session();
        let result = ok(s.call("edit_video", args.clone()));
        let after = f.read();
        let mut r = result.clone();
        r.as_object_mut().unwrap().remove("revision");
        assert_eq!(
            normalize_ids(&before, &after["layers"], &r),
            normalize_ids(&before, &expected["project"]["layers"], &expected["result"]),
            "{args}"
        );
        ok(s.call("rollback_changes", json!({})));
        assert_eq!(f.read()["layers"], before["layers"]);
    }
}
#[test]
fn speed_and_audio_controls_match_tag() {
    for (kind, args) in [
        (
            "video",
            json!({"operation":"speed","layerId":"clip","speed":2}),
        ),
        (
            "audio",
            json!({"operation":"audio","layerId":"clip","gain":0.5,"fadeIn":1,"fadeOut":2}),
        ),
    ] {
        let f = Fixture::new();
        f.media();
        let mut s = f.session();
        ok(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":"clip","layerType":kind,"from":2,"duration":6,"content":if kind=="video"{json!({"asset":"a","embeddedAudio":false,"trim":1,"speed":1})}else{json!({"asset":"a","trim":1})}}]})));
        let p = f.read();
        let expected = oracle(json!({"tool":"edit_video","project":p,"arguments":args}));
        ok(s.call("edit_video", args));
        assert_eq!(f.read()["layers"], expected["project"]["layers"]);
    }
}
#[test]
fn video_rejects_unsafe_edits_without_partial_work() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    f.seed_video(&mut s);
    for args in [
        json!({"operation":"split","layerId":"clip","at":2}),
        json!({"operation":"trim","layerId":"clip","start":0,"end":8}),
        json!({"operation":"speed","layerId":"clip","speed":2}),
        json!({"operation":"slip","layerId":"clip","sourceIn":29}),
        json!({"operation":"audio","layerId":"clip","gain":1}),
        json!({"operation":"insert","assetId":"a","from":19,"duration":2}),
    ] {
        let before = f.read();
        err(s.call("edit_video", args));
        assert_eq!(f.read(), before);
    }
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"set_layer","target":"clip","patch":{"locked":true}}]}),
    ));
    let before = f.read();
    err(s.call(
        "edit_video",
        json!({"operation":"move","layerId":"clip","from":3}),
    ));
    assert_eq!(f.read(), before);
}
#[test]
fn video_revision_and_document_guards_preserve_foreign_work() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    f.seed_video(&mut s);
    let mut other = f.session();
    ok(other.call(
        "edit_video",
        json!({"operation":"move","layerId":"clip","from":3}),
    ));
    let before = f.read();
    assert_eq!(
        err(s.call(
            "edit_video",
            json!({"operation":"move","layerId":"clip","from":4})
        ))["code"],
        "revision_conflict"
    );
    assert_eq!(f.read(), before);
    ok(other.call("finish_session", json!({"commit":true})));
}
struct Registry;
impl MediaRegistry for Registry {
    fn state(&self, _: &str, id: &str) -> MediaState {
        match id {
            "a" => MediaState {
                available: true,
                duration: Some(30.),
                has_audio: true,
                ..Default::default()
            },
            "b" => MediaState {
                error: Some("Synthetic decode error".into()),
                ..Default::default()
            },
            "c" => MediaState {
                cloud: true,
                ..Default::default()
            },
            "d" => MediaState {
                loading: true,
                ..Default::default()
            },
            _ => MediaState::default(),
        }
    }
}
#[test]
fn list_media_paging_status_references_and_folders_match_tag() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    f.seed_video(&mut s);
    f.host.lock().unwrap().media_registry = Some(Arc::new(Registry));
    for args in [
        json!({}),
        json!({"missingOnly":true}),
        json!({"offset":1,"limit":1}),
        json!({"assetId":"a"}),
        json!({"offset":9007199254740991u64}),
    ] {
        let p = f.read();
        let expected = oracle(
            json!({"tool":"list_media","project":p,"arguments":args,"runtime":{"a":{"available":true,"duration":30,"hasAudio":true}}}),
        );
        assert_eq!(ok(s.call("list_media", args)), expected["result"]);
    }
    // Metadata persistence is never confused with a loaded/decoded runtime.
    f.host.lock().unwrap().media_registry = None;
    assert_eq!(
        ok(s.call("list_media", json!({})))["assets"][0]["status"],
        "missing"
    );
}
#[test]
fn manage_media_folders_and_native_undo_match_tag() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    let created = ok(s.call(
        "manage_media",
        json!({"operation":"create_folder","name":"Shots"}),
    ));
    let folder = created["folderId"].as_str().unwrap();
    for args in [
        json!({"operation":"rename","assetIds":["a"],"name":" Camera "}),
        json!({"operation":"move","assetIds":["a"],"folderId":folder}),
        json!({"operation":"rename_folder","folderId":folder,"name":"Scene"}),
        json!({"operation":"move_folder","folderId":folder,"parentId":null}),
        json!({"operation":"delete_folder","folderId":folder}),
    ] {
        let p = f.read();
        let expected = oracle(json!({"tool":"manage_media","project":p,"arguments":args}));
        let result = ok(s.call("manage_media", args));
        assert_eq!(result, expected["result"]);
        let after = f.read();
        assert_eq!(after["assets"], expected["project"]["assets"]);
        assert_eq!(after["assetFolders"], expected["project"]["assetFolders"]);
    }
    assert_eq!(
        err(s.call("rollback_changes", json!({})))["code"],
        "revision_conflict"
    );
    ok(s.call("finish_session", json!({"commit":true})));
    let mut host = f.host.lock().unwrap();
    host.undo(&f.id, Origin::Mcp, 5).unwrap();
    assert!(host.read_document(&f.id).unwrap()["assetFolders"][folder].is_object());
}
#[test]
fn manage_delete_preserves_bytes_and_refuses_locked_or_structured_references() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    f.seed_video(&mut s);
    let before = f.read();
    err(s.call(
        "manage_media",
        json!({"operation":"delete","assetIds":["a"]}),
    ));
    assert_eq!(f.read(), before);
    let args = json!({"operation":"delete","assetIds":["a"],"removeReferenced":true});
    let expected = oracle(json!({"tool":"manage_media","project":before,"arguments":args}));
    assert_eq!(ok(s.call("manage_media", args)), expected["result"]);
    assert!(f.read()["layers"].as_array().unwrap().is_empty());
    // Metadata removal has no filesystem deletion path; kernel Undo retains it.
    ok(s.call("finish_session", json!({"commit":true})));
    f.host.lock().unwrap().undo(&f.id, Origin::Mcp, 5).unwrap();
    assert_eq!(f.read()["assets"], before["assets"]);
    assert_eq!(f.read()["layers"], before["layers"]);
}
fn engine_oracle(input: Value) -> Value {
    let mut c = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/engine-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let r = c.wait_with_output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    serde_json::from_slice(&r.stdout).unwrap()
}
fn image(mime: &str) -> Image {
    let images: Value =
        serde_json::from_str(include_str!("../fixtures/service-images.json")).unwrap();
    Image {
        data: images[if mime == "image/png" { "png" } else { "jpeg" }]
            .as_str()
            .unwrap()
            .into(),
        mime_type: mime.into(),
    }
}
#[derive(Default)]
struct HeadlessDouble {
    captures: Mutex<Vec<CaptureRequest>>,
    calls: Mutex<Vec<MediaRequest>>,
}
impl EngineHook for HeadlessDouble {
    fn capture(&self, _: &CaptureRequest) -> Result<Value> {
        Err(Error::invalid("Frozen capture required"))
    }
    fn capture_snapshot(&self, r: &CaptureRequest, p: &FrozenProject) -> Result<Value> {
        assert_eq!(r.project_id, p.project_id);
        assert_eq!(r.revision, p.revision);
        assert_eq!(p.project["revision"], p.revision);
        self.captures.lock().unwrap().push(r.clone());
        Ok(
            json!({"times":r.times,"images":r.times.iter().map(|_|format!("data:image/png;base64,{PNG}")).collect::<Vec<_>>()}),
        )
    }
    fn resolve_media(&self, r: &MediaRequest) -> Result<pm_mcp::media_tools::MediaSource> {
        self.calls.lock().unwrap().push(r.clone());
        assert_eq!(r.asset.as_ref().unwrap()["id"], "a");
        Ok(pm_mcp::media_tools::MediaSource {
            asset: json!({"id":"a","name":"Synthetic footage","kind":"video"}),
            duration: Some(30.),
            proxy: false,
            layer: None,
        })
    }
    fn probe(
        &self,
        _: &MediaRequest,
        _: &pm_mcp::media_tools::MediaSource,
    ) -> Result<pm_mcp::media_tools::Probe> {
        Ok(pm_mcp::media_tools::Probe {
            format: Some("mov/mp4".into()),
            duration: Some(30.),
            start: 0.,
            bitrate_kbps: Some(114.),
            video: vec![
                json!({"index":0,"codec":"h264","pixelFormat":"yuv420p","width":640,"height":360,"fps":30,"alpha":false}),
            ],
            audio: vec![json!({"index":1,"codec":"aac","sampleRate":48000,"channels":2})],
            other_streams: 0,
        })
    }
    fn sample(&self, r: &pm_mcp::media_tools::FrameRequest) -> Result<Vec<Image>> {
        assert!(r.width > 0 && r.height > 0);
        assert!(r.frames.len() <= 8);
        Ok(r.frames.iter().map(|_| image("image/jpeg")).collect())
    }
    fn scan(&self, r: &pm_mcp::media_tools::ScanRequest) -> Result<pm_mcp::media_tools::Scan> {
        Ok(pm_mcp::media_tools::Scan {
            picks: vec![pm_mcp::media_tools::ScanPick {
                time: r.start,
                since: None,
            }],
            distinct_states: 1,
        })
    }
    fn sheet(&self, r: &pm_mcp::media_tools::SheetRequest) -> Result<Image> {
        assert!(r.plan.columns * r.plan.rows >= r.cells.len());
        assert!(r.cells.len() <= 48);
        Ok(image("image/jpeg"))
    }
    fn waveform(
        &self,
        r: &pm_mcp::media_tools::WaveformRequest,
    ) -> Result<pm_mcp::media_tools::Waveform> {
        Ok(pm_mcp::media_tools::Waveform {
            loudness: json!({"clipping":false}),
            silences: if r.start + 2. < r.end {
                vec![[r.start + 2., (r.start + 3.5).min(r.end)]]
            } else {
                vec![]
            },
            image: if r.image {
                Some(image("image/png"))
            } else {
                None
            },
        })
    }
}
#[test]
fn render_times_defaults_dedup_clamp_selection_and_images_match_tag() {
    let f = Fixture::new();
    let engine = Arc::new(HeadlessDouble::default());
    f.host.lock().unwrap().engine = Some(engine.clone());
    let mut s = f.session();
    ok(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":"selected","layerType":"solid","from":2,"duration":4}]})));
    ok(s.call("select_layers", json!({"layerIds":["selected"]})));
    let before = f.read();
    for args in [
        json!({}),
        json!({"times":[]}),
        json!({"times":[0,20,20,100,1.23456],"width":160}),
    ] {
        let expected = oracle(
            json!({"tool":"render_frames","project":before,"arguments":args,"selection":["selected"],"image":PNG}),
        );
        let raw = s.call("render_frames", args);
        assert_eq!(ok(raw.clone()), expected["result"]);
        assert_eq!(
            raw["content"].as_array().unwrap().len(),
            expected["result"]["rendered"].as_u64().unwrap() as usize + 1
        );
        assert_eq!(raw["content"][1]["data"], PNG);
        assert_eq!(raw["content"][1]["mimeType"], "image/png");
        assert_eq!(f.read(), before);
    }
    assert_eq!(engine.captures.lock().unwrap().len(), 3);
}
#[test]
fn media_engine_boundaries_match_all_four_tagged_tool_bodies() {
    let f = Fixture::new();
    f.media();
    let engine = Arc::new(HeadlessDouble::default());
    f.host.lock().unwrap().engine = Some(engine.clone());
    let mut s = f.session();
    f.seed_video(&mut s);
    let before = f.read();
    let layer = json!({"id":"clip","name":"Clip","type":"video","from":2,"duration":6,"timing":{"constant":true,"samples":[[2,1],[8,7]]},"sourceStart":1,"sourceEnd":7});
    for (tool, args) in [
        ("probe_media", json!({"assetId":"a"})),
        ("probe_media", json!({"layerId":"clip"})),
        ("sample_media_frames", json!({"assetId":"a","times":[1,-1]})),
        (
            "sample_media_frames",
            json!({"layerId":"clip","times":[3,7],"quality":"medium"}),
        ),
        ("sample_media_frames", json!({"assetId":"a","auto":true})),
        ("media_contact_sheet", json!({"assetId":"a","count":12})),
        (
            "media_contact_sheet",
            json!({"layerId":"clip","times":[2,4,7],"columns":2}),
        ),
        (
            "media_contact_sheet",
            json!({"target":"composition","count":4}),
        ),
        ("media_waveform", json!({"assetId":"a"})),
        ("media_waveform", json!({"layerId":"clip","image":false})),
        (
            "media_waveform",
            json!({"assetId":"a","start":1,"end":10,"width":400,"silenceThresholdDb":-45,"minSilence":0.3}),
        ),
    ] {
        let expected = engine_oracle(
            json!({"tool":tool,"arguments":args,"layer":if args["layerId"].is_string(){layer.clone()}else{Value::Null}}),
        );
        let raw = s.call(tool, args.clone());
        assert_eq!(ok(raw.clone()), expected["value"], "{tool} {args}");
        let images = expected["images"].as_array().unwrap();
        assert_eq!(raw["content"].as_array().unwrap().len(), images.len() + 1);
        for (i, image) in images.iter().enumerate() {
            assert_eq!(raw["content"][i + 1]["data"], image["data"]);
            assert_eq!(raw["content"][i + 1]["mimeType"], image["mimeType"]);
        }
        assert_eq!(f.read(), before);
    }
    for r in engine.calls.lock().unwrap().iter() {
        assert_eq!(r.snapshot.project_id, f.id);
        assert_eq!(r.snapshot.revision, before["revision"]);
        assert_eq!(r.snapshot.project, before);
    }
}
#[test]
fn host_absence_and_invalid_targets_are_precise_and_do_not_refresh_edit_guard() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    f.seed_video(&mut s);
    for (tool, args) in [
        ("render_frames", json!({})),
        ("probe_media", json!({"assetId":"a"})),
        ("sample_media_frames", json!({"assetId":"a"})),
        ("media_contact_sheet", json!({"target":"composition"})),
        ("media_waveform", json!({"assetId":"a"})),
        ("export_video", json!({})),
        ("get_workspace_state", json!({})),
        ("capture_panel", json!({"panelId":"viewer"})),
    ] {
        let e = err(s.call(tool, args));
        assert_eq!(e["code"], "not_available_in_this_build", "{tool}: {e}");
        assert!(!e["backing"].as_str().unwrap().is_empty());
    }
    f.host.lock().unwrap().engine = Some(Arc::new(HeadlessDouble::default()));
    for args in [
        json!({}),
        json!({"assetId":"bad"}),
        json!({"layerId":"bad"}),
        json!({"assetId":"bad","layerId":"clip"}),
    ] {
        assert_eq!(
            err(s.call("probe_media", args))["code"],
            "invalid_arguments"
        );
    }
    let mut other = f.session();
    ok(other.call(
        "edit_video",
        json!({"operation":"move","layerId":"clip","from":3}),
    ));
    ok(s.call("render_frames", json!({"times":[0]})));
    assert_eq!(
        err(s.call(
            "edit_video",
            json!({"operation":"move","layerId":"clip","from":4})
        ))["code"],
        "revision_conflict"
    );
    ok(other.call("finish_session", json!({"commit":true})));
}
struct ChildDouble {
    requests: Mutex<Vec<VideoExportRequest>>,
    fail: bool,
}
impl ExportChild for ChildDouble {
    fn video(&self, r: &VideoExportRequest) -> Result<PathBuf> {
        self.requests.lock().unwrap().push(r.clone());
        if self.fail {
            return Err(Error::unavailable(
                "Synthetic export cancelled; child reaped",
            ));
        }
        std::fs::create_dir_all(&r.directory)?;
        let path = r.directory.join(if r.options.format == "prores" {
            "synthetic.mov"
        } else {
            "synthetic.mp4"
        });
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(b"synthetic encoded output; no pixel correctness claim")?;
        Ok(path)
    }
}
#[test]
fn export_child_defaults_formats_no_clobber_errors_and_caption_text() {
    let f = Fixture::new();
    let child = Arc::new(ChildDouble {
        requests: Mutex::new(vec![]),
        fail: false,
    });
    f.host.lock().unwrap().export_child = child.clone();
    let mut s = f.session();
    let before = f.read();
    let result = ok(s.call("export_video", json!({})));
    assert_eq!(result["projectId"], f.id);
    assert_eq!(result["fps"], before["fps"]);
    let calls = child.requests.lock().unwrap();
    let r = &calls[0];
    assert_eq!(r.snapshot.project, before);
    assert_eq!(r.options.format, "mp4");
    assert_eq!(r.options.range, "all");
    assert!(r.options.audio && r.options.motion_blur && !r.options.alpha);
    drop(calls);
    assert_eq!(err(s.call("export_video", json!({})))["code"], "io_error");
    assert_eq!(
        err(s.call("export_video", json!({"alpha":true})))["code"],
        "invalid_arguments"
    );
    ok(s.call("export_video",json!({"format":"prores","range":"work","alpha":true,"audio":false,"motionBlur":false,"name":"Synthetic"})));
    assert_eq!(f.read(), before);
    f.host.lock().unwrap().export_child = Arc::new(ChildDouble {
        requests: Mutex::new(vec![]),
        fail: true,
    });
    err(s.call("export_video", json!({})));
    assert_eq!(f.read(), before);
}
#[test]
fn caption_export_normalization_clipping_language_escaping_matches_tag() {
    for format in ["srt", "vtt"] {
        let f = Fixture::new();
        let mut p = f.read();
        p["layers"] = json!([{"id":"captions","type":"captions","name":"Synthetic captions","from":2,"dur":4,"d":{"language":"en-US","cues":[{"id":"a","start":0,"end":2,"text":" Hello  & <world>\r\n\nagain "},{"id":"b","start":1,"end":3,"text":"Second"},{"id":"c","start":1,"end":4,"text":"Same speaker time"},{"start":8,"end":9,"text":"outside"},{"start":3.95,"end":5,"text":"clipped"},{"start":1,"end":1.001,"text":"too short"}]}}]);
        let revision = p["revision"].as_u64().unwrap();
        f.host
            .lock()
            .unwrap()
            .restore(&f.id, p, revision, 1)
            .unwrap();
        let before = f.read();
        let args = json!({"layerId":"captions","format":format});
        let expected = oracle(json!({"tool":"export_captions","project":before,"arguments":args}));
        let mut s = f.session();
        assert_eq!(ok(s.call("export_captions", args)), expected["result"]);
        assert_eq!(f.read(), before);
        err(s.call("export_captions", json!({"layerId":"missing"})));
    }
}
#[test]
fn scene_digest_matches_owner_selected_3d_source_fixtures() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../fixtures/scene3d-source-read.json")).unwrap();
    for fixture in fixtures["fixtures"].as_array().unwrap() {
        let f = Fixture::new();
        let mut p = f.read();
        for k in ["w", "h", "fps", "dur", "layers", "assets"] {
            p[k] = fixture["project"][k].clone();
        }
        let revision = p["revision"].as_u64().unwrap();
        f.host
            .lock()
            .unwrap()
            .restore(&f.id, p, revision, 1)
            .unwrap();
        let before = f.read();
        let mut expected = fixture["result"].clone();
        expected["revision"] = before["revision"].clone();
        let mut s = f.session();
        let actual = ok(s.call("get_3d_scene", fixture["arguments"].clone()));
        assert_json(&actual, &expected, "scene digest");
        assert_eq!(f.read(), before);
    }
}
#[test]
fn structural_check_uses_evaluated_visibility_and_matches_tag() {
    let f = Fixture::new();
    let mut s = f.session();
    for commands in [
        vec![],
        vec![json!({"type":"add_layer","id":"solid","layerType":"solid","from":2,"duration":4})],
        vec![
            json!({"type":"set_property","target":"solid","path":"opacity","value":0,"mode":"static"}),
        ],
        vec![json!({"type":"set_layer","target":"solid","patch":{"visible":false}})],
        vec![json!({"type":"add_layer","id":"unbound","layerType":"video","from":2,"duration":4})],
    ] {
        if !commands.is_empty() {
            ok(s.call("apply_commands", json!({"commands":commands})));
        }
        let before = f.read();
        let expected = oracle(json!({"tool":"check_project","project":before}));
        assert_eq!(ok(s.call("check_project", json!({}))), expected["result"]);
        assert_eq!(f.read(), before);
    }
}

fn assert_json(a: &Value, b: &Value, path: &str) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for k in a.keys().chain(b.keys()) {
                assert_json(
                    a.get(k).unwrap_or(&Value::Null),
                    b.get(k).unwrap_or(&Value::Null),
                    &format!("{path}.{k}"),
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path} length");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                assert_json(a, b, &format!("{path}[{i}]"));
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}
#[test]
fn exporter_adapter_matches_frozen_tagged_options_and_results() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../fixtures/export-v1.2.0.json")).unwrap();
    for fixture in fixtures["fixtures"].as_array().unwrap() {
        let f = Fixture::new();
        let child = Arc::new(ChildDouble {
            requests: Mutex::new(vec![]),
            fail: false,
        });
        f.host.lock().unwrap().export_child = child.clone();
        let mut s = f.session();
        let raw = s.call("export_video", fixture["arguments"].clone());
        if fixture.get("error").is_some() {
            assert_eq!(err(raw)["message"], fixture["error"]);
            assert!(child.requests.lock().unwrap().is_empty());
            continue;
        }
        let mut expected = fixture["result"].clone();
        expected["projectId"] = json!(f.id);
        expected["path"] = json!(
            expected["path"]
                .as_str()
                .unwrap()
                .replace("$exports", f.root.join("exports").to_str().unwrap())
        );
        assert_eq!(ok(raw), expected);
        let requests = child.requests.lock().unwrap();
        let options = &requests[0].options;
        let legacy = &fixture["options"];
        assert_eq!(options.format, legacy["format"]);
        assert_eq!(options.range, legacy["range"]);
        assert_eq!(options.audio, legacy["audio"]);
        assert_eq!(options.alpha, legacy["alpha"]);
        assert_eq!(options.motion_blur, legacy["mblur"]);
        assert_eq!(options.name.as_deref(), legacy["name"].as_str());
    }
}
#[test]
fn nested_media_references_status_and_folder_errors_match_tag() {
    let f = Fixture::new();
    f.media();
    let mut p = f.read();
    p["assetFolders"] = json!({"root":{"name":"Root","parent":null},"child":{"name":"Child","parent":"root"},"broken":{"name":" ","parent":"missing"}});
    p["assets"]["a"]["folder"] = json!("root");
    for (id, kind) in [
        ("b", "audio"),
        ("c", "image"),
        ("d", "video"),
        ("e", "video"),
    ] {
        p["assets"][id] = json!({"id":id,"name":id,"kind":kind,"persisted":true});
    }
    p["layers"] = json!([{"id":"group","type":"group","lock":true,"from":0,"dur":20},{"id":"nested","type":"extension","group":"group","d":{"data":{"material":{"maps":{"color":"a"}}}},"from":0,"dur":20}]);
    let r = p["revision"].as_u64().unwrap();
    f.host.lock().unwrap().restore(&f.id, p, r, 1).unwrap();
    f.host.lock().unwrap().media_registry = Some(Arc::new(Registry));
    let mut s = f.session();
    let before = f.read();
    let runtime = json!({"a":{"available":true,"duration":30,"hasAudio":true},"b":{"error":"Synthetic decode error"},"c":{"cloud":true},"d":{"loading":true}});
    for args in [
        json!({}),
        json!({"offset":1,"limit":2}),
        json!({"missingOnly":true}),
    ] {
        let expected = oracle(
            json!({"tool":"list_media","project":before,"arguments":args,"runtime":runtime}),
        );
        assert_eq!(ok(s.call("list_media", args)), expected["result"]);
    }
    for args in [
        json!({"operation":"move_folder","folderId":"root","parentId":"child"}),
        json!({"operation":"delete","assetIds":["a"],"removeReferenced":true}),
        json!({"operation":"move","assetIds":["a"],"folderId":"bad"}),
        json!({"operation":"rename","assetIds":["a","b"],"name":"Test"}),
    ] {
        err(s.call("manage_media", args));
        assert_eq!(f.read(), before);
    }
    // An unlocked structured reference also refuses deletion, rather than
    // claiming that removing direct media layers removed every consumer.
    let mut p = before.clone();
    p["layers"][0]["lock"] = json!(false);
    let r = p["revision"].as_u64().unwrap();
    f.host.lock().unwrap().restore(&f.id, p, r, 1).unwrap();
    ok(s.call("list_media", json!({})));
    let before = f.read();
    err(s.call(
        "manage_media",
        json!({"operation":"delete","assetIds":["a"],"removeReferenced":true}),
    ));
    assert_eq!(f.read(), before);
}
#[test]
fn source_timing_and_check_media_limits_use_kernel_evaluation() {
    let f = Fixture::new();
    f.media();
    let mut s = f.session();
    f.seed_video(&mut s);
    ok(s.call("apply_commands",json!({"commands":[{"type":"set_content","target":"clip","patch":{"embeddedAudio":false}},{"type":"replace_keyframes","target":"clip","path":"c.speed","keyframes":[{"time":0,"value":1},{"time":6,"value":2}]}]})));
    let p = f.read();
    let timing = pm_mcp::inspection::Evaluation::new(&p)
        .timing(0, Some(30.))
        .unwrap();
    assert!(!timing.constant);
    assert_eq!(timing.samples.len(), 121);
    assert!(
        (timing.samples.last().unwrap()[1] - 10.).abs() < 1e-4,
        "timing={:?}; content={}",
        timing.samples.last(),
        p["layers"][0]["d"]
    );
    let before = f.read();
    err(s.call(
        "edit_video",
        json!({"operation":"split","layerId":"clip","at":4}),
    ));
    assert_eq!(f.read(), before);
    f.host.lock().unwrap().media_registry = Some(Arc::new(Registry));
    let args = json!({"tool":"check_project","project":before,"runtime":{"a":{"available":true,"duration":30,"hasAudio":true}}});
    assert_eq!(
        ok(s.call("check_project", json!({}))),
        oracle(args)["result"]
    );
}
#[test]
fn video_masks_effects_transitions_rebase_and_unknown_data_survive() {
    let f = Fixture::new();
    f.media();
    let mut seed = f.session();
    f.seed_video(&mut seed);
    ok(seed.call("finish_session", json!({"commit":true})));
    let mut p = f.read();
    let l = &mut p["layers"][0];
    let property = json!({"v":1,"kf":[{"i":"source-key","t":4,"v":1,"hold":true}],"expr":null});
    l["masks"] = json!([{"id":"mask","shape":"rect","mode":"add","on":true,"p":{"x":property}}]);
    l["transitionOut"] = json!({"type":"fixture-transition","p":{"amount":property}});
    l["fx"] = json!([{"id":"fixture-effect","type":"fixture","on":true,"p":{"amount":property}}]);
    l["unknown"] = json!({"preserve":[1,2,3]});
    let r = p["revision"].as_u64().unwrap();
    f.host.lock().unwrap().restore(&f.id, p, r, 1).unwrap();
    let before = f.read();
    let args = json!({"operation":"split","layerId":"clip","at":4});
    let expected = oracle(json!({"tool":"edit_video","project":before,"arguments":args}));
    let mut s = f.session();
    let mut result = ok(s.call("edit_video", args));
    result.as_object_mut().unwrap().remove("revision");
    assert_eq!(
        normalize_ids(&before, &f.read()["layers"], &result),
        normalize_ids(&before, &expected["project"]["layers"], &expected["result"])
    );
}
#[test]
fn empty_noop_video_and_media_do_not_dirty_or_falsely_report_change() {
    let f = Fixture::new();
    f.media();
    let mut seed = f.session();
    f.seed_video(&mut seed);
    ok(seed.call("finish_session", json!({"commit":true})));
    let mut s = f.session();
    let before = f.read();
    let result = ok(s.call(
        "edit_video",
        json!({"operation":"move","layerId":"clip","from":2}),
    ));
    assert_eq!(result["message"], "No source changes");
    assert_eq!(f.read(), before);
    let renamed = ok(s.call(
        "manage_media",
        json!({"operation":"rename","assetIds":["a"],"name":"Synthetic footage"}),
    ));
    assert_eq!(renamed["changed"], false);
    assert_eq!(f.read(), before);
    assert_eq!(
        ok(s.call("finish_session", json!({"commit":true})))["changed"],
        false
    );
}
struct PartialSheet;
impl EngineHook for PartialSheet {
    fn capture(&self, _: &CaptureRequest) -> Result<Value> {
        unreachable!()
    }
    fn composition_sheet(
        &self,
        r: &pm_mcp::media_tools::SheetRequest,
    ) -> Result<pm_mcp::media_tools::SheetOutput> {
        assert!(r.cells.len() > 2);
        Ok(pm_mcp::media_tools::SheetOutput {
            image: image("image/jpeg"),
            completed: 2,
        })
    }
}
#[test]
fn incomplete_composition_sheets_and_invalid_image_blocks_are_truthful() {
    let f = Fixture::new();
    f.host.lock().unwrap().engine = Some(Arc::new(PartialSheet));
    let mut s = f.session();
    let args = json!({"target":"composition","count":4});
    let expected =
        engine_oracle(json!({"tool":"media_contact_sheet","arguments":args,"completed":2}));
    let before = f.read();
    assert_eq!(ok(s.call("media_contact_sheet", args)), expected["value"]);
    assert_eq!(f.read(), before);
    for data in ["", "a", "a===", "AA=A", "AAA!", "AA A"] {
        assert!(
            pm_mcp::services::ToolOutput {
                value: json!({}),
                images: vec![Image {
                    data: data.into(),
                    mime_type: "image/png".into()
                }]
            }
            .content()
            .is_err()
        );
    }
}
#[test]
fn finish_and_disconnect_archive_authored_run_files_without_overwriting() {
    for commit in [false, true] {
        let f = Fixture::new();
        let mut s = McpSession::new(f.host.clone());
        let start = ok(s.call("start_session", json!({"prompt":"Synthetic archive"})));
        let root = PathBuf::from(start["workspace"].as_str().unwrap());
        std::fs::write(
            root.join("extensions/synthetic.ts"),
            "export const synthetic = true;",
        )
        .unwrap();
        std::fs::write(
            root.join("artifacts/synthetic.txt"),
            "Authored synthetic artifact",
        )
        .unwrap();
        let archive = f.root.join("mcp-archives").join(root.file_name().unwrap());
        ok(s.call("finish_session", json!({"commit":commit})));
        if commit {
            assert_eq!(
                std::fs::read_to_string(archive.join("extensions/synthetic.ts")).unwrap(),
                "export const synthetic = true;"
            );
            assert!(root.join("artifacts/synthetic.txt").exists());
        } else {
            assert_eq!(
                std::fs::read_to_string(archive.join("workspace/artifacts/synthetic.txt")).unwrap(),
                "Authored synthetic artifact"
            );
            assert!(!root.exists());
        }
    }
    let f = Fixture::new();
    let mut s = McpSession::new(f.host.clone());
    let start = ok(s.call("start_session", json!({"prompt":"Synthetic disconnect"})));
    let root = PathBuf::from(start["workspace"].as_str().unwrap());
    std::fs::write(
        root.join("artifacts/retained.txt"),
        "Synthetic retained work",
    )
    .unwrap();
    let archive = f.root.join("mcp-archives").join(root.file_name().unwrap());
    drop(s);
    assert_eq!(
        std::fs::read_to_string(archive.join("workspace/artifacts/retained.txt")).unwrap(),
        "Synthetic retained work"
    );
}

#[test]
fn archive_collision_preserves_existing_archive_and_retiring_workspace() {
    let f = Fixture::new();
    let mut s = McpSession::new(f.host.clone());
    let start = ok(s.call("start_session", json!({"prompt":"Synthetic collision"})));
    let root = PathBuf::from(start["workspace"].as_str().unwrap());
    std::fs::write(
        root.join("artifacts/retained.txt"),
        "Synthetic current work",
    )
    .unwrap();
    let archive = f.root.join("mcp-archives").join(root.file_name().unwrap());
    std::fs::create_dir_all(&archive).unwrap();
    std::fs::write(archive.join("previous.txt"), "Synthetic previous work").unwrap();
    assert_eq!(
        err(s.call("finish_session", json!({"commit":false})))["code"],
        "io_error"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("artifacts/retained.txt")).unwrap(),
        "Synthetic current work"
    );
    assert_eq!(
        std::fs::read_to_string(archive.join("previous.txt")).unwrap(),
        "Synthetic previous work"
    );
}
