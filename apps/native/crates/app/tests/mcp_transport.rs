//! Real app executable, disposable profile and a read-only corpus source.
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: mpsc::Receiver<String>,
    next: u64,
}
impl Client {
    fn new(profile: &std::path::Path, project: Option<&std::path::Path>) -> Self {
        Self::with_env(profile, project, &[])
    }
    fn with_env(
        profile: &std::path::Path,
        project: Option<&std::path::Path>,
        env: &[(&str, &std::path::Path)],
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_powermove-native"));
        command.args(["--powermove-mcp", "--profile"]).arg(profile);
        command.envs(env.iter().copied());
        if let Some(project) = project {
            command.arg(project);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input: Some(input),
            output: rx,
            next: 0,
        }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        writeln!(
            self.input.as_mut().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params})
        )
        .unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        let line = self
            .output
            .recv_timeout(Duration::from_secs(30))
            .expect("MCP response timed out");
        let result: Value = serde_json::from_str(&line).expect("stdout must contain JSON-RPC only");
        assert_eq!(result["id"], self.next);
        assert!(result.get("error").is_none(), "{result}");
        result["result"].clone()
    }
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name":name,"arguments":args}));
        assert_eq!(r["isError"], false, "{r}");
        serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
    }
    /// Ends the session as a client does: its input closes, the server exits.
    fn finish(mut self) {
        self.input.take();
        assert!(self.child.wait().unwrap().success());
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn app_headless_clean_reopen_handles_stale_connection_and_torn_lock() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir_all(profile.join("store")).unwrap();
    std::fs::write(profile.join("store/library.lock"), b"{\"pid\":").unwrap();
    let mut first = Client::new(&profile, None);
    first.rpc("initialize", json!({}));
    let id = first.tool("create_project", json!({"name":"Synthetic clean reopen"}))["projectId"].clone();
    first.tool("start_session", json!({"prompt":"Synthetic reopen test"}));
    first.tool("apply_commands", json!({"commands":[{"type":"add_layer","id":"synthetic","layerType":"solid","name":"Synthetic"}]}));
    first.tool("save_project", json!({}));
    first.tool("finish_session", json!({"commit":true}));
    drop(first.input.take());
    assert!(first.child.wait().unwrap().success());
    // A descriptor from a killed/older app must not force the next launch to proxy.
    let descriptor = profile.join("mcp/connection.json");
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&descriptor).unwrap();
    write!(file, "{}", json!({"version":1,"instanceId":"synthetic-stale","pid":i32::MAX,"port":1,"token":"0".repeat(64)})).unwrap();
    drop(file);
    let mut second = Client::new(&profile, None);
    second.rpc("initialize", json!({}));
    let opened = second.tool("open_project", json!({"projectId":id}));
    assert_ne!(opened["recoveryRequired"], true, "{opened}");
    second.tool("start_session", json!({"prompt":"Inspect reopened work"}));
    assert_eq!(second.tool("get_project_state", json!({"indexOnly":true}))["layerCount"], 1);
    second.tool("finish_session", json!({"commit":true}));
    drop(second.input.take());
    assert!(second.child.wait().unwrap().success());
    assert!(!descriptor.exists());
}

