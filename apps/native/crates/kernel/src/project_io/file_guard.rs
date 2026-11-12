//! Conditional legacy-reader requirement, derived from the frozen save state.
//!
//! N3/N4: preserve authored data and opaque fields, including Undo/Redo. This is
//! metadata for the proposed transition reader, NOT protection in v1.2.0/main:
//! the audited shipped readers ignore unknown fields. See docs/native/file-guard.md.

use std::borrow::Cow;

use pm_model::json::write::{JsonTree, View};
use pm_model::{Map, Node, Value};

use crate::history::History;

const MARKER: &str = "requiresEngine";
const NATIVE: &str = "native";

fn field<'a, T: JsonTree>(value: &'a T, key: &str) -> Option<&'a T> {
    match value.view() {
        View::Object(map) => map.get(key.as_bytes()),
        _ => None,
    }
}

fn text<T: JsonTree>(value: Option<&T>) -> Option<&str> {
    match value?.view() {
        View::String(s) => s.as_str(),
        _ => None,
    }
}

fn present<T: JsonTree>(value: Option<&T>) -> bool {
    value.is_some_and(|v| !matches!(v.view(), View::Null))
}

fn is_true<T: JsonTree>(value: Option<&T>) -> bool {
    value.is_some_and(|v| matches!(v.view(), View::Bool(true)))
}

fn has_any<T: JsonTree>(value: &T, keys: &[&str]) -> bool {
    // The old 3D Zod schemas are strict: even an explicit default in a NEW
    // field is unrepresentable. Do not normalize it away to pass this guard.
    keys.iter().any(|key| field(value, key).is_some())
}

fn studio_object<T: JsonTree>(object: &T) -> bool {
    text(field(object, "source").and_then(|s| field(s, "primitive"))) == Some("roundedBox")
        || field(object, "material").is_some_and(|material| {
            has_any(material, &["screen"])
                || field(material, "p").is_some_and(|p| {
                    has_any(
                        p,
                        &[
                            "clearcoat",
                            "clearcoatRoughness",
                            "transmission",
                            "ior",
                            "specular",
                            "sheen",
                            "iridescence",
                            "dispersion",
                            "droplets",
                            "anisotropy",
                            "subsurface",
                        ],
                    )
                })
        })
}

fn studio_light<T: JsonTree>(light: &T) -> bool {
    text(field(light, "type")) == Some("area")
        || field(light, "p").is_some_and(|p| has_any(p, &["width", "height"]))
}

fn studio_camera<T: JsonTree>(camera: &T) -> bool {
    field(camera, "p").is_some_and(|p| has_any(p, &["fStop", "focusDistance"]))
}

fn studio_environment<T: JsonTree>(environment: &T) -> bool {
    has_any(environment, &["shadowCatcher", "samples"])
}

fn any_array<T: JsonTree>(value: Option<&T>, test: impl Fn(&T) -> bool) -> bool {
    match value.map(JsonTree::view) {
        Some(View::Array(items)) => items.iter().any(test),
        _ => false,
    }
}

fn layer_needs_native<T: JsonTree>(layer: &T) -> bool {
    // An upgraded layer carries the sole archived authored source. Older
    // hydration/save paths do not implement that archive/diagnostic contract.
    if present(field(layer, "native3dArchive")) || present(field(layer, "native3dUpgradeIssue")) {
        return true;
    }
    let kind = text(field(layer, "type"));
    // Exact TYPE_META roster in BOTH pinned readers. Missing/null types already
    // mean null in those readers. An extension definition is not a layer type:
    // arbitrary v1–3 definitions remain supported through `extension`.
    if present(field(layer, "type")) && kind.is_none()
        || kind.is_some_and(|kind| {
            !matches!(
                kind,
                "solid"
                    | "text"
                    | "shape"
                    | "image"
                    | "video"
                    | "audio"
                    | "adjustment"
                    | "shader"
                    | "extension"
                    | "null"
                    | "group"
                    | "precomp"
                    | "captions"
            )
        })
    {
        return true;
    }
    let Some(content) = field(layer, "d") else {
        return false;
    };
    if matches!(kind, Some("audio" | "video")) && text(field(content, "pitch")) == Some("follow")
        || kind == Some("precomp") && is_true(field(content, "keepFrameRate"))
    {
        return true;
    }
    if kind == Some("captions") && present(field(content, "nativeCaptionArchive")) {
        return true;
    }
    if kind != Some("extension") {
        return false;
    }
    let Some(data) = field(content, "data") else {
        return false;
    };
    // Deliberately do not inspect arbitrary extension-owned JSON.
    match text(field(content, "definition")) {
        Some("powermove.3d.object") => field(data, "object").is_some_and(studio_object),
        Some("powermove.3d.light") => field(data, "light").is_some_and(studio_light),
        Some("powermove.3d.camera") => {
            field(data, "camera").is_some_and(studio_camera)
                || field(data, "environment").is_some_and(studio_environment)
        }
        Some("powermove.3d.scene") => field(data, "scene").is_some_and(|scene| {
            any_array(field(scene, "objects"), studio_object)
                || any_array(field(scene, "lights"), studio_light)
                || field(scene, "camera").is_some_and(studio_camera)
                || field(scene, "environment").is_some_and(studio_environment)
        }),
        _ => false,
    }
}

fn project_needs_native<T: JsonTree>(root: &T) -> bool {
    // Iterative, scoped traversal: no recursion into extension data, workspace,
    // notes, edits or other opaque members. Canonical roots include the open
    // comp in `comps`; stored roots hold its fields directly. Accept both.
    let mut pending = vec![root];
    while let Some(project) = pending.pop() {
        // Existing declarations are authoritative, including when they occur
        // only in an Undo/Redo state whose feature this build does not know.
        let newer_reader =
            field(project, "minReaderVersion").is_some_and(|value| match value.view() {
                View::Null => false,
                View::Number(n) => !n.is_finite() || n > 1.0,
                _ => true,
            });
        if present(field(project, MARKER))
            || present(field(project, "nativeMediaVersions"))
            || newer_reader
            || any_array(field(project, "layers"), layer_needs_native)
        {
            return true;
        }
        if let Some(View::Object(assets)) = field(project, "assets").map(JsonTree::view)
            && assets.iter().any(|(_, asset)| {
                text(field(asset, "storage")) == Some("linked")
                    || present(field(asset, "link"))
                    || present(field(asset, "interpret"))
            })
        {
            return true;
        }
        if field(project, "color").is_some_and(|color| {
            is_true(field(color, "linearBlending"))
                || text(field(color, "blendDomain")) == Some("displayEncoded")
                || text(field(color, "mode")) == Some("hdr")
        }) {
            return true;
        }
        if let Some(View::Object(comps)) = field(project, "comps").map(JsonTree::view) {
            pending.extend(comps.iter().map(|(_, comp)| comp));
        }
    }
    false
}

