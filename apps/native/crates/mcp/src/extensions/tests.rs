use super::*;
use std::sync::Mutex as StdMutex;

struct Host {
    roots: Vec<PathBuf>,
    history: PathBuf,
    builtins: PathBuf,
    reloads: StdMutex<Vec<Vec<String>>>,
}
impl ExtensionHost for Host {
    fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
    fn history(&self) -> PathBuf {
        self.history.clone()
    }
    fn builtins(&self) -> Option<PathBuf> {
        Some(self.builtins.clone())
    }
    fn manifest(&self, raw: &Value) -> std::result::Result<Value, String> {
        if raw["id"].as_str().is_none_or(|id| !extension_id(id)) {
            return Err("invalid \"id\" (a-z, 0-9, -; 2–64 chars)".into());
        }
        Ok(raw.clone())
    }
    fn check(&self, dir: &Path, manifest: &Value) -> std::result::Result<(), String> {
        let entry = manifest["entry"].as_str().unwrap_or("index.ts");
        let source = fs::read_to_string(dir.join(entry)).map_err(|_| format!("missing {entry}"))?;
        if source.contains("syntax error") {
            return Err("Unexpected token".into());
        }
        Ok(())
    }
    fn reload(&self, ids: &[String]) {
        self.reloads.lock().unwrap().push(ids.to_vec());
    }
    fn api_pack(&self) -> Vec<(String, String)> {
        vec![]
    }
}
struct Dir(PathBuf);
impl Dir {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Fixture {
    dir: Dir,
    host: Host,
}
impl Fixture {
    fn new() -> Self {
        let dir = Dir(std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-extensions")));
        let root = dir.path();
        let host = Host {
            roots: vec![root.join("shipped-app"), root.join("live")],
            history: root.join("history"),
            builtins: root.join("builtins"),
            reloads: StdMutex::default(),
        };
        Self { dir, host }
    }
    fn stage(&self, run: &str) -> Stage {
        prepare(&self.host, &self.dir.path().join("stage"), "project-a", run).unwrap()
    }
    fn live(&self) -> PathBuf {
        self.host.roots[1].clone()
    }
}
fn extension(dir: &Path, id: &str, body: &str) {
    fs::create_dir_all(dir.join(id)).unwrap();
    fs::write(
        dir.join(id).join("manifest.json"),
        json!({"id":id,"name":id,"version":"1.0.0","apiVersion":4,"entry":"index.tsx"}).to_string(),
    )
    .unwrap();
    fs::write(dir.join(id).join("index.tsx"), body).unwrap();
}
fn change(id: &str, action: &str) -> Value {
    json!({"id":id,"action":action,"summary":"synthetic"})
}

#[test]
fn publication_promotes_exactly_the_declared_changes_and_undo_restores_them() {
    let f = Fixture::new();
    extension(&f.live(), "kept", "export const v = 1;");
    let stage = f.stage("run-1");
    assert_eq!(stage.baseline.keys().collect::<Vec<_>>(), ["kept"]);
    extension(&stage.staging, "panel-a", "export function view() {}");
    fs::write(stage.staging.join("kept/index.tsx"), "export const v = 2;").unwrap();

    let error = publish(&f.host, &stage, &[change("panel-a", "created")]).unwrap_err();
    assert!(error.message.starts_with("The agent change report did not match its staged files. Nothing was applied. Actual staged extension changes: [{\"id\":\"kept\",\"action\":\"updated\"},{\"id\":\"panel-a\",\"action\":\"created\"}]"), "{}", error.message);
    let error = publish(
        &f.host,
        &stage,
        &[change("panel-a", "updated"), change("kept", "updated")],
    )
    .unwrap_err();
    assert!(
        error
            .message
            .starts_with("The agent reported panel-a as updated, but it was created."),
        "{}",
        error.message
    );
    assert!(!f.live().join("panel-a").exists());

    let record = publish(
        &f.host,
        &stage,
        &[change("panel-a", "created"), change("kept", "updated")],
    )
    .unwrap()
    .unwrap();
    assert_eq!(record["id"], "run-1");
    assert_eq!(
        fs::read_to_string(f.live().join("kept/index.tsx")).unwrap(),
        "export const v = 2;"
    );
    assert!(f.live().join("panel-a/index.tsx").exists());
    assert_eq!(
        *f.host.reloads.lock().unwrap(),
        [vec!["kept".to_owned(), "panel-a".to_owned()]]
    );
    assert_eq!(change_sets(&stage.history)[0]["id"], "run-1");

    // A second run publishes on top; only the newest can be undone first.
    let next = f.stage("run-2");
    fs::remove_dir_all(next.staging.join("panel-a")).unwrap();
    publish(&f.host, &next, &[change("panel-a", "removed")]).unwrap();
    assert!(!f.live().join("panel-a").exists());
    let error = restore(&f.host, "project-a", "run-1").unwrap_err();
    assert_eq!(
        error.message,
        "Newer extension changes exist. Restore newer change sets first."
    );
    restore(&f.host, "project-a", "run-2").unwrap();
    assert!(f.live().join("panel-a/index.tsx").exists());
    restore(&f.host, "project-a", "run-1").unwrap();
    assert!(!f.live().join("panel-a").exists());
    assert_eq!(
        fs::read_to_string(f.live().join("kept/index.tsx")).unwrap(),
        "export const v = 1;"
    );
    assert!(restore(&f.host, "project-a", "run-1").is_err());
    assert!(change_sets(&stage.history).is_empty());
}

#[test]
fn validation_failures_and_conflicts_publish_nothing() {
    let f = Fixture::new();
    extension(&f.live(), "kept", "export const v = 1;");
    let stage = f.stage("run-1");
    extension(&stage.staging, "broken", "syntax error");
    let error = publish(&f.host, &stage, &[change("broken", "created")]).unwrap_err();
    assert_eq!(
        error.message,
        "broken failed compilation: Unexpected token. Repair its staged source. Nothing was published."
    );
    fs::write(stage.staging.join("broken/manifest.json"), "{").unwrap();
    let error = publish(&f.host, &stage, &[change("broken", "created")]).unwrap_err();
    assert_eq!(
        error.message,
        "broken: provide a valid manifest.json in the current staging directory. Nothing was published."
    );
    fs::remove_dir_all(stage.staging.join("broken")).unwrap();

    let key = format!("sk-{}", "Q7w".repeat(8));
    extension(&stage.staging, "leaky", &format!("const k = \"{key}\";"));
    let error = publish(&f.host, &stage, &[change("leaky", "created")]).unwrap_err();
    assert_eq!(
        error.message,
        "leaky/index.tsx:1 looks like an OpenAI API key. Declare it in manifest vars and read api.vars. Nothing was published."
    );
    fs::remove_dir_all(stage.staging.join("leaky")).unwrap();

    fs::write(stage.staging.join("kept/index.tsx"), "export const v = 2;").unwrap();
    fs::write(f.live().join("kept/index.tsx"), "export const v = 3;").unwrap();
    let error = publish(&f.host, &stage, &[change("kept", "updated")]).unwrap_err();
    assert!(
        error
            .message
            .starts_with("Newer changes exist in these extensions: kept."),
        "{}",
        error.message
    );
    assert_eq!(
        fs::read_to_string(f.live().join("kept/index.tsx")).unwrap(),
        "export const v = 3;"
    );
    assert!(f.host.reloads.lock().unwrap().is_empty());
    // Snapshots are app-owned scratch and never outlive a call.
    assert!(fs::read_dir(&stage.history).unwrap().next().is_none());
}

#[test]
fn an_update_shadows_the_shipped_app_folder_and_undo_brings_it_back() {
    let f = Fixture::new();
    extension(&f.host.roots[0], "old-tool", "export const v = 'electron';");
    let stage = f.stage("run-1");
    fs::write(
        stage.staging.join("old-tool/index.tsx"),
        "export const v = 'native';",
    )
    .unwrap();
    publish(&f.host, &stage, &[change("old-tool", "updated")]).unwrap();
    assert_eq!(
        fs::read_to_string(f.live().join("old-tool/index.tsx")).unwrap(),
        "export const v = 'native';"
    );
    restore(&f.host, "project-a", "run-1").unwrap();
    assert!(!f.live().join("old-tool").exists());
    assert!(f.host.roots[0].join("old-tool").exists());
    let stage = f.stage("run-2");
    fs::remove_dir_all(stage.staging.join("old-tool")).unwrap();
    let error = publish(&f.host, &stage, &[change("old-tool", "removed")]).unwrap_err();
    assert!(
        error.message.contains("cannot be removed here"),
        "{}",
        error.message
    );
}

#[test]
fn an_interrupted_promotion_rolls_back_before_the_next_stage() {
    let f = Fixture::new();
    extension(&f.live(), "kept", "export const v = 1;");
    let pending = f.host.history.join("project-a/.pending-run-9-x");
    fs::create_dir_all(pending.join("before")).unwrap();
    fs::create_dir_all(pending.join("next")).unwrap();
    // The crash happened after `kept` moved to `before` and the new copy moved in.
    fs::rename(f.live().join("kept"), pending.join("before/kept")).unwrap();
    extension(&f.live(), "kept", "export const v = 2;");
    extension(&f.live(), "added", "export const v = 1;");
    fs::write(
        pending.join(RECORD),
        json!({"changes":[change("kept","updated"),change("added","created")]}).to_string(),
    )
    .unwrap();
    let stage = f.stage("run-1");
    assert_eq!(
        fs::read_to_string(f.live().join("kept/index.tsx")).unwrap(),
        "export const v = 1;"
    );
    assert!(!f.live().join("added").exists());
    assert!(!pending.exists());
    assert_eq!(stage.baseline.keys().collect::<Vec<_>>(), ["kept"]);
}

#[test]
fn forks_copy_a_pristine_base_and_rebases_merge_upstream_changes() {
    let f = Fixture::new();
    let builtin = f.host.builtins.join("timeline");
    fs::create_dir_all(builtin.join("src")).unwrap();
    let manifest = |version: &str| {
        json!({"id":"timeline","name":"Timeline","version":version,"apiVersion":4,"entry":"index.tsx","author":"powermove"}).to_string()
    };
    fs::write(builtin.join("manifest.json"), manifest("1.0.0")).unwrap();
    fs::write(builtin.join("index.tsx"), "a\nb\nc\nd\ne\n").unwrap();
    fs::write(builtin.join("src/rows.tsx"), "row\n").unwrap();
    let stage = f.stage("run-1");
    let forked = fork(&f.host, "timeline", None, &stage.staging).unwrap();
    assert_eq!(forked["forkId"], "timeline-fork");
    assert_eq!(forked["forkedFrom"], "timeline@1.0.0");
    let dir = stage.staging.join("timeline-fork");
    let written: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(written["replaces"], json!(["timeline"]));
    assert_eq!(written["author"], "powermove");
    assert_eq!(
        fs::read(dir.join(".forked-from/manifest.json")).unwrap(),
        manifest("1.0.0").into_bytes()
    );
    assert!(fork(&f.host, "timeline", None, &stage.staging).is_err());
    fs::write(dir.join("index.tsx"), "a\nB\nc\nd\ne\n").unwrap();
    publish(&f.host, &stage, &[change("timeline-fork", "created")]).unwrap();

    // The app ships timeline 1.1.0: one clean upstream edit, one conflicting file.
    fs::write(builtin.join("manifest.json"), manifest("1.1.0")).unwrap();
    fs::write(builtin.join("index.tsx"), "a\nb\nc\nD\ne\n").unwrap();
    let stage = f.stage("run-2");
    let rebased = rebase(&f.host, "timeline-fork", &stage.staging).unwrap();
    assert_eq!(rebased["conflicts"], json!([]));
    assert_eq!(
        rebased["mergedFiles"],
        json!(["index.tsx", "manifest.json"])
    );
    assert_eq!(rebased["base"], "1.0.0");
    assert_eq!(rebased["current"], "1.1.0");
    assert_eq!(
        fs::read_to_string(dir.join("index.tsx")).unwrap(),
        "a\nB\nc\nD\ne\n"
    );
    let merged: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(merged["forkedFrom"], "timeline@1.1.0");
    assert_eq!(merged["id"], "timeline-fork");
    assert_eq!(
        fs::read_to_string(dir.join(".forked-from/index.tsx")).unwrap(),
        "a\nb\nc\nD\ne\n"
    );
    assert_eq!(
        fs::read_to_string(stage.staging.join(".rebase/timeline-fork/base/index.tsx")).unwrap(),
        "a\nb\nc\nd\ne\n"
    );
    // The live fork is untouched until the agent publishes the rebase.
    assert_eq!(
        fs::read_to_string(f.live().join("timeline-fork/index.tsx")).unwrap(),
        "a\nB\nc\nd\ne\n"
    );

    fs::write(builtin.join("index.tsx"), "a\nX\nc\nd\ne\n").unwrap();
    let stage = f.stage("run-3");
    let rebased = rebase(&f.host, "timeline-fork", &stage.staging).unwrap();
    assert_eq!(rebased["conflicts"], json!(["index.tsx"]));
    assert_eq!(
        fs::read_to_string(stage.staging.join("timeline-fork/index.tsx")).unwrap(),
        "a\nB\nc\nd\ne\n"
    );
    assert_eq!(
        rebase(&f.host, "timeline", &stage.staging)
            .unwrap_err()
            .message,
        "Only user extensions can be rebased: timeline"
    );
}