#[test]
fn app_headless_geometry_and_highlights_work_without_an_editor_view() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("triangle.glb");
    std::fs::write(&path, pm_scene3d::import::triangle_glb(&[[0.,0.,0.],[0.2,0.,0.],[0.,0.4,0.]], &[0,1,2])).unwrap();
    let mut client = Client::new(&root.path().join("profile"), None);
    client.rpc("initialize", json!({}));
    client.tool("create_project", json!({"name":"Synthetic headless geometry"}));
    client.tool("start_session", json!({"prompt":"Synthetic model placement"}));
    let asset = client.tool("import_media_to_timeline", json!({"path":path}))["assetId"].clone();
    client.tool("edit_3d", json!({"operation":"add_object","id":"source-scale","object":{"source":{"assetId":asset}},"keepRealWorldScale":true}));
    let source = client.tool("get_3d_scene", json!({"target":"source-scale"}));
    assert!((source["scene"]["objects"][0]["p"]["sy"]["v"].as_f64().unwrap() - 0.05_f64.sqrt() / 0.1).abs() < 1e-5);
    client.tool("edit_3d", json!({"operation":"add_object","id":"model","object":{"source":{"assetId":asset}}}));
    client.tool("edit_3d", json!({"operation":"auto_studio","target":"model"}));
    client.tool("edit_3d", json!({"operation":"drop_to_floor","target":"model"}));
    client.tool("edit_3d", json!({"operation":"set_camera","patch":{"p":{"x":0,"y":4,"z":6}}}));
    let light = client.tool("edit_3d", json!({"operation":"light_highlight","at":[0,1,0],"normal":[0,0,1]}))["result"]["id"].clone();
    let scene = client.tool("get_3d_scene", json!({}));
    let highlight = scene["scene"]["lights"].as_array().unwrap().iter().find(|l| l["id"] == light).unwrap();
    assert!(highlight["p"]["y"]["v"].as_f64().unwrap() > 0.);
    client.tool("finish_session", json!({"commit":true}));
    drop(client.input.take());
    assert!(client.child.wait().unwrap().success());
}

#[test]
fn app_mcp_interrupted_work_can_be_recovered_or_declined() {
    for decision in ["recover", "discard"] {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let mut first = Client::new(&profile, None);
        first.rpc("initialize", json!({}));
        let id = first.tool("create_project", json!({"name":"Synthetic interrupted work"}))["projectId"].clone();
        first.tool("start_session", json!({"prompt":"Synthetic interruption"}));
        first.tool("apply_commands", json!({"commands":[{"type":"add_layer","id":"synthetic","layerType":"solid","name":"Synthetic"}]}));
        first.tool("finish_session", json!({"commit":true}));
        first.child.kill().unwrap();
        first.child.wait().unwrap();
        let mut second = Client::new(&profile, None);
        second.rpc("initialize", json!({}));
        let required = second.tool("open_project", json!({"projectId":id}));
        assert_eq!(required["recoveryRequired"], true);
        second.tool("open_project", json!({"projectId":id,"recovery":{"ticket":required["ticket"],"decision":decision}}));
        second.tool("start_session", json!({"prompt":"Inspect recovery decision"}));
        assert_eq!(second.tool("get_project_state", json!({"indexOnly":true}))["layerCount"], u64::from(decision == "recover"));
        second.tool("finish_session", json!({"commit":true}));
        drop(second.input.take());
        assert!(second.child.wait().unwrap().success());
    }
}

