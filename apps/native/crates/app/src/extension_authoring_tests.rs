use super::*;
use pm_mcp::extensions::{ExtensionHost, fork};
use serde_json::json;
use std::{
    fs,
    time::{Duration, Instant},
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(pm_kernel::new_id(name));
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn write(dir: &Path, manifest: Value, files: &[(&str, &str)]) -> Value {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    manifest::parse(&manifest).unwrap()
}

#[test]
fn the_shipped_sample_and_a_v3_extension_pass_and_broken_code_reports_why() {
    let root = scratch("pm-authoring-check");
    let sample = PACK
        .iter()
        .filter_map(|(n, t)| n.strip_prefix("samples/hello-panel/").map(|n| (n, *t)))
        .collect::<Vec<_>>();
    let raw: Value = serde_json::from_str(
        sample
            .iter()
            .find(|(n, _)| *n == "manifest.json")
            .unwrap()
            .1,
    )
    .unwrap();
    let parsed = write(&root.join("hello-panel"), raw, &sample);
    check(&root.join("hello-panel"), &parsed).unwrap();

    let v3 = write(
        &root.join("v3-command"),
        json!({"id":"v3-command","name":"V3","version":"1.0.0","apiVersion":3,"entry":"index.ts"}),
        &[("index.ts", "export default function activate() {}")],
    );
    check(&root.join("v3-command"), &v3).unwrap();

    let broken = write(
        &root.join("broken"),
        json!({"id":"broken","name":"Broken","version":"1.0.0","apiVersion":4,"entry":"index.tsx"}),
        &[("index.tsx", "export const nothing = 1;")],
    );
    let error = check(&root.join("broken"), &broken).unwrap_err();
    assert!(error.contains("exports view() or mount(env)"), "{error}");
    fs::write(root.join("broken/index.tsx"), "export function view( {").unwrap();
    let error = check(&root.join("broken"), &broken).unwrap_err();
    assert!(error.contains("TypeScript parse"), "{error}");
    fs::write(
        root.join("broken/index.tsx"),
        "import { readFileSync } from 'node:fs'; export const view = () => String(readFileSync);",
    )
    .unwrap();
    assert!(
        check(&root.join("broken"), &broken)
            .unwrap_err()
            .contains("node:fs")
    );
    let _ = fs::remove_dir_all(root);
}

/// Every built-in panel ships its source; a fork of each compiles with the
/// native compiler and draws its panel in a real panel isolate.
#[test]
fn every_built_in_forks_compiles_and_mounts_in_place_of_the_built_in() {
    let profile = scratch("pm-authoring-forks");
    let host = AppExtensions::new(profile.clone());
    let stage = profile.join("stage");
    let builtins = host.builtins().unwrap();
    for panel in LIVE_PANELS {
        assert!(
            builtins.join(panel).join("manifest.json").exists(),
            "{panel}"
        );
        let forked = fork(&host, panel, None, &stage).unwrap();
        let dir = PathBuf::from(forked["dir"].as_str().unwrap());
        let raw: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let parsed = host.manifest(&raw).unwrap();
        assert_eq!(replaced_panel(&parsed), Some(*panel));
        check(&dir, &parsed).unwrap_or_else(|e| panic!("{panel} fork: {e}"));
    }
    // The copy is content-addressed: a second call reuses it.
    assert_eq!(host.builtins().unwrap(), builtins);
    let _ = fs::remove_dir_all(profile);
}

const PROJECT: &str = r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":320,"kf":[]},"position.y":{"v":180,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##;

/// The live extension's panel as a window would draw it: compiled from the
/// user folder and mounted on a real kernel, in place of a built-in for a fork.
fn drawn(live: &Path, id: &str) -> (String, Vec<String>) {
    let raw: Value =
        serde_json::from_slice(&fs::read(live.join(id).join("manifest.json")).unwrap()).unwrap();
    let parsed = manifest::parse(&raw).unwrap();
    let runner = CompatRunner::new(&pm_compat_runner::initial()).unwrap();
    let graph = runner
        .compile(&live.join(id), parsed["entry"].as_str().unwrap(), 4)
        .unwrap()
        .to_json();
    drop(runner);
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
    actor
        .handle()
        .open(
            "synthetic",
            pm_kernel::OpenSource::Untitled(pm_model::json::parse(PROJECT).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let panel = replaced_panel(&parsed).unwrap_or(id).to_owned();
    let ticket = match replaced_panel(&parsed) {
        Some(built_in) => panels.replace(built_in, id, &graph),
        None => panels.mount_extension(id, &graph, None),
    };
    settle(ticket.unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let texts = loop {
        let _ = panels.publish();
        if let Some(p) = panels
            .take_publications()
            .into_iter()
            .rev()
            .find(|p| p.panel == panel)
        {
            break p
                .surface
                .nodes()
                .values()
                .map(|n| n.text.clone())
                .collect::<Vec<_>>();
        }
        assert!(Instant::now() < deadline, "{id} never drew {panel}");
        std::thread::sleep(Duration::from_millis(5));
    };
    panels.close();
    actor.shutdown();
    (panel, texts)
}

/// Item 4's agent flow, headless: through the MCP session an agent writes a
/// panel, forks the timeline to add a column, adds a viewer overlay, and the
/// user undoes the last change. Each step is its own change set.
#[test]
fn an_agent_adds_a_panel_a_timeline_column_and_a_viewer_overlay_then_one_is_undone() {
    use pm_mcp::{Automation, McpSession};
    use std::sync::{Arc, Mutex};
    let profile = scratch("pm-authoring-flow");
    let live = profile.join("extensions");
    let mut automation = Automation::new(profile.clone(), profile.join("exports")).unwrap();
    automation
        .create_project(&json!({"name":"Synthetic"}))
        .unwrap();
    let extensions = AppExtensions {
        profile: profile.clone(),
        roots: vec![live.clone()],
    };
    automation.extensions = Some(Arc::new(extensions));
    let host = Arc::new(Mutex::new(automation));
    let mut session = McpSession::new(host.clone());
    let call = |session: &mut McpSession, name: &str, args: Value| -> Value {
        let result = session.call(name, args);
        assert_eq!(result["isError"], false, "{name}: {result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let started = call(
        &mut session,
        "start_session",
        json!({"prompt":"Customise the app"}),
    );
    let stage = |value: &Value| PathBuf::from(value["stagingDirectory"].as_str().unwrap());
    let staging = stage(&started);
    assert!(
        PathBuf::from(started["apiPackDirectory"].as_str().unwrap())
            .join("EXTENSIONS.md")
            .exists()
    );

    // 1. A new panel.
    fs::create_dir_all(staging.join("shot-notes")).unwrap();
    fs::write(staging.join("shot-notes/manifest.json"), json!({"id":"shot-notes","name":"Shot Notes","version":"1.0.0","apiVersion":4,"entry":"index.tsx"}).to_string()).unwrap();
    fs::write(
        staging.join("shot-notes/index.tsx"),
        "export const view = () => <text>Shots reviewed: 0</text>;",
    )
    .unwrap();
    let published = call(
        &mut session,
        "publish_extensions",
        json!({"extensions":[{"id":"shot-notes","action":"created","summary":"Shot notes panel"}]}),
    );
    assert_eq!(stage(&published), staging);
    let (_, texts) = drawn(&live, "shot-notes");
    assert!(texts.contains(&"Shots reviewed: 0".to_owned()), "{texts:?}");

    // 2. The timeline with an extra column on every layer row.
    let forked = call(
        &mut session,
        "fork_builtin_extension",
        json!({"id":"timeline"}),
    );
    let panel = PathBuf::from(forked["dir"].as_str().unwrap()).join("panel.tsx");
    let source = fs::read_to_string(&panel).unwrap();
    let row = "<Layer row={row} send={send} />";
    assert!(source.contains(row));
    fs::write(
        &panel,
        source.replacen(
            row,
            &format!("<>{row}<text style={{text}}>{{`Take ${{row.depth + 1}}`}}</text></>"),
            1,
        ),
    )
    .unwrap();
    let published = call(
        &mut session,
        "publish_extensions",
        json!({"extensions":[{"id":"timeline-fork","action":"created","summary":"Take column"}]}),
    );
    assert_eq!(stage(&published), staging);
    let (panel, texts) = drawn(&live, "timeline-fork");
    assert_eq!(panel, "timeline");
    assert!(texts.contains(&"Take 1".to_owned()), "{texts:?}");

    // 3. A viewer overlay.
    call(
        &mut session,
        "fork_builtin_extension",
        json!({"id":"viewer"}),
    );
    let panel = staging.join("viewer-fork/panel.tsx");
    let source = fs::read_to_string(&panel).unwrap();
    let gizmos = "label=\"Transform gizmos\"\n          style={{ position: \"absolute\", inset: \"fill\" }}\n        />";
    assert!(source.contains(gizmos));
    fs::write(
        &panel,
        source.replacen(
            gizmos,
            &format!("{gizmos}\n        <text style={{{{ position: \"absolute\" }}}}>REC</text>"),
            1,
        ),
    )
    .unwrap();
    let published = call(
        &mut session,
        "publish_extensions",
        json!({"extensions":[{"id":"viewer-fork","action":"created","summary":"REC overlay"}]}),
    );
    let (panel, texts) = drawn(&live, "viewer-fork");
    assert_eq!(panel, "viewer");
    assert!(texts.contains(&"REC".to_owned()), "{texts:?}");
    call(&mut session, "finish_session", json!({"commit":true}));

    // 4. Undo Extension Change takes back only the newest: the overlay.
    let extensions = AppExtensions {
        profile: profile.clone(),
        roots: vec![live.clone()],
    };
    let project = host.lock().unwrap().active_project().unwrap();
    let newest =
        pm_mcp::extensions::change_sets(&pm_mcp::extensions::history_for(&extensions, &project));
    assert_eq!(newest[0]["id"], published["changeSetId"]);
    pm_mcp::extensions::restore(&extensions, &project, newest[0]["id"].as_str().unwrap()).unwrap();
    assert!(!live.join("viewer-fork").exists());
    assert!(live.join("timeline-fork").exists() && live.join("shot-notes").exists());
    let _ = fs::remove_dir_all(profile);
}
