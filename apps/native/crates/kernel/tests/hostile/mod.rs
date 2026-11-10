//! Hostile project inputs, generated from the public engine corpus
//! (`engine/crates/pm-model/tests/fixtures/corpus`). Nothing generated is
//! committed: `hostile_open` builds the cases in memory, and
//! `PM_HOSTILE_OUT=<dir> cargo test -p pm-kernel --test hostile_open write_corpus -- --ignored`
//! writes them to a folder for the real-app pass (`scripts/stability/open-all.py`).
#![allow(dead_code)]

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// How the case reaches the open path.
#[derive(Clone, Debug)]
pub enum Shape {
    /// An ordinary file with these bytes.
    File(Vec<u8>),
    /// A file whose permissions forbid reading.
    Unreadable(Vec<u8>),
    /// A read-only file (it must open; saving goes elsewhere).
    ReadOnly(Vec<u8>),
    /// A folder named like a project.
    Directory,
    /// A named pipe with no writer: reading it would block forever.
    Fifo,
    /// A path that doesn't exist (a vanished volume or deleted file).
    Missing,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub shape: Shape,
}

pub fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../engine/crates/pm-model/tests/fixtures/corpus")
}

/// The public corpus projects, sorted by name.
pub fn seeds() -> Vec<(String, Value)> {
    let mut out: Vec<_> = std::fs::read_dir(corpus_dir())
        .expect("engine corpus fixtures")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            (name, value)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Deterministic xorshift, so every run builds the same cases.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Every path to a value in the tree (object keys and array indices).
fn paths(v: &Value, here: &mut Vec<Step>, out: &mut Vec<Vec<Step>>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                here.push(Step::Key(k.clone()));
                out.push(here.clone());
                paths(c, here, out);
                here.pop();
            }
        }
        Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                here.push(Step::Index(i));
                out.push(here.clone());
                paths(c, here, out);
                here.pop();
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug)]
enum Step {
    Key(String),
    Index(usize),
}