#[test]
fn app_launch_file_recovery_is_listed_and_accepts_a_command_choice() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let path = root.path().join("project.json");
    let original = serde_json::to_vec(&json!({"id":"P","name":"Synthetic working file","compId":"C","compName":"Main","w":64,"h":36,"fps":30,"dur":2,"layers":[],"assets":{},"comps":{}})).unwrap();
    std::fs::write(&path, &original).unwrap();
    let mut first = Client::new(&profile, Some(&path));
    first.rpc("initialize", json!({}));
    first.tool("start_session", json!({"prompt":"Synthetic unsaved working file"}));
    first.tool("apply_commands", json!({"commands":[{"type":"add_layer","id":"synthetic","layerType":"solid","name":"Synthetic"}]}));
    first.tool("finish_session", json!({"commit":true}));
    first.child.kill().unwrap();
    first.child.wait().unwrap();
    let mut second = Client::new(&profile, Some(&path));
    second.rpc("initialize", json!({}));
    let pending = second.tool("list_projects", json!({}))["pendingRecoveries"][0].clone();
    assert_eq!(pending["recoveryRequired"], true);
    second.tool("open_project", json!({"projectId":pending["projectId"],"recovery":{"ticket":pending["ticket"],"decision":"recover"}}));
    second.tool("start_session", json!({"prompt":"Inspect recovered working file"}));
    assert_eq!(second.tool("get_project_state", json!({"indexOnly":true}))["layerCount"], 1);
    second.tool("finish_session", json!({"commit":true}));
    drop(second.input.take());
    assert!(second.child.wait().unwrap().success());
    assert_eq!(std::fs::read(path).unwrap(), original, "EOF must not overwrite unsaved working files");
}
#[test]
fn app_recovery_from_another_file_requires_a_choice_and_keeps_declined_work() {
    fn tree(root: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn read(root: &std::path::Path, at: &std::path::Path, files: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(at).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    read(root, &path, files);
                } else {
                    files.insert(path.strip_prefix(root).unwrap().into(), std::fs::read(path).unwrap());
                }
            }
        }
        let mut files = Default::default();
        read(root, root, &mut files);
        files
    }

    for decision in ["recover", "discard"] {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let original = root.path().join("original.json");
        let restored = root.path().join("restored.json");
        let saved = serde_json::to_vec(&json!({"id":"P","name":"Synthetic restored file","compId":"C","compName":"Main","w":64,"h":36,"fps":30,"dur":2,"layers":[],"assets":{},"comps":{}})).unwrap();
        std::fs::write(&original, &saved).unwrap();
        std::fs::write(&restored, &saved).unwrap();
        let mut first = Client::new(&profile, Some(&original));
        first.rpc("initialize", json!({}));
        let source_id = first.tool("list_projects", json!({}))["currentProjectId"]
            .as_str().unwrap().to_owned();
        first.tool("start_session", json!({"prompt":"Synthetic work from another file"}));
        first.tool("apply_commands", json!({"commands":[{"type":"add_layer","id":"synthetic","layerType":"solid","name":"Synthetic"}]}));
        first.tool("finish_session", json!({"commit":true}));
        first.child.kill().unwrap();
        first.child.wait().unwrap();
        std::fs::remove_file(&original).unwrap();
        let journal = profile.join("journals").join(source_id);
        let kept = tree(&journal);
        assert!(!kept.is_empty());

        let mut second = Client::new(&profile, Some(&restored));
        second.rpc("initialize", json!({}));
        let offer = second.tool("list_projects", json!({}))["pendingRecoveries"][0].clone();
        assert_eq!(offer["recovery"]["kind"], "from_other_file");
        assert_eq!(offer["recovery"]["sourceName"], "original.json");
        assert_eq!(offer["recovery"]["choices"], json!(["recover", "discard"]));
        let invalid = second.rpc("tools/call", json!({"name":"open_project","arguments":{
            "projectId":offer["projectId"],"recovery":{"ticket":offer["ticket"],"decision":"open_as_copy"}
        }}));
        assert_eq!(invalid["isError"], true, "another file's work cannot be copied unasked");
        second.tool("open_project", json!({"projectId":offer["projectId"],"recovery":{
            "ticket":offer["ticket"],"decision":decision
        }}));
        second.tool("start_session", json!({"prompt":"Inspect explicit recovery choice"}));
        assert_eq!(second.tool("get_project_state", json!({"indexOnly":true}))["layerCount"], u64::from(decision == "recover"));
        second.tool("finish_session", json!({"commit":true}));
        if decision == "discard" {
            assert_eq!(tree(&journal), kept, "declining keeps the other file's journal byte for byte");
        }
        drop(second.input.take());
        assert!(second.child.wait().unwrap().success());
        assert_eq!(std::fs::read(restored).unwrap(), saved, "EOF never overwrites the named file");
    }
}

