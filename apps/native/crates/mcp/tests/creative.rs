use pm_mcp::{
    Automation, McpSession, Result,
    creative::{CreativePaths, CreativeReader, parse_workspace},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

struct Fixture {
    root: PathBuf,
    reader: CreativeReader,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-creative"));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let reader = CreativeReader::new(CreativePaths {
            home: root.join("home"),
            applications: root.join("applications"),
            system_cep: root.join("system-cep"),
        })
        .unwrap();
        Self { root, reader }
    }
    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn workspace(&self) {
        self.write(
            "home/Library/Preferences/Adobe/After Effects/24.9/Adobe After Effects 24.9 Prefs.txt",
            "\"Current Workspace2\" = \"Synthetic\"",
        );
        self.write(
            "home/Library/Preferences/Adobe/After Effects/24.9/ModifiedWorkspaces/layout.xml",
            workspace("Synthetic"),
        );
    }
    fn script(&self, name: &str, source: &str) {
        self.write(
            &format!(
                "home/Library/Preferences/Adobe/After Effects/24.9/Scripts/ScriptUI Panels/{name}"
            ),
            source,
        );
    }
    fn compare(&self, tool: &str, args: Value) -> Value {
        let actual = if tool == "inspect_creative_workspace" {
            self.reader.workspace(&args)
        } else {
            self.reader.extension(&args)
        };
        let expected = oracle(json!({"root":self.root,"tool":tool,"arguments":args}));
        compare(actual, &expected);
        expected["result"].clone()
    }
    fn tools(&self) -> Vec<Value> {
        self.compare(
            "inspect_creative_workspace",
            json!({"appId":"after-effects"}),
        )["installedTools"]["tools"]
            .as_array()
            .unwrap()
            .clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn pair(key: &str, tag: &str, text: &str) -> String {
    format!("<prop.pair><key>{key}</key><{tag}>{text}</{tag}></prop.pair>")
}
fn frame(proxy: bool) -> String {
    pair(
        if proxy { "FrameProxy" } else { "Frame" },
        "prop.map",
        &format!(
            "{}{}{}",
            pair(
                "TabIDs",
                "array",
                "<string>Project</string><ustring>Timeline</ustring>"
            ),
            pair("CurrTab", "int", "1"),
            pair("Vis", "true", "")
        ),
    )
}
fn workspace(name: &str) -> String {
    let split = format!(
        "{}{}{}{}{}",
        pair("Place", "float", "0.25"),
        pair("Orient", "true", ""),
        pair("Sub1", "prop.map", &frame(false)),
        pair("Sub2", "prop.map", &frame(true)),
        pair("Sub2Vis", "false", "")
    );
    format!(
        "<prop.map><prop.list>{}{}{}</prop.list></prop.map>",
        pair("UserName", "string", name),
        pair(
            "TopLevelFrame-0",
            "prop.map",
            &pair("Splitter", "prop.map", &split)
        ),
        pair("TopLevelFrame-1", "prop.map", &frame(false))
    )
}
fn oracle(input: Value) -> Value {
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/creative-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn compare(actual: Result<Value>, expected: &Value) {
    match actual {
        Ok(value) => {
            assert!(expected.get("error").is_none(), "{expected}");
            assert_eq!(value, expected["result"]);
        }
        Err(error) => assert_eq!(error.message, expected["error"].as_str().unwrap()),
    }
}
fn extension_args(tool: &Value, path: Option<&str>) -> Value {
    let mut args = json!({"appId":"after-effects","toolId":tool["id"]});
    if let Some(path) = path {
        args["path"] = json!(path)
    }
    args
}

#[test]
fn layout_parser_matches_tag_bounds_visibility_tabs_windows_and_entities() {
    for source in [
        workspace("Synthetic &amp; &lt; &quot; &apos;"),
        workspace(""),
        "<prop.map/>".into(),
    ] {
        let expected = oracle(json!({"source":source}));
        compare(parse_workspace(&source), &expected);
    }
    let layout = parse_workspace(&workspace("Synthetic")).unwrap();
    assert_eq!(layout["groups"].as_array().unwrap().len(), 3);
    assert_eq!(
        layout["groups"][0]["bounds"],
        json!({"x":0,"y":0,"width":0.25,"height":1})
    );
    assert_eq!(layout["groups"][1]["visible"], false);
    assert_eq!(layout["groups"][2]["window"], 1);
}
#[test]
fn layout_parser_refuses_malformed_oversized_deep_and_entity_documents() {
    for source in [
        "<!DOCTYPE x><prop.map/>".into(),
        "<!ENTITY x SYSTEM 'private'>".into(),
        "<prop.map>".into(),
        "<prop.map></other>".into(),
        "<x>".repeat(81),
        "x".repeat(512 * 1024 + 1),
    ] {
        compare(parse_workspace(&source), &oracle(json!({"source":source})));
    }
}
#[test]
fn inventory_does_not_guess_missing_current_or_named_workspace() {
    let f = Fixture::new();
    for args in [
        json!({}),
        json!({"appId":"unsupported"}),
        json!({"appId":"after-effects","workspaceName":null}),
        json!({"appId":"after-effects","workspaceName":"x".repeat(201)}),
        json!({"appId":"after-effects"}),
    ] {
        f.compare("inspect_creative_workspace", args);
    }
    f.workspace();
    f.compare(
        "inspect_creative_workspace",
        json!({"appId":"after-effects","workspaceName":"Missing"}),
    );
    fs::remove_file(f.root.join(
        "home/Library/Preferences/Adobe/After Effects/24.9/Adobe After Effects 24.9 Prefs.txt",
    ))
    .unwrap();
    assert_eq!(
        f.compare(
            "inspect_creative_workspace",
            json!({"appId":"after-effects"})
        )["status"],
        "needs-workspace"
    );
    assert_eq!(
        f.compare(
            "inspect_creative_workspace",
            json!({"appId":"after-effects","workspaceName":"Synthetic"})
        )["status"],
        "ready"
    );
}
#[test]
fn newest_numeric_version_modified_precedence_and_parse_warnings_match_tag() {
    let f = Fixture::new();
    f.workspace();
    f.write(
        "home/Library/Preferences/Adobe/After Effects/24.8/OriginalUserWorkspaces/older.xml",
        workspace("Older"),
    );
    f.write(
        "home/Library/Preferences/Adobe/After Effects/24.9/ModifiedWorkspaces/bad.xml",
        "<!DOCTYPE x>",
    );
    f.write(
        "home/Library/Preferences/Adobe/After Effects/24.9/OriginalUserWorkspaces/original.xml",
        workspace("Synthetic").replace("0.25", "0.75"),
    );
    let value = f.compare(
        "inspect_creative_workspace",
        json!({"appId":"after-effects"}),
    );
    assert_eq!(value["version"], "24.9");
    assert_eq!(value["warnings"], json!(["Could not read layout bad.xml."]));
    assert_eq!(value["workspace"]["groups"][0]["bounds"]["width"], 0.25);
    f.write(
        "home/Library/Preferences/Adobe/After Effects/24.10/OriginalUserWorkspaces/new.xml",
        workspace("New"),
    );
    assert_eq!(
        f.compare(
            "inspect_creative_workspace",
            json!({"appId":"after-effects","workspaceName":"New"})
        )["version"],
        "24.10"
    );
}
#[test]
fn script_inventory_follows_only_literal_local_includes_and_reads_on_request() {
    let f = Fixture::new();
    f.workspace();
    f.script("panel.jsx","#include \"helpers/a.jsxinc\"\n#include <helpers/a.jsxinc>\n#include \"../outside.jsx\"\n#include \"binary.png\"\nSynthetic source 🦀\n");
    f.script(
        "helpers/a.jsxinc",
        "#include \"b.jsinc\"\n#include \"../panel.jsx\"\n",
    );
    f.script("helpers/b.jsinc", "Synthetic helper\n");
    f.script(".private.jsx", "Private");
    // The tagged script-panel inventory includes dot-named labels, but its
    // explicit source reader rejects such paths. Keep label/source distinct.
    let tools = f.tools();
    let tool = tools.iter().find(|t| t["name"] == "panel.jsx").unwrap();
    let result = f.compare("inspect_creative_extension", extension_args(tool, None));
    assert_eq!(result["files"].as_array().unwrap().len(), 3);
    for path in ["panel.jsx", "helpers/a.jsxinc", "helpers/b.jsinc"] {
        f.compare(
            "inspect_creative_extension",
            extension_args(tool, Some(path)),
        );
    }
    f.compare(
        "inspect_creative_extension",
        extension_args(tool, Some("not-listed.js")),
    );
    assert!(
        !result.to_string().contains(f.root.to_str().unwrap()),
        "opaque inventory must not expose roots"
    );
}
#[test]
fn cep_manifest_source_tree_filters_secrets_binary_hidden_and_protected_sources() {
    let f = Fixture::new();
    f.workspace();
    let prefix = "home/Library/Application Support/Adobe/CEP/extensions/synthetic";
    f.write(&format!("{prefix}/CSXS/manifest.xml"),"<ExtensionManifest ExtensionBundleId='synthetic.id'><Host Name='AEFT'/><Menu>Synthetic &amp; CEP</Menu></ExtensionManifest>");
    for (name, text) in [
        ("index.html", "<p>Synthetic</p>"),
        ("source/main.ts", "export const synthetic=1"),
        ("README.md", "Synthetic"),
        ("secret.json", "private"),
        (".env", "private"),
        ("node_modules/private.js", "private"),
        ("binary.png", "pixels"),
        ("nul.js", "not\0source"),
    ] {
        f.write(&format!("{prefix}/{name}"), text);
    }
    f.write(&format!("{prefix}/large.js"), vec![b'x'; 512 * 1024 + 1]);
    let tool = f
        .tools()
        .into_iter()
        .find(|t| t["kind"] == "cep-extension")
        .unwrap();
    let listing = f.compare("inspect_creative_extension", extension_args(&tool, None));
    assert_eq!(listing["files"].as_array().unwrap().len(), 5);
    for file in [
        "index.html",
        "source/main.ts",
        "CSXS/manifest.xml",
        "secret.json",
        ".env",
        "node_modules/private.js",
        "large.js",
        "nul.js",
        "../private.js",
        "/private.js",
        "a\\b.js",
    ] {
        f.compare(
            "inspect_creative_extension",
            extension_args(&tool, Some(file)),
        );
    }
}
#[test]
fn compiled_plugins_and_binary_script_panels_are_metadata_only() {
    let f = Fixture::new();
    f.workspace();
    f.write("applications/Adobe After Effects Synthetic/Plug-ins/Synthetic.plugin/Contents/MacOS/private","binary");
    f.write(
        "applications/Adobe After Effects Synthetic/Support Files/Plug-ins/Synthetic.aex",
        "binary",
    );
    f.script("Protected.jsxbin", "binary");
    let tools = f.tools();
    assert_eq!(tools.len(), 3);
    for tool in &tools {
        assert_eq!(
            f.compare("inspect_creative_extension", extension_args(tool, None))["status"],
            "metadata-only"
        );
        f.compare(
            "inspect_creative_extension",
            extension_args(tool, Some("Contents/MacOS/private")),
        );
    }
}
#[test]
fn system_cep_and_application_bundle_sources_match_tag() {
    let f = Fixture::new();
    f.workspace();
    f.write("system-cep/synthetic/CSXS/manifest.xml","<ExtensionManifest ExtensionBundleName='System' ExtensionBundleId='system.id'><Host Name='AEFT'/></ExtensionManifest>");
    f.write("applications/Adobe After Effects Synthetic/Adobe After Effects.app/Contents/Scripts/ScriptUI Panels/Bundled.jsx","Synthetic bundle");
    f.write(
        "home/Library/Application Support/Adobe/CEP/extensions/other/CSXS/manifest.xml",
        "<Host Name='NOT_AEFT'/>",
    );
    let tools = f.tools();
    assert_eq!(tools.len(), 2);
    for tool in &tools {
        f.compare("inspect_creative_extension", extension_args(tool, None));
    }
}
#[test]
fn extension_validation_precedence_and_disappearing_inventory_match_tag() {
    let f = Fixture::new();
    f.workspace();
    f.script("Synthetic.jsx", "Synthetic source");
    for args in [
        json!({}),
        json!({"appId":"after-effects"}),
        json!({"appId":"after-effects","toolId":"BAD"}),
        json!({"appId":"after-effects","toolId":"0".repeat(32),"path":null}),
        json!({"appId":"after-effects","toolId":"0".repeat(32)}),
    ] {
        f.compare("inspect_creative_extension", args);
    }
    let tool = f.tools().remove(0);
    fs::remove_file(f.root.join(
        "home/Library/Preferences/Adobe/After Effects/24.9/Scripts/ScriptUI Panels/Synthetic.jsx",
    ))
    .unwrap();
    f.compare("inspect_creative_extension", extension_args(&tool, None));
}
#[test]
fn source_count_and_depth_limits_are_visible_and_match_tag() {
    let f = Fixture::new();
    f.workspace();
    for index in 0..66 {
        f.script(
            &format!("part{index}.jsxinc"),
            &format!("#include \"part{}.jsxinc\"", index + 1),
        );
    }
    f.script("Synthetic.jsx", "#include \"part0.jsxinc\"");
    let tool = f.tools().remove(0);
    let result = f.compare("inspect_creative_extension", extension_args(&tool, None));
    assert_eq!(result["files"].as_array().unwrap().len(), 64);
    assert_eq!(result["truncated"], true);
    f.write(
        "system-cep/synthetic/CSXS/manifest.xml",
        "<Host Name='AEFT'/>",
    );
    f.write(
        "system-cep/synthetic/a/b/c/d/e/f/g/h/i/deep.js",
        "Synthetic",
    );
    let cep = f
        .tools()
        .into_iter()
        .find(|t| t["kind"] == "cep-extension")
        .unwrap();
    assert_eq!(
        f.compare("inspect_creative_extension", extension_args(&cep, None))["truncated"],
        true
    );
}
#[test]
fn exact_file_size_boundary_and_unicode_byte_counts_match_tag() {
    let f = Fixture::new();
    f.workspace();
    f.script("Synthetic.jsx", &"😀".repeat(128 * 1024));
    let tool = f.tools().remove(0);
    let result = f.compare("inspect_creative_extension", extension_args(&tool, None));
    assert_eq!(result["files"][0]["bytes"], 512 * 1024);
    f.compare(
        "inspect_creative_extension",
        extension_args(&tool, Some("Synthetic.jsx")),
    );
    f.script("Synthetic.jsx", &"x".repeat(512 * 1024 + 1));
    assert_eq!(
        f.compare("inspect_creative_extension", extension_args(&tool, None))["status"],
        "metadata-only"
    );
}
#[cfg(unix)]
#[test]
fn source_symlinks_and_directory_links_are_never_read() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.workspace();
    f.script(
        "Synthetic.jsx",
        "#include \"linked.jsxinc\"\n#include \"linked/helper.js\"",
    );
    f.write("outside/private.jsxinc", "Synthetic private data");
    f.write("outside/helper.js", "Synthetic private helper");
    let scripts = f
        .root
        .join("home/Library/Preferences/Adobe/After Effects/24.9/Scripts/ScriptUI Panels");
    symlink(
        f.root.join("outside/private.jsxinc"),
        scripts.join("linked.jsxinc"),
    )
    .unwrap();
    symlink(f.root.join("outside"), scripts.join("linked")).unwrap();
    let tool = f.tools().remove(0);
    assert_eq!(
        f.compare("inspect_creative_extension", extension_args(&tool, None))["files"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.compare(
        "inspect_creative_extension",
        extension_args(&tool, Some("linked.jsxinc")),
    );
    f.compare(
        "inspect_creative_extension",
        extension_args(&tool, Some("linked/helper.js")),
    );
    // N3/N4: links in ancestors are also refused, even where the tag's lstat
    // reader would follow them. This is not authored layout/source intent.
    fs::rename(&scripts, scripts.with_file_name("real-panels")).unwrap();
    symlink(scripts.with_file_name("real-panels"), &scripts).unwrap();
    assert_eq!(
        f.reader
            .workspace(&json!({"appId":"after-effects"}))
            .unwrap()["installedTools"]["tools"],
        json!([])
    );
}
#[test]
fn mcp_bodies_use_real_inventory_without_changing_kernel_or_edit_baseline() {
    let f = Fixture::new();
    f.workspace();
    f.script("Synthetic.jsx", "Synthetic");
    let mut host = Automation::new(f.root.join("profile"), f.root.join("exports")).unwrap();
    let id = host
        .create_project(&json!({"name":"Synthetic inspection"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = host.read_document(&id).unwrap();
    host.creative = Arc::new(f.reader.clone());
    let shared = Arc::new(Mutex::new(host));
    let mut session = McpSession::new(shared.clone());
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Inspect","access":"project"}),
        )
        .unwrap();
    let tools = session
        .dispatch(
            "inspect_creative_workspace",
            &json!({"appId":"after-effects"}),
        )
        .unwrap()["installedTools"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        session
            .dispatch(
                "inspect_creative_extension",
                &extension_args(&tools[0], None)
            )
            .unwrap()["status"],
        "ready"
    );
    assert_eq!(shared.lock().unwrap().read_document(&id).unwrap(), before);
    {
        let mut host = shared.lock().unwrap();
        let revision = host.revision(&id).unwrap();
        host.apply_commands(
            &id,
            vec![
                pm_kernel::CompCommand::Set {
                    patch: Box::new(pm_kernel::commands::comps::CompPatch {
                        name: Some("Synthetic foreign edit".into()),
                        ..Default::default()
                    }),
                }
                .into(),
            ],
            "Foreign edit",
            pm_kernel::Origin::Agent,
            revision,
            777,
        )
        .unwrap();
    }
    session
        .dispatch(
            "inspect_creative_workspace",
            &json!({"appId":"after-effects"}),
        )
        .unwrap();
    session
        .dispatch(
            "inspect_creative_extension",
            &extension_args(&tools[0], None),
        )
        .unwrap();
    assert_eq!(
        session
            .dispatch(
                "apply_commands",
                &json!({"commands":[{"type":"add_layer","layerType":"solid"}]})
            )
            .unwrap_err()
            .code,
        "revision_conflict",
        "main-host inventory reads must not refresh the run's edit observation"
    );
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Inspect app","context":"app"}),
        )
        .unwrap();
    assert_eq!(
        session
            .dispatch(
                "inspect_creative_extension",
                &extension_args(&tools[0], None)
            )
            .unwrap()["status"],
        "ready"
    );
}
#[cfg(not(target_os = "macos"))]
#[test]
fn linux_default_reports_actual_unsupported_platform_with_tagged_precedence() {
    let reader = CreativeReader::default();
    let f = Fixture::new();
    let args = json!({"appId":"after-effects"});
    compare(
        reader.workspace(&args),
        &oracle(
            json!({"root":f.root,"platform":"linux","tool":"inspect_creative_workspace","arguments":args}),
        ),
    );
    compare(
        reader.extension(&args),
        &oracle(
            json!({"root":f.root,"platform":"linux","tool":"inspect_creative_extension","arguments":args}),
        ),
    );
}
#[test]
fn relative_inventory_roots_are_rejected() {
    assert!(
        CreativeReader::new(CreativePaths {
            home: Path::new("relative").into(),
            applications: "/synthetic".into(),
            system_cep: "/synthetic".into()
        })
        .is_err()
    );
}

#[test]
fn xml_numeric_and_duplicate_properties_match_tag_without_guessing_panels() {
    for source in [
        workspace("Synthetic").replace("<int>1</int>", "<int>invalid</int>"),
        workspace("Synthetic").replace("<float>0.25</float>", "<float>0x1</float>"),
        workspace("Synthetic").replace("<float>0.25</float>", "<float>invalid</float>"),
        workspace("Synthetic").replace("<int>1</int>", "<int>-1</int>"),
        workspace("Synthetic").replace("<int>1</int>", "<int>100</int>"),
        workspace("Synthetic").replace(
            "<prop.list>",
            &format!("<prop.list>{}", pair("UserName", "string", "Earlier")),
        ),
    ] {
        compare(parse_workspace(&source), &oracle(json!({"source":source})));
    }
}
#[test]
fn unicode_boundary_digests_never_split_a_scalar_or_change_original_xml() {
    // Native deliberate difference: JS slice(0,200) would return a lone high
    // surrogate. That is not authored naming intent; leave the source intact.
    let name = format!("{}😀", "x".repeat(199));
    let source = workspace(&name);
    assert_eq!(parse_workspace(&source).unwrap()["name"], "x".repeat(199));
    assert!(source.contains(&name));
}