fn at<'a>(v: &'a mut Value, path: &[Step]) -> Option<&'a mut Value> {
    let mut cur = v;
    for s in path {
        cur = match (s, cur) {
            (Step::Key(k), Value::Object(m)) => m.get_mut(k)?,
            (Step::Index(i), Value::Array(a)) => a.get_mut(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn remove(v: &mut Value, path: &[Step]) {
    let (last, parent) = path.split_last().unwrap();
    match (last, at(v, parent)) {
        (Step::Key(k), Some(Value::Object(m))) => {
            m.remove(k);
        }
        (Step::Index(i), Some(Value::Array(a))) if *i < a.len() => {
            a.remove(*i);
        }
        _ => {}
    }
}

/// Numbers JSON can carry but serde_json can't hold (overflow to infinity,
/// underflow, negative zero) are spliced into the text through this marker.
const RAW: &str = "\u{1}raw:";
fn raw(text: &str) -> Value {
    Value::String(format!("{RAW}{text}"))
}
fn encode(v: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string(v).unwrap();
    while let Some(i) = s.find("\"\\u0001raw:") {
        let bytes = s.as_bytes();
        let mut j = i + 1;
        while bytes[j] != b'"' {
            j += if bytes[j] == b'\\' { 2 } else { 1 };
        }
        let lit: String = serde_json::from_str(&s[i..=j]).unwrap();
        s.replace_range(i..=j, &lit[RAW.len()..]);
    }
    s.into_bytes()
}

/// Wrong-typed and extreme replacements for any value.
fn hostile_values() -> Vec<(&'static str, Value)> {
    vec![
        ("null", Value::Null),
        ("string", json!("not a number")),
        ("number", json!(7)),
        ("bool", json!(true)),
        ("object", json!({"x": 1})),
        ("array", json!([1, "two", null])),
        ("neg", json!(-1)),
        ("zero", json!(0)),
        ("huge", json!(1e308)),
        ("neghuge", json!(-1e308)),
        ("inf", raw("1e400")),
        ("neginf", raw("-1e400")),
        ("subnormal", raw("5e-324")),
        ("negzero", raw("-0")),
        ("i64max", json!(9223372036854775807i64)),
        ("frac", json!(0.3333333333333333)),
        ("longstr", json!("x".repeat(1 << 20))),
        ("emptystr", json!("")),
        ("lone_surrogate", raw("\"\\ud800\"")),
        ("deep", deep(400)),
    ]
}

fn deep(n: usize) -> Value {
    let mut v = json!(0);
    for _ in 0..n {
        v = json!([v]);
    }
    v
}

/// Project-level edits a real file can carry (old versions, hand edits,
/// sync conflicts, other tools).
fn targeted(seed: &Value) -> Vec<(&'static str, Value)> {
    let mut out: Vec<(&'static str, Value)> = vec![];
    let mut push = |name, f: &dyn Fn(&mut Value)| {
        let mut v = seed.clone();
        f(&mut v);
        out.push((name, v));
    };
    let layers = |v: &mut Value, f: &dyn Fn(&mut Map<String, Value>)| {
        if let Some(Value::Array(a)) = v.get_mut("layers") {
            for l in a.iter_mut().filter_map(Value::as_object_mut) {
                f(l);
            }
        }
        if let Some(Value::Object(comps)) = v.get_mut("comps") {
            for c in comps.values_mut() {
                if let Some(Value::Array(a)) = c.get_mut("layers") {
                    for l in a.iter_mut().filter_map(Value::as_object_mut) {
                        f(l);
                    }
                }
            }
        }
    };
    push("no-layers", &|v| {
        v.as_object_mut().unwrap().remove("layers");
    });
    push("no-id", &|v| {
        v.as_object_mut().unwrap().remove("id");
    });
    push("no-comps", &|v| {
        v.as_object_mut().unwrap().remove("comps");
    });
    push("layers-object", &|v| v["layers"] = json!({"a": 1}));
    push("zero-size", &|v| {
        v["w"] = json!(0);
        v["h"] = json!(0);
    });
    push("huge-size", &|v| {
        v["w"] = json!(1e9);
        v["h"] = json!(1e9);
    });
    push("neg-size", &|v| {
        v["w"] = json!(-1920);
        v["h"] = json!(-1080);
    });
    push("zero-fps", &|v| v["fps"] = json!(0));
    push("huge-fps", &|v| v["fps"] = json!(1e9));
    push("inf-dur", &|v| v["dur"] = raw("1e400"));
    push("neg-dur", &|v| v["dur"] = json!(-5));
    push("huge-dur", &|v| v["dur"] = json!(1e300));
    push("unknown-layer-type", &|v| {
        layers(v, &|l| {
            l.insert("type".into(), json!("hologram"));
        })
    });
    push("layer-without-type", &|v| {
        layers(v, &|l| {
            l.remove("type");
        })
    });
    push("layer-without-id", &|v| {
        layers(v, &|l| {
            l.remove("id");
        })
    });
    push("duplicate-layer-ids", &|v| {
        layers(v, &|l| {
            l.insert("id".into(), json!("same"));
        })
    });
    push("self-parent", &|v| {
        layers(v, &|l| {
            let id = l.get("id").cloned().unwrap_or(Value::Null);
            l.insert("parent".into(), id.clone());
            l.insert("group".into(), id.clone());
            l.insert("matteSource".into(), id);
        })
    });
    push("missing-parent", &|v| {
        layers(v, &|l| {
            l.insert("parent".into(), json!("nobody"));
            l.insert("group".into(), json!("nowhere"));
        })
    });
    push("huge-times", &|v| {
        layers(v, &|l| {
            l.insert("from".into(), json!(1e300));
            l.insert("dur".into(), json!(-1e300));
        })
    });
    push("nan-times", &|v| {
        layers(v, &|l| {
            l.insert("from".into(), raw("1e400"));
            l.insert("dur".into(), raw("-1e400"));
        })
    });
    push("keyframes-garbage", &|v| {
        layers(v, &|l| {
            if let Some(Value::Object(p)) = l.get_mut("p") {
                for prop in p.values_mut() {
                    prop["kf"] = json!([
                        {"t": raw("1e400"), "v": "x"},
                        {"t": -1, "v": {"o": 1}},
                        {"t": 1, "v": [1, 2, 3, 4, 5, 6, 7]},
                        {"t": 1},
                        null,
                        7
                    ]);
                    prop["expr"] = json!("while(true){}");
                }
            }
        })
    });
    push("props-wrong-types", &|v| {
        layers(v, &|l| {
            if let Some(Value::Object(p)) = l.get_mut("p") {
                for (i, prop) in p.values_mut().enumerate() {
                    *prop = match i % 4 {
                        0 => json!("text"),
                        1 => json!(null),
                        2 => json!({"v": [null]}),
                        _ => json!({"v": raw("1e400"), "kf": "no"}),
                    };
                }
            }
        })
    });
    push("data-wrong-types", &|v| {
        layers(v, &|l| {
            l.insert("d".into(), json!([1, 2]));
        })
    });
    push("missing-media", &|v| {
        if let Some(Value::Object(assets)) = v.get_mut("assets") {
            for a in assets.values_mut() {
                a["path"] = json!("/Volumes/Gone/missing.mov");
                a["src"] = json!("file:///Volumes/Gone/missing.mov");
            }
        }
        layers(v, &|l| {
            if let Some(d) = l.get_mut("d").and_then(Value::as_object_mut) {
                d.insert("asset".into(), json!("no-such-asset"));
            }
        })
    });
    push("missing-fonts", &|v| {
        layers(v, &|l| {
            if let Some(d) = l.get_mut("d").and_then(Value::as_object_mut) {
                d.insert("font".into(), json!("No Such Font Family"));
                d.insert("fontFamily".into(), json!("No Such Font Family"));
                d.insert("weight".into(), json!(1234));
            }
        })
    });
    push("missing-extensions", &|v| {
        layers(v, &|l| {
            l.insert("type".into(), json!("ext:com.example.missing/thing"));
            l.insert(
                "fx".into(),
                json!([{"id": "fx1", "type": "ext:com.example.missing/glow", "on": true, "p": {}}]),
            );
        })
    });
    push("precomp-self-cycle", &|v| {
        let comp = v.get("compId").cloned().unwrap_or(json!("root"));
        layers(v, &|l| {
            l.insert("type".into(), json!("precomp"));
            let d = l.entry("d").or_insert(json!({}));
            if !d.is_object() {
                *d = json!({});
            }
            d["comp"] = comp.clone();
            d["compId"] = comp.clone();
        })
    });
    push("precomp-mutual-cycle", &|v| {
        let ids: Vec<String> = v
            .get("comps")
            .and_then(Value::as_object)
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        let root = v.get("compId").cloned().unwrap_or(json!("root"));
        let mut targets = ids.iter().map(|i| json!(i)).collect::<Vec<_>>();
        targets.push(root);
        let n = std::cell::Cell::new(0usize);
        layers(v, &|l| {
            l.insert("type".into(), json!("precomp"));
            let d = l.entry("d").or_insert(json!({}));
            if !d.is_object() {
                *d = json!({});
            }
            let t = targets[n.get() % targets.len()].clone();
            n.set(n.get() + 1);
            d["comp"] = t.clone();
            d["compId"] = t;
        })
    });
    // Long-form scale (plan §1: 5k clips), on a few seeds only: it's slow
    // in debug builds.
    let heavy = [
        "3d-legacy-scene",
        "shapes-editable-paths",
        "text-range-styles",
    ]
    .iter()
    .any(|n| seed.get("id").and_then(Value::as_str) == Some(&format!("P{n}")));
    if heavy {
        push("many-layers", &|v| {
            if let Some(Value::Array(a)) = v.get_mut("layers")
                && let Some(first) = a.first().cloned()
            {
                for i in 0..5000 {
                    let mut l = first.clone();
                    l["id"] = json!(format!("bulk{i}"));
                    a.push(l);
                }
            }
        });
    }
    push("history-garbage", &|v| {
        v["history"] = json!({"version": 99, "index": 1e9, "entries": "nope"})
    });
    out
}

/// PMV3: magic, little-endian u32 header length, header JSON, media bytes.
fn pmv3(document: &[u8]) -> Vec<u8> {
    let header = [
        b"{\"document\":".as_slice(),
        document,
        b",\"media\":[]}".as_slice(),
    ]
    .concat();
    let mut out = b"PMV3\n".to_vec();
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out
}

fn record(data: &[u8], offset: usize) -> Value {
    json!({"offset": offset, "length": data.len(), "sha256": hex(&Sha256::digest(data))})
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// PMV4 as `generate-portable.py` writes it, with optional damage.
fn pmv4(
    document: &[u8],
    media: &[(&str, &[u8])],
    index_patch: Option<&dyn Fn(&mut Value)>,
) -> Vec<u8> {
    let mut body = b"PMV4\n".to_vec();
    body.extend_from_slice(document);
    let mut records = vec![];
    for (id, data) in media {
        let mut r = record(data, body.len());
        r["id"] = json!(id);
        r["revision"] = json!("v1");
        r["type"] = json!("application/octet-stream");
        records.push(r);
        body.extend_from_slice(data);
    }
    let mut index = json!({"version": 4, "document": record(document, 5), "media": records});
    if let Some(p) = index_patch {
        p(&mut index);
    }
    let index = serde_json::to_vec(&index).unwrap();
    let mut out = body.clone();
    out.extend_from_slice(&index);
    out.extend_from_slice(b"PMV4END\n");
    out.extend_from_slice(&(body.len() as f64).to_le_bytes());
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&Sha256::digest(&index));
    out
}

/// Every hostile case. `random_per_seed` structural mutations per corpus
/// project, on top of the targeted, container and file-system cases.
pub fn cases(random_per_seed: usize) -> Vec<Case> {
    let mut out = vec![];
    let file = |name: String, bytes: Vec<u8>| Case {
        name,
        shape: Shape::File(bytes),
    };
    // File-system shapes.
    let small = serde_json::to_vec(&seeds()[0].1).unwrap();
    out.push(Case {
        name: "fs-unreadable".into(),
        shape: Shape::Unreadable(small.clone()),
    });
    out.push(Case {
        name: "fs-read-only".into(),
        shape: Shape::ReadOnly(small.clone()),
    });
    out.push(Case {
        name: "fs-directory".into(),
        shape: Shape::Directory,
    });
    out.push(Case {
        name: "fs-fifo".into(),
        shape: Shape::Fifo,
    });
    out.push(Case {
        name: "fs-missing".into(),
        shape: Shape::Missing,
    });
    // Bytes that aren't a project.
    out.push(file("bytes-empty".into(), vec![]));
    out.push(file(
        "bytes-binary".into(),
        (0..4096u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
            .collect(),
    ));
    out.push(file("bytes-json-scalar".into(), b"42".to_vec()));
    out.push(file("bytes-json-array".into(), b"[]".to_vec()));
    out.push(file("bytes-json-empty-object".into(), b"{}".to_vec()));
    out.push(file(
        "bytes-utf16-bom".into(),
        [&[0xFFu8, 0xFE][..], &small].concat(),
    ));
    out.push(file(
        "bytes-nesting-512".into(),
        format!("{}{}", "[".repeat(512), "]".repeat(512)).into_bytes(),
    ));
    out.push(file(
        "bytes-nesting-100k".into(),
        "[".repeat(100_000).into_bytes(),
    ));
    // Containers.
    out.push(file("pmv3-magic-only".into(), b"PMV3\n".to_vec()));
    out.push(file("pmv4-magic-only".into(), b"PMV4\n".to_vec()));
    let mut liar = b"PMV3\n".to_vec();
    liar.extend_from_slice(&u32::MAX.to_le_bytes());
    liar.extend_from_slice(b"{}");
    out.push(file("pmv3-header-length-lies".into(), liar));
    out.push(file("pmv3-ok".into(), pmv3(&small)));
    out.push(file(
        "pmv4-ok".into(),
        pmv4(&small, &[("m", b"bytes")], None),
    ));
    let mut bad = pmv4(&small, &[], None);
    let n = bad.len();
    bad[n - 1] ^= 0xFF;
    out.push(file("pmv4-bad-index-checksum".into(), bad));
    out.push(file(
        "pmv4-bad-document-checksum".into(),
        pmv4(
            &small,
            &[],
            Some(&|i: &mut Value| i["document"]["sha256"] = json!("0".repeat(64))),
        ),
    ));
    out.push(file(
        "pmv4-media-out-of-range".into(),
        pmv4(
            &small,
            &[("m", b"bytes")],
            Some(&|i: &mut Value| {
                i["media"][0]["offset"] = json!(1e15);
                i["media"][0]["length"] = json!(1e15);
            }),
        ),
    ));
    out.push(file(
        "pmv4-media-checksum-wrong".into(),
        pmv4(
            &small,
            &[("m", b"bytes")],
            Some(&|i: &mut Value| i["media"][0]["sha256"] = json!("f".repeat(64))),
        ),
    ));
    out.push(file(
        "pmv4-many-media".into(),
        pmv4(
            &small,
            &[],
            Some(&|i: &mut Value| {
                i["media"] = Value::Array(
                    (0..200_000)
                        .map(|n| json!({"id": n, "offset": 5, "length": 1, "sha256": "0"}))
                        .collect(),
                )
            }),
        ),
    ));
    let whole = pmv4(&small, &[("m", &[7u8; 4096])], None);
    for cut in [6, whole.len() / 2, whole.len() - 1] {
        out.push(file(format!("pmv4-truncated-{cut}"), whole[..cut].to_vec()));
    }
    // Per corpus project.
    for (si, (seed_name, seed)) in seeds().iter().enumerate() {
        let text = serde_json::to_vec(seed).unwrap();
        for frac in [1usize, 2, 9] {
            out.push(file(
                format!("{seed_name}-truncated-{frac}0pct"),
                text[..text.len() * frac / 10].to_vec(),
            ));
        }
        for (name, v) in targeted(seed) {
            out.push(file(format!("{seed_name}-{name}"), encode(&v)));
        }
        let mut all = vec![];
        paths(seed, &mut vec![], &mut all);
        let values = hostile_values();
        let mut rng = Rng(0x9E3779B97F4A7C15 ^ (si as u64 + 1));
        for n in 0..random_per_seed {
            let path = &all[rng.pick(all.len())];
            let mut v = seed.clone();
            let label = if rng.pick(5) == 0 {
                remove(&mut v, path);
                "removed".to_owned()
            } else {
                let (vn, value) = &values[rng.pick(values.len())];
                if let Some(slot) = at(&mut v, path) {
                    *slot = value.clone();
                }
                (*vn).to_owned()
            };
            let at_label: String = path
                .iter()
                .map(|s| match s {
                    Step::Key(k) => k
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .take(12)
                        .collect(),
                    Step::Index(i) => i.to_string(),
                })
                .collect::<Vec<String>>()
                .join(".");
            out.push(file(
                format!("{seed_name}-r{n:02}-{label}-{at_label}"),
                encode(&v),
            ));
        }
    }
    out
}

/// Materializes a case at `dir/<name>.pmv`; returns its path. A
/// `Directory`/`Fifo`/`Missing` case gets that shape instead of a file.
pub fn write(case: &Case, dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(format!("{}.pmv", case.name));
    match &case.shape {
        Shape::File(b) => std::fs::write(&path, b).unwrap(),
        Shape::Unreadable(b) => {
            std::fs::write(&path, b).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        Shape::ReadOnly(b) => {
            std::fs::write(&path, b).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        Shape::Directory => std::fs::create_dir_all(&path).unwrap(),
        Shape::Fifo => {
            let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0, "mkfifo");
        }
        Shape::Missing => {}
    }
    path
}