#[test]
fn app_stdio_conformance_on_a_corpus_copy_commit_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let generated=Command::new("bun").args(["-e", "import { allProjects } from './engine/corpus/src/projects'; const p=allProjects().find(p=>!p.extensions?.length && p.doc.layers?.length); if(!p) process.exit(1); process.stdout.write(JSON.stringify(p.doc));"])
        .current_dir(repo).output().expect("Bun is required for the existing corpus generator");
    assert!(
        generated.status.success(),
        "Corpus preparation failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );
    for proxy in [false, true] {
        let case = root.path().join(if proxy { "proxy" } else { "direct" });
        std::fs::create_dir_all(&case).unwrap();
        let path = case.join("project.json");
        std::fs::write(&path, &generated.stdout).unwrap();
        let original = std::fs::read(&path).unwrap();
        let profile = case.join("profile");
        let mut owner = if proxy {
            let mut owner = Client::new(&profile, Some(&path));
            owner.rpc("initialize", json!({}));
            Some(owner)
        } else {
            None
        };
        let mut client = Client::new(&profile, if proxy { None } else { Some(&path) });
        let init=client.rpc("initialize",json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
        assert_eq!(init["serverInfo"]["name"], "powermove");
        let tools = client.rpc("tools/list", json!({}));
        assert_eq!(tools["tools"], json!(pm_mcp::catalog::served_tools()));
        client.tool(
            "start_session",
            json!({"prompt":"Synthetic corpus transport test"}),
        );
        let baseline = client.tool("get_project_state", json!({"indexOnly":true}))["layerCount"]
            .as_u64()
            .unwrap();
        for (commit, id) in [(true, "transport-commit"), (false, "transport-rollback")] {
            if !commit {
                client.tool("start_session", json!({"prompt":"Synthetic rollback test"}));
            }
            client.tool("get_project_state", json!({"indexOnly":true}));
            client.tool("apply_commands",json!({"commands":[{"type":"add_layer","id":id,"layerType":"solid","name":"Transport test"}]}));
            client.tool("finish_session", json!({"commit":commit}));
        }
        client.tool("start_session", json!({"prompt":"Verify committed state"}));
        let state = client.tool(
            "get_project_state",
            json!({"indexOnly":true,"layerLimit":200}),
        );
        assert_eq!(state["layerCount"], baseline + 1);
        assert!(
            state["layers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["id"] == "transport-commit")
        );
        assert!(
            !state["layers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["id"] == "transport-rollback")
        );
        client.tool("finish_session", json!({"commit":true}));
        assert_eq!(
            std::fs::read(path).unwrap(),
            original,
            "MCP commit is an edit, not an implicit file save"
        );
        drop(client.input.take());
        assert!(client.child.wait().unwrap().success());
        if let Some(owner) = owner.as_mut() {
            drop(owner.input.take());
            assert!(owner.child.wait().unwrap().success());
        }
        assert!(!profile.join("mcp/connection.json").exists());
    }
}

/// Effects compile in the helper the app bundle carries beside its executable.
fn glsl_compiler() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("POWERMOVE_GLSL_COMPILER") {
        return path.into();
    }
    let output = Command::new(env!("CARGO"))
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args([
            "build",
            "--locked",
            "-p",
            "pm-glsl",
            "--bin",
            "pm-glsl-compiler",
        ])
        .args(["--message-format=json"])
        .stderr(Stdio::inherit())
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|v| v["reason"] == "compiler-artifact" && v["target"]["name"] == "pm-glsl-compiler")
        .and_then(|v| v["executable"].as_str().map(Into::into))
        .expect("cargo did not report the GLSL compiler")
}
/// The real executable over real pipes, with a real engine child for each job.
#[test]
fn export_video_and_render_frames_reach_the_production_engine_job() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    let compiler = glsl_compiler();
    let mut client = Client::with_env(&profile, None, &[("POWERMOVE_GLSL_COMPILER", &compiler)]);
    client.rpc(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    client.tool(
        "create_project",
        json!({"name":"Synthetic export","width":320,"height":180,"fps":30,"duration":0.5}),
    );
    client.tool("start_session", json!({"prompt":"Synthetic export"}));
    client.tool(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"solid","layerType":"solid","name":"Solid"}]}),
    );
    // The width is a ceiling, as in Electron's snapshot: downscale large
    // compositions, but keep smaller ones on their original pixel grid.
    for (width, expected) in [(160, (160, 90)), (1280, (320, 180))] {
        let frames = client.rpc(
            "tools/call",
            json!({"name":"render_frames","arguments":{"times":[0],"width":width}}),
        );
        assert_eq!(frames["isError"], false, "{frames}");
        let image = &frames["content"][1];
        assert_eq!(image["mimeType"], "image/png");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .unwrap();
        let png = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        assert_eq!((png.info().width, png.info().height), expected);
    }
    let exported = client.tool("export_video", json!({"audio":false}));
    let path = std::path::Path::new(exported["path"].as_str().unwrap());
    assert!(path.starts_with(profile.join("exports")));
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp", "not an MP4 container");
}

