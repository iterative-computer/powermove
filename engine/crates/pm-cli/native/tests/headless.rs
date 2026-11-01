use base64::{Engine, engine::general_purpose::STANDARD};
use pm_native_cli::{
    backend::{self, Backend, ExportChild, FrameEngine, FrameSpec, Image},
    options::Options,
    project::Project,
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::{TempDir, tempdir};

struct Fixture {
    dir: TempDir,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let source = dir.path().join("synthetic.json");
        fs::write(&source,serde_json::to_vec(&json!({"v":"1.0.0","unknownEnvelope":{"keep":true},"ws":{"unknown":5},"proj":synthetic()})).unwrap()).unwrap();
        Self { dir, source }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
    fn run(&self, command: &str, args: &[&str]) -> (i32, Value) {
        let out = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
            .arg(command)
            .arg(&self.source)
            .args(args)
            .output()
            .unwrap();
        let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "stdout {:?}, stderr {:?}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (out.status.code().unwrap(), value)
    }
}
fn synthetic() -> Value {
    json!({"id":"synthetic-project","name":"Synthetic project","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","layers":[],"comps":{},"assets":{},"markers":[],"work":[0,2],"params":{},"unknownProject":{"keep":[1,2,3]}})
}
fn body(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn args(raw: &[&str]) -> pm_native_cli::Result<Options> {
    Options::parse(raw.iter().map(OsString::from), &mut &b""[..])
}
fn add() -> Value {
    json!({"type":"add_layer","id":"synthetic-layer","layerType":"solid","name":"Synthetic solid"})
}

#[test]
fn help_and_usage_are_one_machine_readable_result() {
    let output = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        body(&v)["usage"]
            .as_str()
            .unwrap()
            .contains("render-frames")
    );
    let f = Fixture::new();
    let (code, result) = f.run("unknown", &[]);
    assert_eq!(code, 2);
    assert_eq!(body(&result)["code"], "invalid_arguments");
}
#[test]
fn options_refuse_duplicates_misplaced_and_malformed_inputs() {
    for raw in [
        vec!["apply", "p"],
        vec!["info", "p", "--output", "q"],
        vec!["info", "p", "--args", "{}", "--args", "{}"],
        vec!["info", "p", "--args", "[]"],
        vec!["info", "p", "--args", "{"],
        vec!["export", "p", "--output", "q", "--backend", "fake"],
        vec!["apply", "p", "--output", "q", "--expected-revision", "-1"],
        vec!["migrate", "p", "--scratch", "q"],
        vec!["migrate", "p", "--dry-run"],
        vec!["info", "p", "--dry-run"],
        vec!["info", "p", "--bogus", "x"],
    ] {
        assert!(args(&raw).is_err(), "{raw:?}");
    }
}
#[test]
fn arguments_from_stdin_keep_the_exact_mcp_shape() {
    let p = Options::parse(
        ["apply", "p", "--output", "q", "--args-file", "-"].map(OsString::from),
        &mut &br#"{"commands":[{"type":"set_composition","patch":{"name":"Exact"}}]}"#[..],
    )
    .unwrap();
    assert_eq!(p.arguments["commands"][0]["patch"]["name"], "Exact");
}
#[test]
fn missing_input_and_unreadable_argument_files_are_io_errors() {
    let f = Fixture::new();
    assert_eq!(
        f.run("info", &["--args-file", "/does-not-exist/synthetic.json"])
            .0,
        5
    );
    let out = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .args(["open", "/does-not-exist/synthetic.pmv"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
}
#[test]
fn reads_reuse_mcp_digests_without_source_side_writes() {
    let f = Fixture::new();
    let before = fs::read(&f.source).unwrap();
    for command in ["open", "info"] {
        let (code, result) = f.run(command, &[]);
        assert_eq!(code, 0, "{result}");
        let p = Project::open(&f.source, false).unwrap();
        assert_eq!(
            body(&result),
            body(&pm_mcp::success(p.info(&json!({})).unwrap()))
        );
    }
    let (code, result) = f.run("timeline", &[]);
    assert_eq!(code, 0, "{result}");
    let p = Project::open(&f.source, false).unwrap();
    assert_eq!(
        body(&result),
        body(&pm_mcp::success(p.timeline(&json!({})).unwrap()))
    );
    assert_eq!(fs::read(&f.source).unwrap(), before);
    assert_eq!(fs::read_dir(f.dir.path()).unwrap().count(), 1);
}
#[test]
fn apply_resolves_ids_in_order_and_persists_one_undo_entry() {
    let f = Fixture::new();
    let before = fs::read(&f.source).unwrap();
    let output = f.path("edited.pmv");
    let request = json!({"label":"Synthetic batch","commands":[add(),{"type":"set_layer","target":"synthetic-layer","patch":{"name":"Exact authored name"}}]});
    let (code, result) = f.run(
        "apply",
        &[
            "--output",
            output.to_str().unwrap(),
            "--expected-revision",
            "1",
            "--args",
            &request.to_string(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    assert_eq!(body(&result)["results"].as_array().unwrap().len(), 2);
    assert_eq!(fs::read(&f.source).unwrap(), before);
    let opened = pm_kernel::open_project(&output, Default::default()).unwrap();
    assert_eq!(
        opened
            .file
            .envelope
            .extra
            .get("unknownEnvelope")
            .map(pm_native_cli::to_json),
        Some(json!({"keep":true}))
    );
    let p = Project::open(&output, false).unwrap();
    assert_eq!(
        p.document().unwrap()["layers"][0]["name"],
        "Exact authored name"
    );
    assert_eq!(
        p.document().unwrap()["unknownProject"],
        json!({"keep":[1,2,3]})
    );
    assert_eq!(p.published().unwrap().history.entries, 1);
    let mut store = pm_kernel::DocumentStore::open(opened);
    store.undo().unwrap();
    assert!(
        pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap())
            .get("layers")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn invalid_later_command_keeps_the_entire_batch_unapplied() {
    let f = Fixture::new();
    let before = fs::read(&f.source).unwrap();
    let output = f.path("refused.pmv");
    let request = json!({"commands":[add(),{"type":"set_layer","target":"missing-id","patch":{"name":"Wrong"}}]});
    let (code, result) = f.run(
        "apply",
        &[
            "--output",
            output.to_str().unwrap(),
            "--args",
            &request.to_string(),
        ],
    );
    assert_ne!(code, 0, "{result}");
    assert!(!output.exists());
    assert_eq!(fs::read(&f.source).unwrap(), before);
}
#[test]
fn revision_conflict_happens_before_mutation_or_save() {
    let f = Fixture::new();
    let output = f.path("stale.pmv");
    let (code, result) = f.run(
        "apply",
        &[
            "--output",
            output.to_str().unwrap(),
            "--expected-revision",
            "0",
            "--args",
            &json!({"commands":[add()]}).to_string(),
        ],
    );
    assert_eq!(code, 4);
    assert_eq!(body(&result)["code"], "revision_conflict");
    assert!(!output.exists());
}
#[test]
fn existing_and_linked_destinations_are_never_replaced() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let target = f.path("authored.pmv");
    fs::write(&target, b"synthetic authored bytes").unwrap();
    for name in ["authored.pmv", "linked.pmv", "dangling.pmv"] {
        let path = f.path(name);
        if name == "linked.pmv" {
            symlink(&target, &path).unwrap();
        }
        if name == "dangling.pmv" {
            symlink(f.path("missing"), &path).unwrap();
        }
        let (code, result) = f.run(
            "apply",
            &[
                "--output",
                path.to_str().unwrap(),
                "--args",
                &json!({"commands":[add()]}).to_string(),
            ],
        );
        assert_eq!(code, 2, "{result}");
    }
    assert_eq!(fs::read(target).unwrap(), b"synthetic authored bytes");
}
#[test]
fn legacy_pmv3_embedded_media_is_preserved_in_pmv4() {
    let f = Fixture::new();
    let input = f.path("synthetic-v3.pmv");
    let header=serde_json::to_vec(&json!({"document":{"proj":({let mut p=synthetic();p["assets"]=json!({"synthetic-media":{"id":"synthetic-media"}});p}),"unknown":7},"media":[{"id":"synthetic-media","type":"application/octet-stream","offset":0,"length":5}]})).unwrap();
    let mut bytes = b"PMV3\n".to_vec();
    bytes.extend((header.len() as u32).to_le_bytes());
    bytes.extend(header);
    bytes.extend(b"hello");
    fs::write(&input, &bytes).unwrap();
    let mut p = Project::open(&input, true).unwrap();
    let output = f.path("v4.pmv");
    p.apply(&json!({"commands":[add()]}), &output).unwrap();
    assert_eq!(fs::read(&input).unwrap(), bytes);
    let opened = pm_kernel::open_project(&output, Default::default()).unwrap();
    assert_eq!(opened.file.media.len(), 1);
    let media = f.path("roundtrip-media.bin");
    opened
        .file
        .media
        .iter()
        .next()
        .unwrap()
        .copy_to(&media)
        .unwrap();
    assert_eq!(fs::read(media).unwrap(), b"hello");
}
#[test]
fn corrupt_container_cannot_report_validation_success() {
    let f = Fixture::new();
    let output = f.path("valid.pmv");
    Project::open(&f.source, true)
        .unwrap()
        .apply(&json!({"commands":[add()]}), &output)
        .unwrap();
    let mut bytes = fs::read(&output).unwrap();
    bytes[8] ^= 1;
    fs::write(&output, bytes).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("validate")
        .arg(output)
        .output()
        .unwrap();
    assert!(!out.status.success());
}
#[test]
fn validation_reports_provable_layer_reference_issues() {
    let f = Fixture::new();
    let output = f.path("layers.pmv");
    Project::open(&f.source, true)
        .unwrap()
        .apply(&json!({"commands":[add()]}), &output)
        .unwrap();
    let p = Project::open(&output, false).unwrap();
    let mut doc = p.document().unwrap();
    doc["layers"][0]["parent"] = json!("absent-layer");
    fs::write(&f.source, doc.to_string()).unwrap();
    let (code, result) = f.run("validate", &[]);
    assert_eq!(code, 1);
    assert_eq!(body(&result)["ok"], false);
    assert_eq!(
        body(&result)["issues"][0]["code"],
        "missing_layer_reference"
    );
}
#[test]
fn valid_project_reports_integrity_scope_without_claiming_creative_lint() {
    let f = Fixture::new();
    let (code, result) = f.run("validate", &[]);
    assert_eq!(code, 0, "{result}");
    assert_eq!(body(&result)["ok"], true);
    assert_eq!(body(&result)["creativeLintAvailable"], false);
}
#[test]
fn frame_plan_preserves_mcp_clamping_rounding_dedup_and_defaults() {
    let p = synthetic();
    let plan =
        backend::frame_plan(&p, &json!({"times":[0.0001,0.0002,99.0],"width":320}), 9).unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(
        plan[0],
        FrameSpec {
            time: 0.,
            width: 320,
            height: 180,
            revision: 9
        }
    );
    assert_eq!(plan[1].time, 2.);
    let defaults = backend::frame_plan(&p, &json!({"times":[]}), 1).unwrap();
    assert_eq!(
        defaults.iter().map(|s| s.time).collect::<Vec<_>>(),
        vec![0., 1., 1.967]
    );
    assert!(backend::frame_plan(&p, &json!({"times":[-1]}), 1).is_err());
    assert!(backend::frame_plan(&p, &json!({"times":[0,1,2,3,4,5]}), 1).is_err());
}
#[test]
fn test_double_frames_are_real_decodable_labelled_pngs_in_mcp_content() {
    let f = Fixture::new();
    let (code, result) = f.run(
        "render-frames",
        &[
            "--backend",
            "test-double",
            "--args",
            r#"{"times":[0,1],"width":160}"#,
        ],
    );
    assert_eq!(code, 0, "{result}");
    assert_eq!(body(&result)["testDouble"], true);
    assert_eq!(body(&result)["rendered"], 2);
    for content in result["content"].as_array().unwrap().iter().skip(1) {
        assert_eq!(content["mimeType"], "image/png");
        let bytes = STANDARD.decode(content["data"].as_str().unwrap()).unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        assert_eq!((decoder.info().width, decoder.info().height), (160, 90));
        let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
        decoder.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels[3], 255);
    }
}
struct BadEngine;
impl FrameEngine for BadEngine {
    fn frame(&mut self, _: &Project, s: &FrameSpec) -> pm_native_cli::Result<Image> {
        Ok(Image {
            png: b"\x89PNG\r\n\x1a\n".to_vec(),
            revision: s.revision + 1,
        })
    }
}
impl ExportChild for BadEngine {
    fn export(&mut self, _: &Project, _: &Value, _: &Path) -> pm_native_cli::Result<()> {
        Ok(())
    }
}
impl Backend for BadEngine {}
#[test]
fn stale_frames_and_child_success_without_a_file_are_failures() {
    let f = Fixture::new();
    let p = Project::open(&f.source, false).unwrap();
    let error = backend::render_frames(&p, &json!({"times":[0]}), &mut BadEngine).unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    let error =
        backend::export(&p, &json!({}), &f.path("missing.mp4"), &mut BadEngine).unwrap_err();
    assert_eq!(error.code, "export_error");
}
#[test]
fn test_export_returns_only_after_a_labelled_manifest_exists() {
    let f = Fixture::new();
    let output = f.path("synthetic.test-export.json");
    let (code, result) = f.run(
        "export",
        &[
            "--backend",
            "test-double",
            "--output",
            output.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    assert_eq!(body(&result)["testDouble"], true);
    let v: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(v["encodedVideo"], false);
    assert_eq!(v["projectId"], "synthetic-project");
    let (code, _) = f.run(
        "export",
        &[
            "--backend",
            "test-double",
            "--output",
            f.path("fake.mp4").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 2);
}
#[test]
fn alpha_export_requires_prores_even_for_the_double() {
    let f = Fixture::new();
    let (code, result) = f.run(
        "export",
        &[
            "--backend",
            "test-double",
            "--output",
            f.path("alpha.test-export.json").to_str().unwrap(),
            "--args",
            r#"{"alpha":true}"#,
        ],
    );
    assert_eq!(code, 2, "{result}");
}
#[test]
#[cfg(not(target_os = "macos"))]
fn native_render_and_export_fail_explicitly_on_linux() {
    let f = Fixture::new();
    assert_eq!(f.run("render-frames", &[]).0, 3);
    assert_eq!(
        f.run(
            "export",
            &["--output", f.path("native.mp4").to_str().unwrap()]
        )
        .0,
        3
    );
}
#[test]
fn stdio_arguments_work_through_the_actual_binary() {
    let f = Fixture::new();
    let output = f.path("stdin.pmv");
    let mut child = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("apply")
        .arg(&f.source)
        .arg("--output")
        .arg(&output)
        .args(["--args-file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(json!({"commands":[add()]}).to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(output.is_file());
}

#[test]
fn source_hash_guards_bind_separate_processes_to_the_inspected_file() {
    let f = Fixture::new();
    let opened = pm_kernel::open_project(&f.source, Default::default()).unwrap();
    let hash = opened.file.version.as_ref().unwrap().hash.clone();
    let output = f.path("hash-guarded.pmv");
    let (code, result) = f.run(
        "apply",
        &[
            "--expected-source-hash",
            &hash.to_uppercase(),
            "--output",
            output.to_str().unwrap(),
            "--args",
            &json!({"commands":[add()]}).to_string(),
        ],
    );
    assert_eq!(code, 0, "{result}");
    let mut changed = synthetic();
    changed["name"] = json!("Changed after inspection");
    fs::write(&f.source, changed.to_string()).unwrap();
    let stale = f.path("stale-hash.pmv");
    let (code, result) = f.run(
        "apply",
        &[
            "--expected-source-hash",
            &hash,
            "--output",
            stale.to_str().unwrap(),
            "--args",
            &json!({"commands":[add()]}).to_string(),
        ],
    );
    assert_eq!(code, 4, "{result}");
    assert!(!stale.exists());
    assert!(args(&["info", "p", "--expected-source-hash", "bad"]).is_err());
}
#[test]
fn source_change_after_actor_open_is_refused_before_edits() {
    let f = Fixture::new();
    let mut p = Project::open(&f.source, true).unwrap();
    let mut changed = synthetic();
    changed["name"] = json!("Changed while planning");
    fs::write(&f.source, changed.to_string()).unwrap();
    let error = p
        .apply(&json!({"commands":[add()]}), &f.path("changed.pmv"))
        .unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(p.published().unwrap().revision(), 1);
}
#[test]
fn actor_edit_respects_existing_locked_layers() {
    let f = Fixture::new();
    let locked = f.path("locked.pmv");
    Project::open(&f.source,true).unwrap().apply(&json!({"commands":[add(),{"type":"set_layer","target":"synthetic-layer","patch":{"locked":true}}]}),&locked).unwrap();
    let mut p = Project::open(&locked, true).unwrap();
    let before = p.document().unwrap();
    let error=p.apply(&json!({"commands":[{"type":"set_property","target":"synthetic-layer","path":"opacity","value":20}]}),&f.path("rejected.pmv")).unwrap_err();
    assert!(!error.message.is_empty());
    assert_eq!(p.document().unwrap(), before);
}
#[test]
fn portrait_frames_and_extreme_aspect_ratios_are_bounded() {
    let mut p = synthetic();
    p["w"] = json!(180);
    p["h"] = json!(320);
    let plan = backend::frame_plan(&p, &json!({"times":[0],"width":160}), 1).unwrap();
    assert_eq!((plan[0].width, plan[0].height), (160, 284));
    p["w"] = json!(1);
    p["h"] = json!(10000);
    assert!(backend::frame_plan(&p, &json!({}), 1).is_err());
}
#[test]
fn failed_save_keeps_acknowledged_edits_in_a_recovery_archive() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let blocked = f.path("blocked");
    fs::create_dir(&blocked).unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o500)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("apply")
        .arg(&f.source)
        .arg("--output")
        .arg(blocked.join("output.pmv"))
        .arg("--args")
        .arg(json!({"commands":[add()]}).to_string())
        .output()
        .unwrap();
    fs::set_permissions(blocked, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!out.status.success());
    let diagnostic = String::from_utf8(out.stderr).unwrap();
    let receipt = diagnostic
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v.get("recoveryJournal").is_some())
        .expect("acknowledged changes must have an archive");
    let archive = PathBuf::from(receipt["recoveryJournal"].as_str().unwrap());
    assert!(archive.join("journal").is_dir());
    // Test owns only synthetic state under this randomly created scratch path.
    fs::remove_dir_all(archive).unwrap();
}

#[test]
fn anonymous_and_named_composition_identities_reach_the_engine_unchanged() {
    let f = Fixture::new();
    let p = Project::open(&f.source, false).unwrap();
    assert_eq!(p.composition_id().unwrap(), "@root:synthetic-project");
    drop(p);
    let mut doc = synthetic();
    doc["compId"] = json!("authored-comp-id");
    fs::write(&f.source, doc.to_string()).unwrap();
    let p = Project::open(&f.source, false).unwrap();
    assert_eq!(p.composition_id().unwrap(), "authored-comp-id");
}
#[test]
#[cfg(target_os = "macos")]
fn macos_metal_frame_capture_uses_the_real_service() {
    let f = Fixture::new();
    let (code, result) = f.run("render-frames", &["--args", r#"{"times":[0],"width":160}"#]);
    assert_eq!(code, 0, "{result}");
    assert_eq!(body(&result)["rendered"], 1);
    assert!(body(&result).get("testDouble").is_none());
}
#[test]
#[cfg(target_os = "macos")]
fn macos_text_capture_pumps_the_native_font_broker() {
    let f = Fixture::new();
    let text = f.path("text.pmv");
    Project::open(&f.source,true).unwrap().apply(&json!({"commands":[{"type":"add_layer","id":"synthetic-text","layerType":"text","name":"Synthetic text"}]}),&text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("render-frames")
        .arg(text)
        .args(["--args", r#"{"times":[0],"width":160}"#])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
#[test]
#[cfg(target_os = "macos")]
fn macos_video_export_finishes_the_same_binary_child() {
    let f = Fixture::new();
    let mut doc = synthetic();
    doc["dur"] = json!(0.1);
    doc["work"] = json!([0, 0.1]);
    fs::write(&f.source, doc.to_string()).unwrap();
    for (format, name) in [("mp4", "synthetic.mp4"), ("prores", "synthetic.mov")] {
        let destination = f.path(name);
        let (code, result) = f.run(
            "export",
            &[
                "--output",
                destination.to_str().unwrap(),
                "--args",
                &json!({"format":format,"audio":false,"motionBlur":false}).to_string(),
            ],
        );
        assert_eq!(code, 0, "{result}");
        assert!(fs::metadata(destination).unwrap().len() > 0);
        assert!(body(&result).get("testDouble").is_none());
    }
}
