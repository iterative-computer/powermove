//! `pm-cli render` and definition snapshots (contract 10; plan section 22,
//! "The CLI"): built-in effects draw from the built-in snapshots, a
//! `--definitions` set (the app's winning registrations) replaces them, so
//! an override by id draws the fork's GLSL; an effect with no snapshot that
//! is not marked missing fails naming it (exit 3), one marked missing is
//! skipped as in the app. User overrides draw through the production ANGLE
//! helper; without that helper an unverified program fails by name.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

// Remove fixtures even when an assertion panics.
struct Scratch(PathBuf);
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("pm-cli-defs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

/// A 320 × 180 green solid with `fx`.
fn project(fx: Value) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in [
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("position.x", 0.0),
        ("position.y", 0.0),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("rotation", 0.0),
        ("opacity", 100.0),
        ("skew", 0.0),
    ] {
        p.insert(k.into(), ch(json!(v)));
    }
    let bg = json!({
        "id": "bg", "type": "solid", "name": "bg", "from": 0, "dur": 1, "on": true, "solo": false,
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "threeD": false, "p": p, "fx": fx, "masks": [], "d": { "color": "#3FCF8E", "w": 320, "h": 180 },
        "transitionIn": null, "transitionOut": null
    });
    json!({
        "id": "P1", "name": "cli", "w": 320, "h": 180, "fps": 25, "dur": 1, "work": [0, 1],
        "compId": "P1-root",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s", "color": "#000000", "position": 0 }] },
        "layers": [bg], "assets": {}, "comps": {}
    })
}

fn invert() -> Value {
    json!([{ "id": "fx1", "type": "invert", "on": true, "p": { "amount": ch(json!(100)) } }])
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("render")
        .args(args)
        .env_remove("POWERMOVE_ANGLE_HELPER")
        .env_remove("POWERMOVE_ANGLE_ENDPOINT")
        .env_remove("POWERMOVE_ANGLE_TOKEN")
        .output()
        .unwrap()
}

// These gates use the production Node/Chromium helper. Set
// POWERMOVE_ANGLE_HELPER to a prebuilt CLI dist/native-angle-child.cjs; run
// this integration binary under pm-electron-lock, never build under the lock.
// Without the helper a default workspace run skips them with a printed
// reason; POWERMOVE_REQUIRE_ANGLE_HELPER=1 (the gate runs) makes that a failure.
fn angle_helper() -> Option<String> {
    match std::env::var("POWERMOVE_ANGLE_HELPER") {
        Ok(helper) if !helper.is_empty() => Some(helper),
        _ if std::env::var_os("POWERMOVE_REQUIRE_ANGLE_HELPER").is_some() => panic!(
            "POWERMOVE_REQUIRE_ANGLE_HELPER is set: build the CLI host and set POWERMOVE_ANGLE_HELPER"
        ),
        _ => {
            eprintln!(
                "skipped: this gate needs the production ANGLE helper (set POWERMOVE_ANGLE_HELPER to a built CLI dist/native-angle-child.cjs)"
            );
            None
        }
    }
}

fn cli_with_angle(args: &[&str], env: &[(&str, &std::ffi::OsStr)]) -> Option<std::process::Output> {
    let helper = angle_helper()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_pm-cli"));
    command
        .arg("render")
        .args(args)
        .env("POWERMOVE_ANGLE_HELPER", helper)
        .env_remove("POWERMOVE_ENGINE_PARITY_ENV")
        .env_remove("POWERMOVE_ENGINE_PARITY_STORE")
        .env_remove("POWERMOVE_ENGINE_SHADER_CACHE");
    for (key, value) in env {
        command.env(key, value);
    }
    Some(command.output().unwrap())
}

fn ok(out: &std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The top-left pixel of an 8-bit RGBA PNG still.
fn corner(path: &Path) -> [u8; 3] {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let channels = info.color_type.samples();
    let at = (2 * info.width as usize + 2) * channels;
    [buf[at], buf[at + 1], buf[at + 2]]
}

fn near(px: [u8; 3], want: [u8; 3]) -> bool {
    px.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2)
}