/// The pixel at the centre of a PNG, as RGB.
fn centre_of_png(bytes: &[u8]) -> [u8; 3] {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let at = (info.height / 2) as usize * info.line_size
        + (info.width / 2) as usize * info.color_type.samples();
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}
/// An extension that wraps the `raster` service (it paints its text layers
/// itself) draws in a still and in an exported video of a project that has
/// no window, as it does when a window owns the project: the agent's host
/// answers the engine's override asks, a delivery from a realm of its own.
#[test]
fn a_raster_wrapper_draws_in_stills_and_exports_of_a_project_without_a_window() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let extension = profile.join("extensions/synthetic-wrap");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(
        extension.join("manifest.json"),
        json!({"id":"synthetic-wrap","name":"Wrap","version":"1.0.0","apiVersion":3}).to_string(),
    )
    .unwrap();
    std::fs::write(
        extension.join("index.ts"),
        "export default function(api: any){const base=api.services.get('raster');
           api.services.register('raster',(L: any,...rest: any[])=>{
             if(L.type!=='text')return base(L,...rest);
             const c=document.createElement('canvas');c.width=96;c.height=96;
             const x=c.getContext('2d');x.fillStyle='#00ff00';x.fillRect(0,0,96,96);
             return {cv:c,w:96,h:96,anchorX:48,anchorY:48}})}",
    )
    .unwrap();
    let compiler = glsl_compiler();
    let mut client = Client::with_env(&profile, None, &[("POWERMOVE_GLSL_COMPILER", &compiler)]);
    client.rpc(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    client.tool(
        "create_project",
        json!({"name":"Synthetic wrap","width":320,"height":180,"fps":30,"duration":0.5}),
    );
    client.tool("start_session", json!({"prompt":"Synthetic wrap"}));
    client.tool(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"title","layerType":"text","name":"Title",
            "content":{"text":"."}}]}),
    );
    let frames = client.rpc(
        "tools/call",
        json!({"name":"render_frames","arguments":{"times":[0],"width":320}}),
    );
    assert_eq!(frames["isError"], false, "{frames}");
    let still = base64::engine::general_purpose::STANDARD
        .decode(frames["content"][1]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(centre_of_png(&still), [0, 255, 0], "the still");

    let exported = client.tool("export_video", json!({"audio":false}));
    let video = std::path::Path::new(exported["path"].as_str().unwrap());
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg");
    let frame = root.path().join("frame.png");
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .args(["-frames:v", "1"])
        .arg(&frame)
        .status()
        .unwrap();
    assert!(status.success());
    let [r, g, b] = centre_of_png(&std::fs::read(frame).unwrap());
    assert!(r < 40 && g > 215 && b < 40, "the video: {r} {g} {b}");
}

