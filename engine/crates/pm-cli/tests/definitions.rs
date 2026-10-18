//! `pm-cli render` and definition snapshots (contract 10; plan section 22,
//! "The CLI"): built-in effects draw from the built-in snapshots, a
//! `--definitions` set (the app's winning registrations) replaces them, so
//! an override by id draws the fork's GLSL; an effect with no snapshot that
//! is not marked missing fails naming it (exit 3), one marked missing is
//! skipped as in the app; a program only the app's WebGL fallback runner
//! draws fails as not headless-renderable, named, on every run (exit 4).
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-cli-defs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
        .output()
        .unwrap()
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
    let s = ok(&cli(&[file.to_str().unwrap(), "-o", out.to_str().unwrap(), "--format", "still"]));
    assert_eq!(s["definitions"], json!("built-in"));
    // #3FCF8E inverted.
    assert!(near(corner(&out), [0xC0, 0x30, 0x71]), "{:?}", corner(&out));
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
    let s = ok(&cli(&[
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--format",
        "still",
        "--definitions",
        defs.to_str().unwrap(),
    ]));
    assert_eq!(s["definitions"], json!(defs.to_str().unwrap()));
    assert!(near(corner(&out), [0, 0, 255]), "the fork draws: {:?}", corner(&out));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_effect_without_a_snapshot_fails_naming_it_unless_marked_missing() {
    let dir = scratch("unknown");
    let file = dir.join("p.json");
    let fx = json!([{ "id": "fx1", "type": "acme.glow", "on": true, "p": {} }]);
    std::fs::write(&file, project(fx).to_string()).unwrap();
    let out = dir.join("still.png");
    let run = cli(&[file.to_str().unwrap(), "-o", out.to_str().unwrap(), "--format", "still"]);
    assert_eq!(run.status.code(), Some(3), "{}", String::from_utf8_lossy(&run.stderr));
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
