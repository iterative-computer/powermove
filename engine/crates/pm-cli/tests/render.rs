//! `pm-cli render` end to end: a JSON project with linked media and a PMV3
//! project with embedded media render to verified files; projects the engine
//! cannot render exit 3 without writing anything; existing files are kept
//! unless `--force`.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixture(name: &str) -> PathBuf {
    repo().join("apps/desktop/e2e/fixtures").join(name)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-cli-render-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn layer(id: &str, ty: &str, d: Value, x: f64, y: f64) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in [
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("position.x", x),
        ("position.y", y),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("rotation", 0.0),
        ("opacity", 100.0),
        ("skew", 0.0),
    ] {
        p.insert(k.into(), ch(json!(v)));
    }
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": 1, "on": true, "solo": false,
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "threeD": false, "p": p, "fx": [], "masks": [], "d": d,
        "transitionIn": null, "transitionOut": null
    })
}

/// A 1-second 320 × 180 project: a PNG over a solid, a WAV tone.
fn project(source_paths: bool, with_text: bool) -> Value {
    let mut layers = vec![];
    if with_text {
        layers.push(layer("t", "text", json!({ "text": "Routed" }), 10.0, 10.0));
    }
    layers.push(layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "contain", "w": 160, "h": 90 }),
        80.0,
        45.0,
    ));
    let mut tone = layer(
        "tone",
        "audio",
        json!({ "asset": "tone", "gain": 1, "trim": 0, "fadeIn": 0, "fadeOut": 0 }),
        0.0,
        0.0,
    );
    tone["from"] = json!(0);
    layers.push(tone);
    layers.push(layer(
        "bg",
        "solid",
        json!({ "color": "#3FCF8E", "w": 320, "h": 180 }),
        0.0,
        0.0,
    ));
    let mut still = json!({ "id": "still", "name": "still-alpha.png", "kind": "image", "format": "png", "w": 64, "h": 64 });
    let mut tone_asset =
        json!({ "id": "tone", "name": "tone.wav", "kind": "audio", "format": "wav", "dur": 2 });
    if source_paths {
        still["sourcePath"] = json!(fixture("still-alpha.png"));
        tone_asset["sourcePath"] = json!(fixture("tone.wav"));
    }
    json!({
        "id": "P1", "name": "cli", "w": 320, "h": 180, "fps": 25, "dur": 1, "work": [0.2, 0.6],
        "compId": "P1-root",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s", "color": "#000000", "position": 0 }] },
        "layers": layers, "assets": { "still": still, "tone": tone_asset }, "comps": {}
    })
}

/// A PMV3 file: magic, header length, `{document, media}`, media bytes.
fn pmv3(document: &Value, media: &[(&str, &str, Vec<u8>)]) -> Vec<u8> {
    let mut offset = 0usize;
    let mut records = vec![];
    for (id, ty, data) in media {
        records.push(json!({ "id": id, "type": ty, "offset": offset, "length": data.len() }));
        offset += data.len();
    }
    let header =
        serde_json::to_vec(&json!({ "document": { "proj": document, "v": 1 }, "media": records }))
            .unwrap();
    let mut out = b"PMV3\n".to_vec();
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    for (_, _, data) in media {
        out.extend_from_slice(data);
    }
    out
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pm-cli"))
        .arg("render")
        .args(args)
        .output()
        .unwrap()
}