/// A project opened at launch starts its extensions, but the tool server
/// answers at once: it neither waits for them nor quits when they are slow.
#[test]
fn a_project_opened_at_launch_is_served_while_its_extensions_still_load() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let extension = profile.join("extensions/synthetic-slow");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(
        extension.join("manifest.json"),
        json!({"id":"synthetic-slow","name":"Slow","version":"1.0.0","apiVersion":3}).to_string(),
    )
    .unwrap();
    // Held and never released: the load settles only at the hold's limit.
    std::fs::write(
        extension.join("index.ts"),
        "export default function(api: any){api.readiness.hold()}",
    )
    .unwrap();
    let initialize = json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}});
    let saved = {
        let mut client = Client::new(&profile, None);
        client.rpc("initialize", initialize.clone());
        client.tool(
            "create_project",
            json!({"name":"Slow load","width":320,"height":180,"fps":30,"duration":0.5}),
        );
        client.tool("start_session", json!({"prompt":"Slow load"}));
        let saved = client.tool("save_project", json!({}))["path"]
            .as_str()
            .map(std::path::PathBuf::from)
            .unwrap();
        client.finish();
        saved
    };
    let started = std::time::Instant::now();
    let mut client = Client::new(&profile, Some(&saved));
    client.rpc("initialize", initialize);
    client.tool("start_session", json!({"prompt":"Slow load"}));
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the tool server waited {:?} for the extensions",
        started.elapsed()
    );
}

/// A raster wrapper is more than an effect definition: it can measure hidden
/// DOM and rasterize detached text through the app's actual font registry.
#[test]
fn headless_raster_wrappers_have_base_text_and_real_dom_layout() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let extension = profile.join("extensions/synthetic-raster");
    std::fs::create_dir_all(&extension).unwrap();
    std::fs::write(
        extension.join("manifest.json"),
        json!({"id":"synthetic-raster","name":"Synthetic raster","version":"1.0.0","apiVersion":3})
            .to_string(),
    )
    .unwrap();
    std::fs::write(extension.join("index.ts"), r#"
        export default function activate(api: any) {
          const base = api.services.get('raster');
          const measure = document.createElement('div');
          Object.assign(measure.style, {position:'fixed',visibility:'hidden',width:'37px',height:'11px'});
          document.body.append(measure);
          api.services.register('raster', (layer: any, scale: number, time: number) => {
            if (layer.type !== 'text') return base(layer, scale, time);
            const full = base(layer, scale, time);
            const detached = base({...layer,id:'detached',d:{...layer.d,text:'I'}}, scale, time);
            if (!full?.cv || !detached?.cv || full.w <= detached.w || detached.w <= 0)
              throw Error('The base text renderer did not draw detached content');
            const rect = measure.getBoundingClientRect();
            if (rect.width !== 37 || rect.height !== 11) throw Error('Missing real DOM layout: '+JSON.stringify(rect));
            const cv = document.createElement('canvas'); cv.width=64; cv.height=64;
            const ctx = cv.getContext('2d'); ctx.fillStyle='#00ff00';
            ctx.fillRect(0,0,rect.width,rect.height);
            return {cv,w:64,h:64,anchorX:0,anchorY:0,key:'synthetic-measured'};
          });
        }
    "#).unwrap();
    let project = root.path().join("project.json");
    std::fs::write(&project, json!({
        "id":"synthetic","name":"Synthetic","compId":"root","w":320,"h":180,"fps":30,"dur":1,
        "bg":"#000000","assets":{},"comps":{},"layers":[{
            "id":"probe","type":"text","name":"Probe","from":0,"dur":1,"on":true,
            "fx":[],"masks":[],"p":{
                "position.x":{"v":160,"kf":[]},"position.y":{"v":90,"kf":[]},
                "scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"opacity":{"v":100,"kf":[]}},
            "d":{"text":"WWWW","font":"Helvetica","size":48,"weight":400,"align":"left","color":"#ffffff"}
        }]
    }).to_string()).unwrap();
    let compiler = glsl_compiler();
    let mut client = Client::with_env(
        &profile,
        Some(&project),
        &[("POWERMOVE_GLSL_COMPILER", &compiler)],
    );
    client.rpc("initialize", json!({}));
    client.tool("start_session", json!({"prompt":"Synthetic raster check"}));
    for time in [0., 0.1] {
        let reply = client.rpc(
            "tools/call",
            json!({"name":"render_frames","arguments":{"times":[time],"width":320}}),
        );
        assert_eq!(reply["isError"], false, "{reply}");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(reply["content"][1]["data"].as_str().unwrap())
            .unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::ALPHA);
        let mut reader = decoder.read_info().unwrap();
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut rgba).unwrap();
        let green = rgba[..info.buffer_size()]
            .chunks_exact(4)
            .filter(|p| p[0] < 2 && p[1] > 253 && p[2] < 2 && p[3] > 253)
            .count();
        assert_eq!(
            green,
            37 * 11,
            "The measured wrapper did not reach the rendered frame"
        );
    }
    // Delivery uses a frozen realm and a different hidden page. The wrapper
    // throws if either its fonts or its synchronous layout are unavailable.
    let exported = client.tool("export_video", json!({"audio":false}));
    assert!(
        std::fs::metadata(exported["path"].as_str().unwrap())
            .unwrap()
            .len()
            > 0
    );
}