fn history_needs_native(root: Option<&Node>, saved: &Value) -> bool {
    if matches!(saved, Value::Null) {
        return false;
    }
    let Some(root) = root else { return true };
    let Ok(history) = History::import(saved, text(field(root, "compId"))) else {
        // A reader that drops unimportable history cannot represent this file.
        // Keep the exact raw history; never erase it to remove the requirement.
        return true;
    };
    let source = saved.get("history").unwrap_or(saved);
    let entries = source.get("entries").and_then(Value::as_array);
    if entries.is_some_and(|entries| entries.len() != history.records().len()) {
        // Import can trim history to its budget. A partial inventory is not a
        // proof of legacy readability, including for raw preserved dictionaries.
        return true;
    }
    // Large edits save as id-keyed operations. The shipped reader's
    // `validPatches` refuses them, dropping the file's whole history.
    let holds_operations = |entry: &Value| {
        ["forward", "backward"].iter().any(|side| {
            entry
                .get(side)
                .and_then(Value::as_array)
                .is_some_and(|ops| {
                    ops.iter().any(|op| {
                        let op = op.get("patch").unwrap_or(op);
                        op.get("op").and_then(Value::as_str).is_some()
                    })
                })
        })
    };
    if entries.is_some_and(|entries| entries.iter().any(holds_operations)) {
        return true;
    }
    // Reconstruct each reachable state, not isolated patch values: a leaf
    // patch can change a material without carrying its layer/definition.
    for undo in [true, false] {
        let mut state = root.clone();
        let records = history.records();
        let steps: Box<dyn Iterator<Item = _>> = if undo {
            Box::new(records[..history.applied()].iter().rev())
        } else {
            Box::new(records[history.applied()..].iter())
        };
        for record in steps {
            let ops = if undo {
                &record.inverse.to_vec()
            } else {
                &record.forward.to_vec()
            };
            let mut replay = pm_model::Document::from_root(Some(state), root.rev());
            if replay.apply_ops_at(&ops, root.rev() + 1).is_err() {
                return true;
            }
            state = replay.root().expect("replayed project").clone();
            if project_needs_native(&state) {
                return true;
            }
        }
    }
    false
}