fn summary(out: &std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn missing_media_exports_unattended_and_reports_clips() {
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Clean(scratch("missing-media"));
    let project_file = dir.0.join("p.json");
    std::fs::write(&project_file, project(false, false).to_string()).unwrap();
    let output = dir.0.join("without.mp4");
    let result = summary(&cli(&[
        project_file.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--preset",
        "web",
        "--no-audio",
    ]));
    assert_eq!(
        result["missingMedia"],
        json!([
            { "asset": "still", "name": "still-alpha.png" },
            { "asset": "tone", "name": "tone.wav" }
        ])
    );
    assert!(output.is_file());
}

#[test]
fn json_projects_with_linked_media_render_to_verified_files() {
    let dir = scratch("json");
    let project_file = dir.join("p.json");
    std::fs::write(&project_file, project(true, false).to_string()).unwrap();
    let out_file = dir.join("master.mov");
    let s = summary(&cli(&[
        project_file.to_str().unwrap(),
        "-o",
        out_file.to_str().unwrap(),
    ]));
    assert_eq!(s["codec"], "apch", "{s}");
    assert_eq!(s["frames"], 25);
    assert_eq!(s["audioSamples"], 48_000);
    assert_eq!(s["colr"], json!([1, 13, 1]));
    // The work area at 50 fps, half size, as H.264 with the broadcast tags.
    let web = dir.join("web.mp4");
    let s = summary(&cli(&[
        project_file.to_str().unwrap(),
        "-o",
        web.to_str().unwrap(),
        "--preset",
        "web",
        "--range",
        "work",
        "--fps",
        "50",
        "--scale",
        "0.5",
        "--color",
        "broadcast",
    ]));
    assert_eq!(s["codec"], "avc1");
    assert_eq!(s["frames"], 20, "0.4 s at 50 fps");
    assert_eq!(s["size"], json!([160, 90]));
    assert_eq!(s["colr"], json!([1, 1, 1]));
    // An existing file is kept without --force, replaced with it.
    let before = std::fs::read(&web).unwrap();
    let again = cli(&[
        project_file.to_str().unwrap(),
        "-o",
        web.to_str().unwrap(),
        "--preset",
        "web",
    ]);
    assert_eq!(
        again.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(std::fs::read(&web).unwrap(), before);
    summary(&cli(&[
        project_file.to_str().unwrap(),
        "-o",
        web.to_str().unwrap(),
        "--preset",
        "web",
        "--force",
    ]));
    assert_ne!(std::fs::read(&web).unwrap(), before);
    // ProRes is written to MOV only.
    let wrong = cli(&[
        project_file.to_str().unwrap(),
        "-o",
        dir.join("x.mp4").to_str().unwrap(),
    ]);
    assert_eq!(wrong.status.code(), Some(2));
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(names.is_empty(), "temporary files left: {names:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pmv_projects_render_from_embedded_media() {
    let dir = scratch("pmv");
    let file = dir.join("p.pmv");
    let bytes = pmv3(
        &project(false, false),
        &[
            (
                "still",
                "image/png",
                std::fs::read(fixture("still-alpha.png")).unwrap(),
            ),
            (
                "tone",
                "audio/wav",
                std::fs::read(fixture("tone.wav")).unwrap(),
            ),
        ],
    );
    std::fs::write(&file, bytes).unwrap();
    let out_file = dir.join("alpha.mov");
    let s = summary(&cli(&[
        file.to_str().unwrap(),
        "-o",
        out_file.to_str().unwrap(),
        "--preset",
        "master-alpha",
    ]));
    assert_eq!(s["codec"], "ap4h");
    assert_eq!(s["pixelFormat"], "4:4:4:4");
    assert_eq!(s["audioSamples"], 48_000);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn projects_the_engine_cannot_render_exit_3_and_write_nothing() {
    // pm-cli has fonts, so text no longer refuses. A project color setting
    // this engine does not know (one from a later version) still does
    // (`Feature::ProjectColor`), and always will: drawing it with a guess
    // would differ from the app that wrote it.
    let dir = scratch("routed");
    let mut p = project(true, true);
    p["color"] = json!({ "mode": "a-mode-from-a-later-version" });
    let project_file = dir.join("p.json");
    std::fs::write(&project_file, p.to_string()).unwrap();
    let out = cli(&[
        project_file.to_str().unwrap(),
        "-o",
        dir.join("x.mov").to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("project color"), "{stderr}");
    assert!(!stderr.contains("text"), "text no longer refuses: {stderr}");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "only the project"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn text_projects_render_with_the_hosts_fonts() {
    let dir = scratch("text");
    let project_file = dir.join("p.json");
    let mut text_project = project(false, false);
    text_project["assets"] = json!({});
    text_project["layers"] = json!([layer(
        "t",
        "text",
        json!({"text":"Visible text", "font":"system-ui", "size":48, "weight":400, "color":"#ffffff"}),
        20.0,
        20.0
    )]);
    std::fs::write(&project_file, text_project.to_string()).unwrap();
    let out_file = dir.join("text.png");
    let s = summary(&cli(&[
        project_file.to_str().unwrap(),
        "-o",
        out_file.to_str().unwrap(),
        "--format",
        "still",
    ]));
    assert!(s["frames"].as_u64().unwrap_or(0) > 0, "{s}");
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&out_file).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    let channels = info.color_type.samples();
    let bright = bytes[..info.buffer_size()]
        .chunks_exact(channels)
        .filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
        .count();
    assert!(
        bright > 100,
        "text must contribute white ink over the black background, found {bright} bright pixels"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_preset_and_the_phase_3b_formats_render_from_the_cli() {
    let dir = scratch("formats");
    let project_file = dir.join("p.json");
    std::fs::write(&project_file, project(true, false).to_string()).unwrap();
    let p = project_file.to_str().unwrap();
    let out = |name: &str| dir.join(name).to_str().unwrap().to_owned();
    // Web · HEVC.
    let s = summary(&cli(&[p, "-o", &out("web.mp4"), "--preset", "web-hevc"]));
    assert_eq!(
        (s["codec"].as_str(), s["audioCodec"].as_str()),
        (Some("hvc1"), Some("mp4a")),
        "{s}"
    );
    // HEVC 10-bit in MOV with float PCM and a start timecode and reel.
    let s = summary(&cli(&[
        p,
        "-o",
        &out("hevc10.mov"),
        "--format",
        "hevc10",
        "--audio",
        "float",
        "--timecode",
        "01:00:00:00",
        "--reel",
        "A001",
    ]));
    assert_eq!(s["codec"], "hvc1");
    assert_eq!(s["pixelFormat"], "4:2:0");
    assert_eq!(
        s["timecode"]["startFrame"], 90_000,
        "one hour at 25 fps: {s}"
    );
    assert_eq!(s["timecode"]["reel"], "A001");
    // WebM.
    let s = summary(&cli(&[p, "-o", &out("web.webm"), "--format", "vp9"]));
    assert_eq!(
        (s["codec"].as_str(), s["audioCodec"].as_str()),
        (Some("vp09"), Some("opus")),
        "{s}"
    );
    assert_eq!(s["audioSamples"], 48_000);
    // Image sequence · PNG: a new folder of frames named after it.
    let s = summary(&cli(&[p, "-o", &out("Frames"), "--preset", "png"]));
    assert_eq!(
        (s["codec"].as_str(), s["frames"].as_u64()),
        (Some("png"), Some(25)),
        "{s}"
    );
    assert!(dir.join("Frames/Frames_00024.png").is_file());
    // A 16-bit still.
    let s = summary(&cli(&[p, "-o", &out("still.png"), "--format", "still16"]));
    assert_eq!(s["frames"], 1);
    // Extensions must match the format.
    assert_eq!(
        cli(&[p, "-o", &out("x.mp4"), "--format", "vp9"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        cli(&[p, "-o", &out("x.png"), "--preset", "png"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        cli(&[
            p,
            "-o",
            &out("x.mp4"),
            "--audio",
            "pcm24",
            "--preset",
            "web"
        ])
        .status
        .code(),
        Some(1)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

struct SvgProject(PathBuf);

impl Drop for SvgProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn svg_project(name: &str) -> SvgProject {
    let fixture = SvgProject(scratch(name));
    let dir = &fixture.0;
    let svg = dir.join("art.svg");
    std::fs::write(
        &svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32"><rect width="64" height="32" rx="8" fill="#ff5a1f"/><circle cx="48" cy="16" r="8" fill="#fff"/></svg>"##,
    )
    .unwrap();
    let mut p = project(true, false);
    p["assets"]["still"] = json!({
        "id": "still", "name": "art.svg", "kind": "image", "format": "svg", "w": 64, "h": 32,
        "sourcePath": svg,
    });
    let project_file = dir.join("p.json");
    std::fs::write(&project_file, p.to_string()).unwrap();
    fixture
}

/// SVG images are drawn natively: a bare pm-cli with no Chromium helper
/// (none configured, none beside it) renders an SVG project, and `scope`
/// reads it too. The SVG (a 64 × 32 orange rounded rect with a white
/// circle) is `contain`ed in a 160 × 90 box at the top left: 2.5×.
#[test]
fn svg_images_render_without_chromium() {
    let fixture = svg_project("svg-native");
    let dir = &fixture.0;
    let project_file = dir.join("p.json");
    let still = dir.join("svg.png");
    let bare = |command: &str, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_pm-cli"))
            .arg(command)
            .args(args)
            .env_remove("POWERMOVE_ANGLE_ENDPOINT")
            .env_remove("POWERMOVE_ANGLE_TOKEN")
            .env("POWERMOVE_ANGLE_HELPER", dir.join("missing-helper.cjs"))
            .output()
            .unwrap()
    };
    let out = bare(
        "render",
        &[
            project_file.to_str().unwrap(),
            "-o",
            still.to_str().unwrap(),
            "--format",
            "still",
        ],
    );
    let s = summary(&out);
    assert!(s.get("approximateSvg").is_none(), "{s}");
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&still).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!((info.width, info.height), (320, 180));
    let channels = info.color_type.samples();
    let at = |x: usize, y: usize| {
        let i = (y * 320 + x) * channels;
        [bytes[i], bytes[i + 1], bytes[i + 2]]
    };
    // Within 1 code value: an 8-bit export is dithered.
    let near = |x: usize, y: usize, rgb: [u8; 3]| {
        let p = at(x, y);
        assert!(
            p.iter().zip(rgb).all(|(a, b)| a.abs_diff(b) <= 1),
            "({x}, {y}): {p:?}, expected {rgb:?}"
        );
    };
    let (orange, white, solid) = ([0xff, 0x5a, 0x1f], [0xff; 3], [0x3f, 0xcf, 0x8e]);
    // The rect's interior, the circle's center (48, 16) → (120, 45), and the
    // solid beside the box.
    near(40, 45, orange);
    near(120, 45, white);
    near(240, 45, solid);
    // The rect spans y 5..85 (32 × 2.5, centered in 90), its corners
    // rounded (rx 8 → 20 px): the box's corner pixel is the solid's.
    near(0, 5, solid);
    near(80, 4, solid);
    near(80, 5, orange);
    let scopes = dir.join("scopes");
    let out = bare(
        "scope",
        &[project_file.to_str().unwrap(), "-o", scopes.to_str().unwrap()],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(scopes.join("scope.json").exists());
}