/// One second of tone, then one of silence.
fn tone_wav() -> Vec<u8> {
    let rate = 8000u32;
    let mut samples = vec![];
    for i in 0..rate {
        let tone = (i as f64 * 0.2).sin() * 0.5 * 32767.;
        samples.extend_from_slice(&(tone as i16).to_le_bytes());
    }
    samples.resize(samples.len() * 2, 0);
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for field in [16u32, 1 | 1 << 16, rate, rate * 2, 2 | 16 << 16] {
        wav.extend_from_slice(&field.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}
/// Real files over real pipes into the kernel's durable media.
#[test]
fn import_media_retains_bytes_places_layers_and_replaces_in_place() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    let audio = root.path().join("tone.wav");
    std::fs::write(&audio, tone_wav()).unwrap();
    let mut client = Client::new(&profile, None);
    client.rpc(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    client.tool(
        "create_project",
        json!({"name":"Synthetic import","width":320,"height":180,"fps":30,"duration":4}),
    );
    client.tool("start_session", json!({"prompt":"Synthetic import"}));
    let refused = client.rpc(
        "tools/call",
        json!({"name":"import_media","arguments":{"path":audio,"expectedFingerprint":"v2:0:none"}}),
    );
    assert_eq!(refused["isError"], true, "{refused}");
    let imported = client.tool("import_media", json!({"path":audio}));
    assert_eq!(imported["status"], "imported", "{imported}");
    assert_eq!(imported["persisted"], true);
    let asset = imported["assetId"].as_str().unwrap().to_owned();
    let fingerprint = imported["fingerprint"].as_str().unwrap().to_owned();
    assert!(fingerprint.starts_with("v2:"), "{fingerprint}");
    std::fs::remove_file(&audio).unwrap();
    let listed = client.tool("list_media", json!({"assetId":asset}));
    let row = &listed["assets"][0];
    assert_eq!(row["status"], "available", "{listed}");
    assert_eq!(row["fingerprint"], fingerprint);
    assert_eq!(row["kind"], "audio");
    assert_eq!(row["duration"], 2.0);
    let probe = client.rpc(
        "tools/call",
        json!({"name":"probe_media","arguments":{"assetId":asset}}),
    );
    assert_eq!(probe["isError"], false, "{probe}");
    std::fs::write(&audio, tone_wav()).unwrap();
    let again = client.tool("import_media", json!({"path":audio}));
    assert_eq!(again["status"], "already_imported", "{again}");
    assert_eq!(again["assetId"], asset);
    let placed = client.tool("import_media_to_timeline", json!({"path":audio,"at":1}));
    let layer = placed["layerId"].as_str().expect("placed layer").to_owned();
    assert_eq!(placed["assetId"], asset);
    let state = client.tool("get_timeline", json!({}));
    assert!(state.to_string().contains(&layer), "{state}");
    let mut longer = tone_wav();
    longer.push(0);
    longer.push(0);
    std::fs::write(&audio, longer).unwrap();
    let replaced = client.tool("replace_media", json!({"assetId":asset,"path":audio}));
    assert_eq!(replaced["status"], "replaced", "{replaced}");
    assert_eq!(replaced["assetId"], asset);
    assert_ne!(replaced["fingerprint"], fingerprint);
    let mismatched = root.path().join("still.png");
    std::fs::write(&mismatched, b"not really a png").unwrap();
    let wrong = client.rpc(
        "tools/call",
        json!({"name":"replace_media","arguments":{"assetId":asset,"path":mismatched}}),
    );
    assert_eq!(wrong["isError"], true, "{wrong}");
    client.tool("finish_session", json!({"commit":true}));
}
/// A real audio file through the app's media tools: measured, not modelled.
#[test]
fn media_tools_measure_a_linked_original() {
    let root = tempfile::tempdir().unwrap();
    let audio = root.path().join("tone.wav");
    std::fs::write(&audio, tone_wav()).unwrap();
    let project = json!({"id":"synthetic","name":"Synthetic media","compId":"c","w":320,"h":180,
        "fps":30,"dur":2,"bg":"#000000","comps":{},"layers":[],"params":{},"markers":[],
        "assets":{"tone":{"id":"tone","kind":"audio","name":"Tone","sourcePath":audio}}});
    let path = root.path().join("project.json");
    std::fs::write(&path, project.to_string()).unwrap();
    let mut client = Client::new(&root.path().join("profile"), Some(&path));
    client.rpc(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    client.tool("start_session", json!({"prompt":"Synthetic media"}));
    let probe = client.rpc(
        "tools/call",
        json!({"name":"probe_media","arguments":{"assetId":"tone"}}),
    );
    assert_eq!(probe["isError"], false, "{probe}");
    let text = probe["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("8000"), "{text}");
    let wave = client.rpc(
        "tools/call",
        json!({"name":"media_waveform","arguments":{"assetId":"tone"}}),
    );
    assert_eq!(wave["isError"], false, "{wave}");
    let measured: Value =
        serde_json::from_str(wave["content"][0]["text"].as_str().unwrap()).unwrap();
    let silence = &measured["silences"][0];
    assert!(
        (silence[0].as_f64().unwrap() - 1.).abs() < 0.05,
        "{measured}"
    );
}

/// Each tool either reaches a body or refuses with the documented reason. The
/// table in docs/native/mcp.md is the list of refusals, in both directions.
#[test]
fn every_catalog_tool_has_a_body_or_a_documented_refusal() {
    let docs = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../docs/native/mcp.md"),
    )
    .unwrap();
    let section = docs
        .split("<!-- refused-tools -->")
        .nth(1)
        .expect("refusal table markers");
    let documented = section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .flat_map(|line| line.split('|').next())
        .flat_map(|tools| tools.split(", "))
        .map(|tool| tool.trim().trim_matches('`').to_owned())
        .filter(|tool| !tool.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    let mut client = Client::new(&profile, None);
    client.rpc(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    client.tool("create_project", json!({"name":"Synthetic catalog"}));
    client.tool("start_session", json!({"prompt":"Synthetic catalog"}));
    let mut refused = std::collections::BTreeSet::new();
    for tool in pm_mcp::catalog::names() {
        if matches!(
            tool.as_str(),
            "start_session" | "finish_session" | "export_video" | "render_frames"
        ) {
            continue;
        }
        // These two validate their arguments ahead of the capability check.
        let arguments = match tool.as_str() {
            "thread_read" => json!({"threadId":"none"}),
            "fork_builtin_extension" => json!({"id":"timeline"}),
            "publish_extensions" => json!({"extensions":[]}),
            _ => json!({}),
        };
        let reply = client.rpc("tools/call", json!({"name":tool,"arguments":arguments}));
        let text = reply["content"][0]["text"].as_str().unwrap_or_default();
        if text.starts_with("Not available in this build") {
            assert!(text.len() > 40, "{tool} refuses without a reason: {text}");
            refused.insert(tool);
        }
    }
    assert_eq!(refused, documented);
}