/// Recompute only the writer-owned envelope stamp. Root declarations and
/// unknown envelope requirements are never removed or overwritten (N4).
/// Borrow the original extras for ordinary projects; don't copy a large tree.
pub(super) fn save_extra<'a>(root: Option<&Node>, history: &Value, extra: &'a Map) -> Cow<'a, Map> {
    let needed = root.is_some_and(project_needs_native) || history_needs_native(root, history);
    let ours = extra.get(MARKER).and_then(Value::as_str) == Some(NATIVE);
    let absent = extra.get(MARKER).is_none_or(|v| matches!(v, Value::Null));
    if needed && absent {
        let mut out = extra.clone();
        out.insert(MARKER, Value::from(NATIVE));
        Cow::Owned(out)
    } else if !needed && ours {
        let mut out = extra.clone();
        out.remove(MARKER);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(extra)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_io::{
        OpenOptions, SaveOptions, SaveTarget, WriteFault, open_project,
        save::{SaveLayout, SavePlan},
    };
    use crate::{DocumentStore, Origin};
    use pm_model::container::{ReadOptions, read_container};
    use pm_model::json::{parse, to_canonical_string};
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            // macOS /tmp and /var are system symlinks. Test only canonical roots.
            let root = std::env::temp_dir().canonicalize().unwrap();
            let path = root.join(format!(
                "pm-fileguard-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn project(layer: &str) -> Value {
        parse(&format!(
            r#"{{"id":"P","compId":"C","w":640,"h":360,"fps":23.976,"dur":5,"assets":{{}},"layers":[{layer}]}}"#
        ))
        .unwrap()
    }
    fn extension(definition: &str, data: &str) -> String {
        format!(
            r#"{{"id":"L","type":"extension","d":{{"definition":"{definition}","version":1,"data":{data}}}}}"#
        )
    }
    fn plan(store: &mut DocumentStore, path: &Path) -> SavePlan {
        store
            .prepare_save(
                SaveTarget::SaveAs {
                    path: path.to_owned(),
                    replace: false,
                },
                SaveOptions::default(),
            )
            .unwrap()
    }
    fn run(store: &mut DocumentStore, plan: SavePlan) -> SaveLayout {
        let receipt = std::thread::spawn(move || plan.run())
            .join()
            .unwrap()
            .unwrap();
        let layout = receipt.layout;
        store.finish_save(receipt).unwrap();
        layout
    }
    fn document(path: &Path) -> Value {
        read_container(&File::open(path).unwrap(), ReadOptions::default())
            .unwrap()
            .document
    }
    fn save(dir: &TempDir, name: &str, project: Value, history: Value) -> PathBuf {
        let path = dir.path(name);
        let mut store = DocumentStore::new_untitled(project);
        let mut plan = plan(&mut store, &path);
        plan.history = super::super::HistoryToWrite::Raw(history);
        let before = to_canonical_string(store.root().unwrap());
        run(&mut store, plan);
        assert_eq!(to_canonical_string(store.root().unwrap()), before);
        let reopened = DocumentStore::open(open_project(&path, OpenOptions::default()).unwrap());
        assert_eq!(to_canonical_string(reopened.root().unwrap()), before);
        path
    }
    fn assert_marked(path: &Path, expected: bool) {
        let doc = document(path);
        assert_eq!(
            doc.get(MARKER).and_then(Value::as_str),
            expected.then_some(NATIVE),
            "{}",
            path.display()
        );
    }

    // The executable transition proposal is tested as written, rather than maintaining an untested second implementation.
    const PROPOSAL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../docs/native/file-guard.md"
    ));

    fn oracle(cases: &[(PathBuf, bool)]) {
        let proposal = PROPOSAL
            .split("<!-- transition-guard:start -->")
            .nth(1)
            .unwrap()
            .split("```js\n")
            .nth(1)
            .unwrap()
            .split("\n```")
            .next()
            .unwrap();
        let request = serde_json::json!({
            "cases": cases.iter().map(|(path, guarded)| serde_json::json!({
                "path": path, "guarded": guarded
            })).collect::<Vec<_>>(),
            "incremental": FROZEN_INCREMENTAL_TS,
            "container": FROZEN_CONTAINER_DECODE_TS,
            "validator": FROZEN_VALIDATOR_TS,
            "hydration": FROZEN_HYDRATION_TS,
            "proposal": proposal,
        });
        let runtime = std::env::var_os("PM_TEST_JS_RUNTIME").unwrap_or_else(|| "node".into());
        let mut child = Command::new(runtime)
            .args(["--input-type=module", "-e", ORACLE_JS])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Node 22.18+ or Bun is required for extracted reader tests");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }

    #[test]
    fn plain_projects_all_legacy_types_and_opaque_fields_stay_readable() {
        let dir = TempDir::new();
        let mut cases = Vec::new();
        for kind in [
            "solid",
            "text",
            "shape",
            "image",
            "video",
            "audio",
            "adjustment",
            "shader",
            "extension",
            "null",
            "group",
            "precomp",
            "captions",
        ] {
            let mut p = project(&format!(r#"{{"id":"L","type":"{kind}"}}"#));
            p.as_object_mut().unwrap().insert(
                "futureField",
                parse(r#"{"exact":0.1234567890123456,"text":"🧭 é","requiresEngine":"opaque"}"#)
                    .unwrap(),
            );
            let path = save(&dir, &format!("{kind}.pmv"), p, Value::Null);
            assert_marked(&path, false);
            cases.push((path, false));
        }
        let layer = extension(
            "third.party.layer",
            r#"{"type":"future-layer","object":{"material":{"p":{"clearcoat":1}}},"requiresEngine":"not-project-metadata"}"#,
        );
        let path = save(&dir, "opaque-extension.pmv", project(&layer), Value::Null);
        assert_marked(&path, false);
        cases.push((path, false));
        oracle(&cases);
    }

    #[test]
    fn actual_native_features_are_marked_and_shipped_readers_ignore_the_marker() {
        let dir = TempDir::new();
        let mut cases = Vec::new();
        let mut projects = vec![
            project(
                r#"{"id":"L","type":"future-layer","p":{"future.channel":{"v":12}},"d":{"kept":true}}"#,
            ),
            project(r#"{"id":"L","type":123}"#),
            project(r#"{"id":"L","type":"video","d":{"pitch":"follow"}}"#),
            project(r#"{"id":"L","type":"audio","d":{"pitch":"follow"}}"#),
            project(r#"{"id":"L","type":"precomp","d":{"keepFrameRate":true}}"#),
        ];
        for asset in [
            r#"{"storage":"linked"}"#,
            r#"{"link":{}}"#,
            r#"{"interpret":{"fps":24}}"#,
        ] {
            let mut p = project(r#"{"id":"L","type":"image"}"#);
            p.as_object_mut()
                .unwrap()
                .insert("assets", parse(&format!(r#"{{"A":{asset}}}"#)).unwrap());
            projects.push(p);
        }
        for color in [
            r#"{"linearBlending":true}"#,
            r#"{"blendDomain":"displayEncoded"}"#,
            r#"{"mode":"hdr"}"#,
        ] {
            let mut p = project(r#"{"id":"L","type":"shape"}"#);
            p.as_object_mut()
                .unwrap()
                .insert("color", parse(color).unwrap());
            projects.push(p);
        }
        for (i, p) in projects.into_iter().enumerate() {
            let path = save(&dir, &format!("feature-{i}.pmv"), p, Value::Null);
            assert_marked(&path, true);
            cases.push((path, true));
        }
        oracle(&cases);
    }

    #[test]
    fn raw_legacy_3d_stays_readable_but_automatic_upgrade_archives_require_native() {
        let dir = TempDir::new();
        let mut cases = Vec::new();
        for (i, (definition, data)) in [
            (
                "powermove.3d.object",
                r#"{"object":{"source":{"primitive":"box"},"material":{"p":{"roughness":0.4}}}}"#,
            ),
            ("powermove.3d.light", r#"{"light":{"type":"sun"}}"#),
            (
                "powermove.3d.camera",
                r#"{"camera":{"p":{"fov":45}},"environment":{"shadows":true}}"#,
            ),
            (
                "powermove.3d.scene",
                r#"{"scene":{"version":1,"objects":[],"lights":[]}}"#,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut p = project(&extension(definition, data));
            p.as_object_mut().unwrap().insert(
                "color",
                parse(r#"{"linearBlending":false,"blendDomain":"hdrSafe"}"#).unwrap(),
            );
            // Raw legacy 3D and its old defaults alone are not a requirement.
            assert!(!project_needs_native(&p));
            let original = p.get("layers").unwrap().as_array().unwrap()[0].clone();
            let path = save(&dir, &format!("legacy-3d-{i}.pmv"), p, Value::Null);
            assert_marked(&path, true);
            let saved = document(&path);
            let archived = saved
                .get("proj")
                .unwrap()
                .get("layers")
                .unwrap()
                .as_array()
                .unwrap()[0]
                .get("native3dArchive")
                .unwrap()
                .get("source")
                .unwrap();
            assert_eq!(archived, &original);
            cases.push((path, true));
        }
        oracle(&cases);
    }

    #[test]
    fn studio_schema_additions_require_native_even_when_explicit_defaults() {
        let dir = TempDir::new();
        let mut cases = Vec::new();
        let mut layers = vec![
            extension(
                "powermove.3d.object",
                r#"{"object":{"source":{"primitive":"roundedBox"}}}"#,
            ),
            extension(
                "powermove.3d.object",
                r#"{"object":{"material":{"screen":null}}}"#,
            ),
            extension("powermove.3d.light", r#"{"light":{"type":"area"}}"#),
            extension(
                "powermove.3d.light",
                r#"{"light":{"p":{"width":2,"height":1.2}}}"#,
            ),
            extension(
                "powermove.3d.camera",
                r#"{"camera":{"p":{"fStop":0,"focusDistance":0}}}"#,
            ),
            extension(
                "powermove.3d.camera",
                r#"{"environment":{"shadowCatcher":false}}"#,
            ),
            extension("powermove.3d.camera", r#"{"environment":{"samples":64}}"#),
            extension(
                "powermove.3d.scene",
                r#"{"scene":{"objects":[{"material":{"screen":"C"}}]}}"#,
            ),
        ];
        for key in [
            "clearcoat",
            "clearcoatRoughness",
            "transmission",
            "ior",
            "specular",
            "sheen",
            "iridescence",
            "dispersion",
            "droplets",
            "anisotropy",
            "subsurface",
        ] {
            layers.push(extension(
                "powermove.3d.object",
                &format!(r#"{{"object":{{"material":{{"p":{{"{key}":{{"v":0,"kf":[]}}}}}}}}}}"#),
            ));
        }
        for (i, layer) in layers.iter().enumerate() {
            let path = save(
                &dir,
                &format!("studio-{i}.pmv"),
                project(layer),
                Value::Null,
            );
            assert_marked(&path, true);
            cases.push((path, true));
        }
        oracle(&cases);
    }

    #[test]
    fn unopened_and_deep_compositions_are_inventoried() {
        let dir = TempDir::new();
        let mut p = project(r#"{"id":"L","type":"shape"}"#);
        let nested = project(r#"{"id":"new","type":"future-layer"}"#);
        p.as_object_mut().unwrap().insert(
            "comps",
            parse(&format!(
                r#"{{"other":{{"id":"other","w":640,"h":360,"layers":[],"comps":{{"deep":{}}}}}}}"#,
                to_canonical_string(&nested)
            ))
            .unwrap(),
        );
        let path = save(&dir, "nested.pmv", p, Value::Null);
        assert_marked(&path, true);
        oracle(&[(path, true)]);
    }

    #[test]
    fn partial_undo_redo_dictionary_and_root_relative_history_are_guarded() {
        let dir = TempDir::new();
        let p = project(&extension(
            "powermove.3d.object",
            r#"{"object":{"material":{"p":{}}}}"#,
        ));
        let mut cases = Vec::new();
        for (i, (paths, prefix, index, forward, backward)) in [
            ("composition", r#""comps","C","#, 0, "false", "true"),
            ("composition", r#""comps","C","#, -1, "true", "false"),
            ("root", "", -1, "true", "false"),
        ]
        .into_iter()
        .enumerate()
        {
            let patch = |exists| {
                format!(
                    r#"{{"path":[{prefix}"layers",0,"d","data","object","material","p","clearcoat"],"exists":{exists},"value":{{"v":0.5,"kf":[]}}}}"#
                )
            };
            let history = parse(&format!(r#"{{"version":1,"paths":"{paths}","index":{index},"entries":[{{"label":"Studio channel","forward":[{}],"backward":[{}]}}]}}"#,patch(forward),patch(backward))).unwrap();
            let path = save(
                &dir,
                &format!("history-{i}.pmv"),
                p.clone(),
                history.clone(),
            );
            assert_eq!(document(&path).get("history"), Some(&history));
            assert_marked(&path, true);
            cases.push((path, true));
        }
        let history = parse(r#"{"version":1,"paths":"composition","index":-1,"entries":[{"label":"Future layer","forward":[{"path":["comps","C","layers"],"exists":true,"value":[{"id":"future","type":"future-layer"}]}],"backward":[{"path":["comps","C","layers"],"exists":true,"value":[]}]}]}"#).unwrap();
        let packed = crate::history::pack(history);
        let path = save(&dir, "dictionary.pmv", p, packed.clone());
        assert_eq!(document(&path).get("history"), Some(&packed));
        assert_marked(&path, true);
        cases.push((path, true));
        oracle(&cases);
    }

    #[test]
    fn reachable_requirement_declarations_and_failed_history_replay_stay_guarded() {
        let dir = TempDir::new();
        let mut cases = Vec::new();
        for (i, (path, value)) in [
            (r#"["requiresEngine"]"#, r#""future-engine""#),
            (r#"["minReaderVersion"]"#, "2"),
            (r#"["comps","C","w","invalid"]"#, "1"),
        ]
        .into_iter()
        .enumerate()
        {
            let history = parse(&format!(r#"{{"version":1,"paths":"composition","index":-1,"entries":[{{"label":"Future requirement","forward":[{{"path":{path},"exists":true,"value":{value}}}],"backward":[]}}]}}"#)).unwrap();
            let output = save(
                &dir,
                &format!("declaration-{i}.pmv"),
                project(r#"{"id":"L","type":"shape"}"#),
                history.clone(),
            );
            assert_marked(&output, true);
            assert_eq!(document(&output).get("history"), Some(&history));
            cases.push((output, true));
        }
        oracle(&cases);
    }

    #[test]
    fn unimportable_history_and_foreign_requirements_survive() {
        let dir = TempDir::new();
        let history = parse(r#"{"version":99,"futureUndo":{"kept":true}}"#).unwrap();
        let path = save(
            &dir,
            "unknown-history.pmv",
            project(r#"{"id":"L","type":"shape"}"#),
            history.clone(),
        );
        assert_eq!(document(&path).get("history"), Some(&history));
        assert_marked(&path, true);
        oracle(&[(path, true)]);
        let root = Node::from_value(project(r#"{"id":"L","type":"future-layer"}"#), 1);
        for requirement in [
            Value::from("future-engine"),
            Value::Bool(false),
            Value::Number(3.0),
            parse(r#"{"opaque":true}"#).unwrap(),
        ] {
            let mut extra = Map::new();
            extra.insert(MARKER, requirement.clone());
            assert_eq!(
                save_extra(Some(&root), &Value::Null, &extra).get(MARKER),
                Some(&requirement)
            );
        }
        let mut p = project(r#"{"id":"L","type":"shape"}"#);
        p.as_object_mut()
            .unwrap()
            .insert(MARKER, Value::from("future-engine"));
        let path = save(&dir, "root-requirement.pmv", p, Value::Null);
        assert_eq!(
            document(&path).get("proj").unwrap().get(MARKER),
            Some(&Value::from("future-engine"))
        );
        oracle(&[(path, true)]);
    }

    #[test]
    fn append_compaction_recompute_only_our_stamp_and_failures_keep_prior_bytes() {
        let dir = TempDir::new();
        let path = save(
            &dir,
            "project.pmv",
            project(r#"{"id":"L","type":"future-layer"}"#),
            Value::Null,
        );
        let mut store = DocumentStore::open(open_project(&path, OpenOptions::default()).unwrap());
        store
            .restore_project(
                project(r#"{"id":"L","type":"shape"}"#),
                "Replace",
                Origin::Interface,
            )
            .unwrap();
        let retained = store
            .prepare_save(SaveTarget::InPlace, SaveOptions::default())
            .unwrap();
        run(&mut store, retained);
        assert_marked(&path, true);
        let before = fs::read(&path).unwrap();
        let mut failed = store
            .prepare_save(
                SaveTarget::InPlace,
                SaveOptions {
                    fault: Some(WriteFault::FailBeforePublish),
                    ..SaveOptions::default()
                },
            )
            .unwrap();
        // Explicitly clear history in the frozen plan. Otherwise Undo of the
        // replace still needs the original future layer and must keep its guard.
        failed.history = super::super::HistoryToWrite::Raw(Value::Null);
        assert!(
            std::thread::spawn(move || failed.run())
                .join()
                .unwrap()
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut append = store
            .prepare_save(SaveTarget::InPlace, SaveOptions::default())
            .unwrap();
        append.history = super::super::HistoryToWrite::Raw(Value::Null);
        assert_eq!(run(&mut store, append), SaveLayout::Appended);
        assert_marked(&path, false);
        let mut compact = store
            .prepare_save(
                SaveTarget::InPlace,
                SaveOptions {
                    compact_min_dead_bytes: Some(0),
                    ..SaveOptions::default()
                },
            )
            .unwrap();
        compact.history = super::super::HistoryToWrite::Raw(Value::Null);
        assert_eq!(run(&mut store, compact), SaveLayout::Compacted);
        assert_marked(&path, false);
        oracle(&[(path, false)]);
    }

    #[test]
    fn save_plan_freezes_requirement_and_workspace_is_not_feature_data() {
        let dir = TempDir::new();
        let path = dir.path("frozen.pmv");
        let mut store = DocumentStore::new_untitled(project(r#"{"id":"L","type":"shape"}"#));
        let workspace =
            parse(r#"{"layers":[{"type":"future-layer"}],"assets":{"A":{"storage":"linked"}}}"#)
                .unwrap();
        store.set_workspace(Some(workspace.clone()));
        let frozen = plan(&mut store, &path);
        store
            .restore_project(
                project(r#"{"id":"L","type":"future-layer"}"#),
                "Replace",
                Origin::Interface,
            )
            .unwrap();
        run(&mut store, frozen);
        assert_marked(&path, false);
        assert_eq!(document(&path).get("ws"), Some(&workspace));
        assert!(store.is_dirty());
        oracle(&[(path, false)]);
    }

    // Vendored, unchanged reader/validator extracts, identical at the audited
    // release references. Hydration uses the five exact decisive statements above its
    // source boundary; it does not claim to emulate the full renderer.
    const FROZEN_INCREMENTAL_TS: &str = r####"/** PMV4 keeps immutable media at stable offsets and publishes a complete tail index.
 * Writers build a sibling clone and atomically replace the file; live files are
 * never appended in place. Unreachable records are removed by compaction. */
export const INCREMENTAL_MAGIC = new TextEncoder().encode('PMV4\n');
const END_MAGIC = new TextEncoder().encode('PMV4END\n');
export const PROJECT_FOOTER_BYTES = 64;
const MAX_INDEX_BYTES = 64 * 1024 * 1024;
export interface ProjectRecord { offset: number; length: number; sha256: string }
export interface IncrementalMedia extends ProjectRecord { id: string; revision: string; type: string }
export interface ProjectIndex { version: 4; document: ProjectRecord; media: IncrementalMedia[] }
export interface SaveMedia { id: string; revision: string; type: string; length: number }
export const isIncrementalProject = (bytes: Uint8Array): boolean => INCREMENTAL_MAGIC.every((byte, index) => bytes[index] === byte);
const HASH = /^[a-f0-9]{64}$/;

export function projectFooter(offset: number, length: number, digest: string): Uint8Array {
  if (!Number.isSafeInteger(offset) || offset < 5 || !Number.isSafeInteger(length) || length < 2 || length > MAX_INDEX_BYTES || !HASH.test(digest)) throw new Error('Invalid project index');
  const footer = new Uint8Array(PROJECT_FOOTER_BYTES), view = new DataView(footer.buffer);
  footer.set(END_MAGIC); view.setFloat64(8, offset, true); view.setUint32(16, length, true);
  for (let i = 0; i < 32; i++) footer[32 + i] = parseInt(digest.slice(i * 2, i * 2 + 2), 16);
  return footer;
}
export function readProjectFooter(footer: Uint8Array, size: number): ProjectRecord {
  if (footer.length !== PROJECT_FOOTER_BYTES || !END_MAGIC.every((b, i) => footer[i] === b)) throw new Error('The project commit is truncated or invalid.');
  const view = new DataView(footer.buffer, footer.byteOffset, footer.byteLength);
  const offset = view.getFloat64(8, true), length = view.getUint32(16, true);
  if (!Number.isSafeInteger(size) || !Number.isSafeInteger(offset) || offset < 5 || length < 2 || length > MAX_INDEX_BYTES || offset + length !== size - PROJECT_FOOTER_BYTES) throw new Error('The project index is invalid.');
  return { offset, length, sha256: Array.from(footer.subarray(32), b => b.toString(16).padStart(2, '0')).join('') };
}
export function validateProjectIndex(value: any, indexOffset: number): ProjectIndex {
  const validRange = (r: any) => r && Number.isSafeInteger(r.offset) && r.offset >= 5 && Number.isSafeInteger(r.length) && r.length >= 0
    && r.offset <= indexOffset && r.length <= indexOffset - r.offset && typeof r.sha256 === 'string' && HASH.test(r.sha256);
  if (value?.version !== 4 || !validRange(value.document) || value.document.length < 2 || !Array.isArray(value.media) || value.media.length > 100_000) throw new Error('The project index is invalid.');
  const ids = new Set<string>();
  for (const item of value.media) {
    if (!validRange(item) || typeof item.id !== 'string' || !item.id || item.id.length > 1000 || ids.has(item.id)
      || typeof item.revision !== 'string' || !item.revision || item.revision.length > 200 || typeof item.type !== 'string' || item.type.length > 1000) throw new Error('The project media index is invalid.');
    ids.add(item.id);
  }
  const ranges = [value.document, ...value.media].filter(r => r.length).sort((a, b) => a.offset - b.offset);
  for (let i = 1; i < ranges.length; i++) if (ranges[i]!.offset < ranges[i - 1]!.offset + ranges[i - 1]!.length) throw new Error('The project records overlap.');
  return value;
}
export async function readIncrementalIndex(size: number, read: (offset: number, length: number) => Promise<Uint8Array>, hash = sha256HexOf): Promise<ProjectIndex> {
  if (size < 5 + PROJECT_FOOTER_BYTES) throw new Error('The project commit is truncated.');
  const tail = readProjectFooter(await read(size - PROJECT_FOOTER_BYTES, PROJECT_FOOTER_BYTES), size);
  const bytes = await read(tail.offset, tail.length);
  if (await hash(bytes) !== tail.sha256) throw new Error('The project index checksum does not match.');
  return validateProjectIndex(JSON.parse(new TextDecoder().decode(bytes)), tail.offset);
}
export function decodeIncrementalProject(bytes: Uint8Array): { document: any; media: Array<IncrementalMedia & { data: Uint8Array }>; binary: true } {
  const tail = readProjectFooter(bytes.subarray(bytes.length - PROJECT_FOOTER_BYTES), bytes.length);
  const raw = bytes.subarray(tail.offset, tail.offset + tail.length);
  if (sha256Hex(raw) !== tail.sha256) throw new Error('The project index checksum does not match.');
  const index = validateProjectIndex(JSON.parse(new TextDecoder().decode(raw)), tail.offset);
  const content = (r: ProjectRecord) => {
    const data = bytes.subarray(r.offset, r.offset + r.length);
    if (sha256Hex(data) !== r.sha256) throw new Error('The project content checksum does not match.');
    return data;
  };
  return { document: JSON.parse(new TextDecoder().decode(content(index.document))), media: index.media.map(r => ({ ...r, data: content(r) })), binary: true };
}
"####;
    const FROZEN_CONTAINER_DECODE_TS: &str = r####"export const MAGIC = new Uint8Array([0x50, 0x4d, 0x56, 0x33, 0x0a]); // PMV3\n
export interface ProjectContainerMedia {
  id: string;
  type: string;
  data: Uint8Array;
}

export interface DecodedProjectContainer {
  document: any;
  media: ProjectContainerMedia[];
  binary: boolean;
}

const bytes = (value: Uint8Array | ArrayBuffer): Uint8Array =>
  value instanceof Uint8Array ? value : new Uint8Array(value);

export function isProjectContainer(value: Uint8Array): boolean {
  return value.byteLength >= MAGIC.length && MAGIC.every((byte, index) => value[index] === byte);
}

/** PMV3 stores media as raw bytes, avoiding base64's 33% size and transient string copies. */
export function encodeProjectContainer(document: any, media: ProjectContainerMedia[]): Uint8Array {
  let offset = 0;
  const descriptors = media.map(item => {
    const descriptor = { id: item.id, type: item.type, offset, length: item.data.byteLength };
    offset += item.data.byteLength;
    return descriptor;
  });
  const header = new TextEncoder().encode(JSON.stringify({ document, media: descriptors }));
  const result = new Uint8Array(MAGIC.length + 4 + header.byteLength + offset);
  result.set(MAGIC, 0);
  new DataView(result.buffer).setUint32(MAGIC.length, header.byteLength, true);
  result.set(header, MAGIC.length + 4);
  let cursor = MAGIC.length + 4 + header.byteLength;
  for (const item of media) {
    result.set(item.data, cursor);
    cursor += item.data.byteLength;
  }
  return result;
}

/** UTF-8 encoding also yields: a single TextEncoder call can stall on large histories. */
export async function encodeTextChunks(text: string): Promise<Uint8Array[]> {
  const chunks: Uint8Array[] = [], encoder = new TextEncoder();
  let deadline = performance.now() + 4;
  for (let start = 0; start < text.length;) {
    let end = Math.min(text.length, start + 256 * 1024);
    const last = text.charCodeAt(end - 1);
    if (end < text.length && last >= 0xd800 && last <= 0xdbff) end--;
    chunks.push(encoder.encode(text.slice(start, end))); start = end;
    if (performance.now() >= deadline) {
      await new Promise<void>(resolve => setTimeout(resolve, 0)); deadline = performance.now() + 4;
    }
  }
  return chunks;
}

/** Reuse serialized document bytes and copy large media in bounded slices. */
export async function encodeProjectContainerAsync(documentJSON: string | Uint8Array[], media: ProjectContainerMedia[]): Promise<Uint8Array> {
  let offset = 0;
  const descriptors = media.map(item => {
    const descriptor = { id: item.id, type: item.type, offset, length: item.data.byteLength };
    offset += item.data.byteLength;
    return descriptor;
  });
  const encoder = new TextEncoder();
  const header = [encoder.encode('{"document":'),
    ...(typeof documentJSON === 'string' ? await encodeTextChunks(documentJSON) : documentJSON),
    encoder.encode(',"media":' + JSON.stringify(descriptors) + '}')];
  const headerLength = header.reduce((sum, chunk) => sum + chunk.length, 0);
  const result = new Uint8Array(MAGIC.length + 4 + headerLength + offset);
  result.set(MAGIC);
  new DataView(result.buffer).setUint32(MAGIC.length, headerLength, true);
  let cursor = MAGIC.length + 4, deadline = performance.now() + 4;
  for (const source of [...header, ...media.map(item => item.data)]) {
    for (let start = 0; start < source.length; start += 1024 * 1024) {
      const chunk = source.subarray(start, start + 1024 * 1024);
      result.set(chunk, cursor); cursor += chunk.length;
      if (performance.now() >= deadline) {
        await new Promise<void>(resolve => setTimeout(resolve, 0)); deadline = performance.now() + 4;
      }
    }
  }
  return result;
}

/** Blob parts retain media without reading every source into renderer memory. */
export function encodeProjectContainerBlob(document: Uint8Array[], media: Array<{ id: string; type: string; data: Blob }>): Blob {
  let offset = 0;
  const descriptors = media.map(item => {
    const entry = { id: item.id, type: item.type, offset, length: item.data.size };
    offset += item.data.size;
    if (!Number.isSafeInteger(offset)) throw new Error('The media offsets exceed the project format’s numeric range.');
    return entry;
  });
  const header = new Blob(['{"document":', ...document as Uint8Array<ArrayBuffer>[], ',"media":', JSON.stringify(descriptors), '}']);
  if (header.size > 0xffffffff) throw new Error('The document metadata exceeds the PMV3 header format.');
  const prefix = new Uint8Array(MAGIC.length + 4);
  prefix.set(MAGIC);
  new DataView(prefix.buffer).setUint32(MAGIC.length, header.size, true);
  return new Blob([prefix, header, ...media.map(item => item.data)], { type: 'application/x-powermove' });
}

export interface ProjectMediaRange { id: string; type: string; offset: number; length: number; revision?: string; sha256?: string }

/** Validate every range before any imported media can be persisted. */
export function projectContainerIndex(header: any, bodyStart: number, fileSize: number): { document: any; media: ProjectMediaRange[] } {
  if (!header || typeof header !== 'object' || !header.document || !Array.isArray(header.media)
    || !Number.isSafeInteger(bodyStart) || !Number.isSafeInteger(fileSize) || bodyStart < 0 || bodyStart > fileSize) {
    throw new Error('The project container header is invalid.');
  }
  const ids = new Set<string>();
  const media = header.media.map((item: any) => {
    if (!item || typeof item.id !== 'string' || ids.has(item.id) || typeof item.type !== 'string'
      || !Number.isSafeInteger(item.offset) || item.offset < 0
      || !Number.isSafeInteger(item.length) || item.length < 0
      || item.offset > fileSize - bodyStart || item.length > fileSize - bodyStart - item.offset) {
      throw new Error('The project container media index is invalid.');
    }
    ids.add(item.id);
    return { id: item.id, type: item.type, offset: bodyStart + item.offset, length: item.length };
  });
  return { document: header.document, media };
}

export function decodeProjectContainer(input: string | Uint8Array | ArrayBuffer): DecodedProjectContainer {
  if (typeof input === 'string') return { document: JSON.parse(input), media: [], binary: false };
  const data = bytes(input);
  if (isIncrementalProject(data)) return decodeIncrementalProject(data);
  if (!isProjectContainer(data)) {
    return { document: JSON.parse(new TextDecoder().decode(data)), media: [], binary: false };
  }
  if (data.byteLength < MAGIC.length + 4) throw new Error('The project container is truncated.');
  const headerLength = new DataView(data.buffer, data.byteOffset, data.byteLength).getUint32(MAGIC.length, true);
  const bodyStart = MAGIC.length + 4 + headerLength;
  if (headerLength < 2 || bodyStart > data.byteLength) throw new Error('The project container header is invalid.');
  const header = JSON.parse(new TextDecoder().decode(data.subarray(MAGIC.length + 4, bodyStart)));
  const index = projectContainerIndex(header, bodyStart, data.byteLength);
  const media = index.media.map(item => ({ id: item.id, type: item.type, data: data.subarray(item.offset, item.offset + item.length) }));
  return { document: index.document, media, binary: true };
}
"####;
    const FROZEN_VALIDATOR_TS: &str = r####"export function assertProjectDocument(document: any): void {
  const project = document?.proj || document;
  if (!project || !Array.isArray(project.layers) || !Number.isFinite(project.w) || !Number.isFinite(project.h)) {
    throw new Error('This is not a Powermove project.');
  }
}
"####;
    const FROZEN_HYDRATION_TS: &str = r####"export function hydrateBoundary(p, PM) {
  const base = {};
  Object.assign(base, p);
  const container = base;
  for (const L of base.layers) {
      if (L.type != null && !PM.TYPE_META[L.type]) L.type = 'null';
      L.collapsed = L.collapsed !== false; L.fx = Array.isArray(L.fx) ? L.fx : []; L.p = L.p && typeof L.p === 'object' ? L.p : {}; L.d = L.d && typeof L.d === 'object' ? L.d : {};
      const fresh = PM.mkLayer(L.type || 'null', {}, container);
      Object.keys(L.p).forEach(k => { if (!(k in fresh.p)) delete L.p[k]; });
  }
  return base;
}
"####;

    const ORACLE_JS: &str = r####"
import fs from 'node:fs';
import {createHash} from 'node:crypto';
import * as nodeModule from 'node:module';
import assert from 'node:assert/strict';
process.on('uncaughtException', error => {
  console.error(String(error.stack).replace(/data:text\/javascript;base64,[A-Za-z0-9+/=]+/g, '[frozen-reader]'));
  process.exit(1);
});
const request = JSON.parse(fs.readFileSync(0, 'utf8'));
const strip = nodeModule.stripTypeScriptTypes ?? (s => new Bun.Transpiler({loader:'ts',target:'node'}).transformSync(s));
const load = s => import('data:text/javascript;base64,' + Buffer.from(strip(s)).toString('base64'));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const prefix = `import {createHash} from 'node:crypto';
const sha256Hex = bytes => createHash('sha256').update(bytes).digest('hex');
const sha256HexOf = async bytes => sha256Hex(bytes);
`;
// These extracts are byte-identical at all four audited release sources. Share the
// immutable vendored module while reporting/testing all four release contracts.
const old = await load(prefix + request.incremental + request.container + request.validator + request.hydration);
const transition = await load(request.proposal);
const roster = ['solid','text','shape','image','video','audio','adjustment','shader','extension','null','group','precomp','captions'];
const PM = {TYPE_META:Object.fromEntries(roster.map(k => [k,{}])), mkLayer:()=>({p:{}})};
let comparisons = 0;
for (const reference of ['v1.2.0@07ed5fa17', 'v1.3.0@3a4edb1f9', 'v1.3.1@5458b5449', 'v1.3.2/main@c7f46a056']) {
  for (const {path,guarded} of request.cases) {
    const bytes = fs.readFileSync(path), initialHash = hash(bytes);
    const decoded = old.decodeProjectContainer(bytes);
    old.assertProjectDocument(decoded.document);
    // Both production async-index and complete-buffer PMV4 entry points.
    const streamed = await old.readIncrementalIndex(bytes.length, async (o,n) => bytes.subarray(o,o+n));
    assert.deepEqual(JSON.parse(bytes.subarray(streamed.document.offset,streamed.document.offset+streamed.document.length)),decoded.document);
    const doc = decoded.document;
    const requirement = [doc,doc.proj].some(p => p && Object.hasOwn(p,'requiresEngine') && p.requiresEngine != null);
    assert.equal(requirement,guarded,reference + ':' + path);
    const untouched = JSON.stringify(doc);
    if (guarded) assert.throws(() => transition.assertLegacyReadable(doc), e => e.code === 'PMV_REQUIRES_NATIVE');
    else transition.assertLegacyReadable(doc);
    assert.equal(JSON.stringify(doc),untouched,'proposal cannot mutate input');

    // Unknown app-version / envelope fields are NOT a refusal in the old app.
    const futureEnvelope = {...doc,v:'999.0.0',minReaderVersion:999,opaqueFuture:{kept:true}};
    old.assertProjectDocument(futureEnvelope);
    const project = structuredClone(doc.proj);
    const futureField = {exact:0.1234567890123456,text:'🧭 é'};
    project.futureField = futureField;
    const unknown = project.layers.find(l => l.type != null && !PM.TYPE_META[l.type]);
    const hydrated = old.hydrateBoundary(project,PM);
    assert.deepEqual(hydrated.futureField,futureField);
    if (unknown) {
      assert.equal(unknown.type,'null','shipped hydrator loses unknown layer type');
      assert.deepEqual(unknown.p,{},'shipped hydrator loses unrecognized channels');
    }

    const tail = old.readProjectFooter(bytes.subarray(bytes.length-64),bytes.length);
    const index = JSON.parse(bytes.subarray(tail.offset,tail.offset+tail.length));
    index.version = 5;
    const indexBytes = Buffer.from(JSON.stringify(index));
    const unsupported = Buffer.concat([bytes.subarray(0,tail.offset),indexBytes,old.projectFooter(tail.offset,indexBytes.length,hash(indexBytes))]);
    assert.throws(() => old.decodeProjectContainer(unsupported), /The project index is invalid\./);
    await assert.rejects(old.readIncrementalIndex(unsupported.length,async(o,n)=>unsupported.subarray(o,o+n)), /The project index is invalid\./);
    const unknownMagic = Buffer.from(bytes);
    unknownMagic.set(Buffer.from('PMV5\n'));
    assert.throws(() => old.decodeProjectContainer(unknownMagic), SyntaxError);
    const corrupted = Buffer.from(bytes);
    corrupted[streamed.document.offset] ^= 1;
    assert.throws(() => old.decodeProjectContainer(corrupted), /content checksum/);

    // PMV3 has no numeric container-version guard; an extra header field is ignored.
    const pmv3 = old.encodeProjectContainer(doc,[]);
    const header = JSON.parse(new TextDecoder().decode(pmv3.subarray(9)));
    header.version = 999;
    const headerBytes = Buffer.from(JSON.stringify(header));
    const pmv3Prefix = Buffer.from(pmv3.subarray(0,9));
    pmv3Prefix.writeUInt32LE(headerBytes.length,5);
    const decoded3 = old.decodeProjectContainer(Buffer.concat([pmv3Prefix,headerBytes]));
    assert.deepEqual(decoded3.document,doc);
    assert.equal(hash(fs.readFileSync(path)),initialHash,'reader/proposal tests never rewrite native file');
    comparisons++;
  }
}
for (const holder of ['envelope','project']) {
  for (const value of ['native','future-engine',false,0,{},[]]) {
    const doc = {proj:{w:1,h:1,layers:[]}};
    (holder === 'project' ? doc.proj : doc).requiresEngine = value;
    assert.throws(() => transition.assertLegacyReadable(doc),e=>e.code === 'PMV_REQUIRES_NATIVE');
  }
  for (const value of [2,'2',{},false]) {
    const doc = {proj:{w:1,h:1,layers:[]}};
    (holder === 'project' ? doc.proj : doc).minReaderVersion = value;
    assert.throws(() => transition.assertLegacyReadable(doc),e=>e.code === 'PMV_READER_TOO_OLD');
  }
}
for (const value of [null,0,1]) transition.assertLegacyReadable({proj:{minReaderVersion:value},requiresEngine:null});
console.log(`${comparisons} frozen-reader/native-output comparisons; transition proposal passed`);
"####;
}