/// The built-in set as JSON with `edit` applied to it.
fn definitions_file(dir: &Path, edit: impl FnOnce(&mut Value)) -> PathBuf {
    let mut set = serde_json::to_value(pm_render::all_builtin_definitions()).unwrap();
    edit(&mut set);
    let path = dir.join("definitions.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// Replaces the body of `id`'s wrapped GLSL (everything after the last
/// `void main(){`) and keeps the lengths contract 10 checks.
fn rewrite_body(set: &mut Value, id: &str, origin: &str, body: &str) {
    let d = set["definitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d["id"] == id)
        .unwrap();
    d["origin"] = json!(origin);
    let source = d["glsl"]["source"].as_str().unwrap().to_owned();
    let at = source.rfind("void main(){").unwrap() + "void main(){".len();
    let body_with_end = format!("{body}\n}}");
    d["glsl"]["source"] = json!(format!("{}{body_with_end}", &source[..at]));
    d["glsl"]["bodyUtf16Len"] = json!(body_with_end.encode_utf16().count());
    d["glsl"]["bodyUtf8Len"] = json!(body_with_end.len());
}

#[test]
fn built_in_effects_draw_from_the_built_in_definitions() {
    let dir = scratch("builtin");
    let file = dir.join("p.json");
    std::fs::write(&file, project(invert()).to_string()).unwrap();
    let out = dir.join("still.png");
    let s = ok(&cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
    ]));
    assert_eq!(s["definitions"], json!("built-in"));
    // #3FCF8E inverted.
    assert!(near(corner(&out), [0xC0, 0x30, 0x71]), "{:?}", corner(&out));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rendering_with_the_built_in_set_warns_when_the_project_draws_definitions() {
    let dir = scratch("builtin-warning");
    let file = dir.join("p.json");
    std::fs::write(&file, project(invert()).to_string()).unwrap();
    let out = dir.join("still.png");
    let run = cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
    ]);
    ok(&run);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("warning: drawing the built-in definitions for")
            && stderr.contains("--definitions"),
        "{stderr}"
    );
    // With a snapshot, or with nothing that draws a definition, no warning.
    let set = definitions_file(&dir, |_| {});
    let out2 = dir.join("still2.png");
    let run = cli(&[
        file.to_str().unwrap(),
        "-o",
        out2.to_str().unwrap(),
        "--format",
        "still",
        "--definitions",
        set.to_str().unwrap(),
    ]);
    ok(&run);
    assert!(!String::from_utf8_lossy(&run.stderr).contains("warning: drawing the built-in"));
    let plain = dir.join("plain.json");
    std::fs::write(&plain, project(json!([])).to_string()).unwrap();
    let out3 = dir.join("still3.png");
    let run = cli(&[
        plain.to_str().unwrap(),
        "-o",
        out3.to_str().unwrap(),
        "--format",
        "still",
    ]);
    ok(&run);
    assert!(!String::from_utf8_lossy(&run.stderr).contains("warning: drawing the built-in"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_definitions_snapshot_overrides_built_ins_by_id() {
    let dir = scratch("override");
    let file = dir.join("p.json");
    std::fs::write(&file, project(invert()).to_string()).unwrap();
    let defs = definitions_file(&dir, |set| {
        rewrite_body(
            set,
            "invert",
            "fork",
            "\n      vec4 c = texture(u_tex, v_st); o = vec4(0.0, 0.0, c.a, c.a);",
        )
    });
    let out = dir.join("still.png");
    let Some(run) = cli_with_angle(
        &[
            file.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--format",
            "still",
            "--definitions",
            defs.to_str().unwrap(),
        ],
        &[],
    ) else {
        return;
    };
    let s = ok(&run);
    assert_eq!(s["definitions"], json!(defs.to_str().unwrap()));
    assert!(
        near(corner(&out), [0, 0, 255]),
        "the fork draws: {:?}",
        corner(&out)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_effect_without_a_snapshot_fails_naming_it_unless_marked_missing() {
    let dir = scratch("unknown");
    let file = dir.join("p.json");
    let fx = json!([{ "id": "fx1", "type": "acme.glow", "on": true, "p": {} }]);
    std::fs::write(&file, project(fx).to_string()).unwrap();
    let out = dir.join("still.png");
    let run = cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
    ]);
    assert_eq!(
        run.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("no definition for effect acme.glow"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(!out.exists(), "nothing written");
    // Marked missing in the app's set: skipped with its data kept.
    let defs = definitions_file(&dir, |set| set["missing"] = json!(["acme.glow"]));
    ok(&cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
        "--definitions",
        defs.to_str().unwrap(),
    ]));
    assert!(near(corner(&out), [0x3F, 0xCF, 0x8E]), "{:?}", corner(&out));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_program_only_the_webgl_runner_draws_is_not_headless_renderable() {
    let dir = scratch("runner");
    let file = dir.join("p.json");
    let fx = json!([{ "id": "fx1", "type": "acme.sky", "on": true, "p": {} }]);
    std::fs::write(&file, project(fx).to_string()).unwrap();
    // A samplerCube (amendment 9: the bind check refuses non-sampler2D
    // samplers; WebGL draws them).
    let source = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform samplerCube u_sky;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_st) + texture(u_sky, vec3(0.0, 0.0, 1.0)); }\n";
    let defs = definitions_file(&dir, |set| {
        set["definitions"].as_array_mut().unwrap().push(json!({
            "id": "acme.sky", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
            "glsl": { "source": source, "bodyLineOffset": 0, "bodyUtf16Len": source.encode_utf16().count(), "bodyUtf8Len": source.len() },
            "params": [],
            "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
        }));
    });
    let out = dir.join("still.png");
    for _ in 0..2 {
        let run = cli(&[
            file.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--format",
            "still",
            "--definitions",
            defs.to_str().unwrap(),
        ]);
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(run.status.code(), Some(4), "{stderr}");
        assert!(stderr.contains("not headless-renderable"), "{stderr}");
        assert!(stderr.contains("acme.sky"), "{stderr}");
        assert!(stderr.contains("layer bg"), "{stderr}");
        assert!(!out.exists(), "nothing written");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_parameter_overrides_change_rendered_expression_values_and_refuse_invalid_values() {
    let dir = scratch("parameters");
    let file = dir.join("p.json");
    let mut p = project(json!([]));
    p["params"] = json!({ "alpha": { "control": "num", "value": 100, "min": 0, "max": 100 } });
    p["layers"][0]["p"]["opacity"]["expr"] = json!("param('alpha')");
    std::fs::write(&file, p.to_string()).unwrap();
    let out = dir.join("parameter.png");
    ok(&cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
        "--param",
        "alpha=0",
    ]));
    assert_eq!(corner(&out), [0, 0, 0]);
    let invalid = dir.join("invalid.png");
    let result = cli(&[
        file.to_str().unwrap(),
        "-o",
        invalid.to_str().unwrap(),
        "--format",
        "still",
        "--param",
        "alpha=101",
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid value for parameter"));
    assert!(!invalid.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unverified_shader_layer_is_not_delivered_without_its_browser_reference() {
    let dir = scratch("unverified-layer");
    let file = dir.join("p.json");
    let mut p = project(json!([]));
    p["layers"][0]["type"] = json!("shader");
    p["layers"][0]["d"] = json!({ "code": "void main(){ fragColor = vec4(0.25, 0.5, 0.75, 1.0); }", "uniforms": {}, "w": 320, "h": 180 });
    std::fs::write(&file, p.to_string()).unwrap();
    let out = dir.join("shader.png");
    let result = cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
    ]);
    assert!(
        !result.status.success(),
        "an unknown program must not render natively"
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("fallback runner"));
    assert!(!out.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// The desktop's probe namespace compares native output with Electron's
/// WebGL runner. A CLI first probe (Playwright's Chromium) honours it but must
/// never leave a `.native` (or `.runner`) marker the desktop would trust, on
/// both entry points: direct pm-cli reading the profile's descriptor, and
/// `powermove render`, which passes the descriptor in the environment.
#[test]
fn a_cli_first_probe_leaves_no_decision_in_the_desktop_namespace() {
    let dir = scratch("first-probe");
    let file = dir.join("p.json");
    std::fs::write(&file, project(invert()).to_string()).unwrap();
    // An exactly representable fork: native and ANGLE agree, so a writable
    // namespace would record a native verification.
    let defs = definitions_file(&dir, |set| {
        rewrite_body(
            set,
            "invert",
            "fork",
            "\n      vec4 c = texture(u_tex, v_st); o = vec4(0.0, 0.0, c.a, c.a);",
        )
    });
    let cache = dir.join("Shader Cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("desktop-environment.json"),
        json!({ "v": 1, "environment": "{\"electron\":\"desktop\",\"chrome\":\"electron chromium\"}" }).to_string(),
    )
    .unwrap();
    let markers = || -> Vec<String> {
        std::fs::read_dir(cache.join("angle-parity"))
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let descriptor = std::fs::read_to_string(cache.join("desktop-environment.json")).unwrap();
    let descriptor: Value = serde_json::from_str(&descriptor).unwrap();
    let descriptor = descriptor["environment"].as_str().unwrap().to_owned();
    for (entry, env) in [
        (
            "direct",
            vec![("POWERMOVE_ENGINE_SHADER_CACHE", cache.as_os_str())],
        ),
        (
            "powermove render",
            vec![
                ("POWERMOVE_ENGINE_SHADER_CACHE", cache.as_os_str()),
                (
                    "POWERMOVE_ENGINE_PARITY_ENV",
                    std::ffi::OsStr::new(&descriptor),
                ),
                (
                    "POWERMOVE_ENGINE_PARITY_STORE",
                    std::ffi::OsStr::new("read-only"),
                ),
            ],
        ),
    ] {
        let out = dir.join(format!("{}.png", entry.replace(' ', "-")));
        let Some(run) = cli_with_angle(
            &[
                file.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "--format",
                "still",
                "--definitions",
                defs.to_str().unwrap(),
            ],
            &env,
        ) else {
            return;
        };
        ok(&run);
        assert!(
            near(corner(&out), [0, 0, 255]),
            "{entry}: {:?}",
            corner(&out)
        );
        assert_eq!(
            markers(),
            Vec::<String>::new(),
            "{entry} wrote a probe decision"
        );
    }
    // The desktop's own verification is still honoured: a native marker for
    // this exact program would be read (the pin-store harness checks pixels).
    std::fs::remove_dir_all(dir).unwrap();
}

/// `pm-cli scope` draws with `render`'s setup (DEFS-4): the same probe store,
/// definitions, and Chromium runner, so its waveform, parade, vectorscope and
/// scope.json never come from a user program drawn natively unverified.
fn scope(args: &[&str], helper: Option<&str>, cache: &Path) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pm-cli"));
    command
        .arg("scope")
        .args(args)
        .env_remove("POWERMOVE_ANGLE_ENDPOINT")
        .env_remove("POWERMOVE_ANGLE_TOKEN")
        .env_remove("POWERMOVE_ENGINE_PARITY_ENV")
        .env_remove("POWERMOVE_ENGINE_PARITY_STORE")
        // An empty probe namespace: nothing is verified natively.
        .env("POWERMOVE_ENGINE_SHADER_CACHE", cache);
    match helper {
        Some(helper) => command.env("POWERMOVE_ANGLE_HELPER", helper),
        None => command.env_remove("POWERMOVE_ANGLE_HELPER"),
    };
    command.output().unwrap()
}

fn shader_layer_project(dir: &Path) -> PathBuf {
    let file = dir.join("p.json");
    let mut p = project(json!([]));
    p["layers"][0]["type"] = json!("shader");
    p["layers"][0]["d"] = json!({ "code": "void main(){ fragColor = vec4(0.25, 0.5, 0.75, 1.0); }", "uniforms": {}, "w": 320, "h": 180 });
    std::fs::write(&file, p.to_string()).unwrap();
    file
}

#[test]
fn scope_never_measures_an_unverified_shader_layer_drawn_natively() {
    let dir = scratch("scope-unverified-layer");
    let file = shader_layer_project(&dir);
    let cache = dir.join("Shader Cache");
    std::fs::create_dir_all(&cache).unwrap();
    let out = dir.join("scopes");
    let run = scope(
        &[file.to_str().unwrap(), "-o", out.to_str().unwrap()],
        None,
        &cache,
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(4), "{stderr}");
    assert!(stderr.contains("not headless-renderable"), "{stderr}");
    assert!(stderr.contains("fallback runner"), "{stderr}");
    assert!(stderr.contains("layer bg"), "{stderr}");
    assert!(
        !out.join("scope.json").exists(),
        "no scopes of native pixels"
    );
}

#[test]
fn scope_takes_definitions_and_refuses_a_runner_only_program_without_a_helper() {
    let dir = scratch("scope-runner");
    let file = dir.join("p.json");
    let fx = json!([{ "id": "fx1", "type": "acme.sky", "on": true, "p": {} }]);
    std::fs::write(&file, project(fx).to_string()).unwrap();
    let source = "#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform samplerCube u_sky;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_st) + texture(u_sky, vec3(0.0, 0.0, 1.0)); }\n";
    let defs = definitions_file(&dir, |set| {
        set["definitions"].as_array_mut().unwrap().push(json!({
            "id": "acme.sky", "kind": "effect", "version": 0, "origin": "local", "renderer": null,
            "glsl": { "source": source, "bodyLineOffset": 0, "bodyUtf16Len": source.encode_utf16().count(), "bodyUtf8Len": source.len() },
            "params": [],
            "flags": { "passes": 1, "keepOrig": false, "backdrop": false, "rawShader": true, "viewportSafe": false, "viewportPadding": [], "clamp": false }
        }));
    });
    let cache = dir.join("Shader Cache");
    std::fs::create_dir_all(&cache).unwrap();
    let out = dir.join("scopes");
    let run = scope(
        &[
            file.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--definitions",
            defs.to_str().unwrap(),
        ],
        None,
        &cache,
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.status.code(), Some(4), "{stderr}");
    assert!(stderr.contains("acme.sky"), "{stderr}");
    assert!(!out.join("scope.json").exists());
    // Built-in effects now draw in scope from the built-in set (they were
    // routed before scope took definitions), with render's warning.
    std::fs::write(&file, project(invert()).to_string()).unwrap();
    let run = scope(
        &[file.to_str().unwrap(), "-o", out.to_str().unwrap()],
        None,
        &cache,
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert!(
        stderr.contains("drawing the built-in definitions"),
        "{stderr}"
    );
    let stats: Value = serde_json::from_slice(&run.stdout).unwrap();
    // #3FCF8E inverted: #C03071.
    let mean = |c: &str| stats[c]["mean"].as_f64().unwrap();
    assert!(
        (mean("red") - f64::from(0xC0u8) / 255.0).abs() < 2.0 / 255.0,
        "{stats}"
    );
    assert!(
        (mean("green") - f64::from(0x30u8) / 255.0).abs() < 2.0 / 255.0,
        "{stats}"
    );
    assert!(
        (mean("blue") - f64::from(0x71u8) / 255.0).abs() < 2.0 / 255.0,
        "{stats}"
    );
}

#[test]
fn scope_measures_an_unverified_shader_layer_through_the_chromium_runner() {
    let Some(helper) = angle_helper() else {
        return;
    };
    let dir = scratch("scope-runner-layer");
    let file = shader_layer_project(&dir);
    let cache = dir.join("Shader Cache");
    std::fs::create_dir_all(&cache).unwrap();
    let out = dir.join("scopes");
    let run = scope(
        &[file.to_str().unwrap(), "-o", out.to_str().unwrap()],
        Some(&helper),
        &cache,
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    let stats: Value = serde_json::from_slice(&run.stdout).unwrap();
    for (channel, want) in [("red", 0.25), ("green", 0.5), ("blue", 0.75)] {
        for k in ["min", "max"] {
            let got = stats[channel][k].as_f64().unwrap();
            assert!((got - want).abs() <= 1.0 / 255.0, "{channel} {k}: {stats}");
        }
    }
    assert!(out.join("scope.json").exists());
    // A first probe leaves no decision the desktop would trust.
    let left: Vec<_> = std::fs::read_dir(cache.join("angle-parity"))
        .map(|d| d.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}
