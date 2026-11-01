#![allow(dead_code)]
use pm_first_launch::{Config, beta_profile};
use pm_native_migrate::indexeddb;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn write(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
pub fn var(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (n & 127) as u8;
        n >>= 7;
        out.push(b | if n > 0 { 128 } else { 0 });
        if n == 0 {
            return out;
        }
    }
}
fn data(b: &[u8]) -> Vec<u8> {
    let mut out = var(b.len() as u64);
    out.extend(b);
    out
}
fn utf16(s: &str) -> Vec<u8> {
    let mut out = var(s.encode_utf16().count() as u64);
    for c in s.encode_utf16() {
        out.extend(c.to_be_bytes());
    }
    out
}
fn string(s: &str) -> Vec<u8> {
    let mut out = vec![b'S'];
    out.extend(data(s.as_bytes()));
    out
}
pub fn log(path: &Path, payload: &[u8]) {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    assert!(payload.len() < 32761);
    let kind = 1;
    let checksum = crc32c::crc32c_append(crc32c::crc32c(&[kind]), payload)
        .rotate_right(15)
        .wrapping_add(0xa282ead8);
    f.write_all(&checksum.to_le_bytes()).unwrap();
    f.write_all(&(payload.len() as u16).to_le_bytes()).unwrap();
    f.write_all(&[kind]).unwrap();
    f.write_all(payload).unwrap();
    f.sync_all().unwrap();
}
pub fn database(path: &Path, cmp: &str, entries: &[(Vec<u8>, Vec<u8>)]) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("CURRENT"), b"MANIFEST-000001\n").unwrap();
    let mut m = vec![1];
    m.extend(data(cmp.as_bytes()));
    m.extend([2, 3, 3, 4, 4, 0]);
    log(&path.join("MANIFEST-000001"), &m);
    let mut batch = 1u64.to_le_bytes().to_vec();
    batch.extend((entries.len() as u32).to_le_bytes());
    for (k, v) in entries {
        batch.push(1);
        batch.extend(data(k));
        batch.extend(data(v));
    }
    log(&path.join("000003.log"), &batch);
}
pub fn media_db(profile: &Path, extra: bool) {
    let dir = profile.join("IndexedDB/app_powermove_0.indexeddb.leveldb");
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    let mut entries = Vec::new();
    let mut key = indexeddb::encode_prefix(0, 0, 0);
    key.push(0);
    entries.push((key, vec![5]));
    let mut key = indexeddb::encode_prefix(1, 0, 0);
    key.push(1);
    entries.push((key, utf16("powermove-media")[1..].to_vec()));
    let mut key = indexeddb::encode_prefix(1, 0, 0);
    key.extend([4]);
    entries.push((key, vec![2]));
    let mut key = indexeddb::encode_prefix(1, 0, 0);
    key.extend([50, 1, 0]);
    entries.push((key, utf16("assets")[1..].to_vec()));
    let mut blobs = vec![
        ("media:original", b"synthetic original".as_slice()),
        ("media:proxy", b"synthetic playback proxy".as_slice()),
        ("undo-only", b"synthetic undo only".as_slice()),
    ];
    if extra {
        blobs.push(("media:after-tr", b"synthetic later import"));
    }
    for (i, (id, bytes)) in blobs.iter().enumerate() {
        let number = i as u64 + 2;
        let file = profile.join(format!(
            "IndexedDB/app_powermove_0.indexeddb.blob/1/00/{number:x}"
        ));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
        let mut suffix = vec![1];
        suffix.extend(utf16(id));
        let mut key = indexeddb::encode_prefix(1, 1, 1);
        key.extend(&suffix);
        let mut value = vec![1, 255, 20, 255, 16, b'o'];
        value.extend(string("id"));
        value.extend(string(id));
        value.extend(string("blob"));
        value.extend([b'\\', b'i', 0]);
        value.extend(string("revision"));
        value.extend(string("synthetic-revision"));
        value.extend([b'{', 3]);
        entries.push((key, value));
        let mut key = indexeddb::encode_prefix(1, 1, 3);
        key.extend(suffix);
        let mut ext = vec![0];
        ext.extend(var(number));
        ext.extend(utf16("video/mp4"));
        ext.extend(var(bytes.len() as u64));
        entries.push((key, ext));
    }
    database(&dir, "idb_cmp1", &entries);
}
pub fn profile(root: &Path) -> Config {
    let source = root.join("Electron");
    fs::create_dir_all(source.join("store")).unwrap();
    media_db(&source, false);
    let fp = |number| {
        pm_native_migrate::migration::sampled_fingerprint(&source.join(format!(
            "IndexedDB/app_powermove_0.indexeddb.blob/1/00/{number:x}"
        )))
        .unwrap()
    };
    let original = json!({"id":"clip","storageKey":"media:original","fingerprint":fp(2),"revision":"synthetic-revision","kind":"video"});
    let proxy = json!({"id":"proxy","storageKey":"media:proxy","fingerprint":fp(2),"playbackProxy":true,"playbackProxyVersion":2,"revision":"synthetic-revision","kind":"video"});
    let offline = json!({"id":"offline","storageKey":"absent","fingerprint":format!("v2:10:{}","0".repeat(64)),"sourcePath":root.join("missing-original.mov"),"kind":"video"});
    let undo = json!({"id":"old","storageKey":"content-key","fingerprint":fp(4),"revision":"synthetic-revision","kind":"video"});
    let mut project = json!({"id":"p","name":"Synthetic project","w":200,"h":100,"fps":30,"dur":2,"assets":{"clip":original,"proxy":proxy,"offline":offline},"layers":[],"comps":{},"revision":1});
    // Undo contains only a field-level patch. The restored object must inherit
    // its id/metadata before reference discovery can see its legacy bare key.
    project["assets"]["old"] = undo;
    project["assets"]["old"]["storageKey"] = "absent-current".into();
    project["assets"]["old"]["id"] = "undo-only".into();
    write(
        &source.join("store/projects.json"),
        &json!([{"id":"p","name":"Synthetic project"}]),
    );
    write(
        &source.join("store/project.p.json"),
        &json!({"v":"1.2.0","proj":project}),
    );
    write(
        &source.join("store/projectHistory.p.json"),
        &json!({"version":1,"paths":"composition","index":0,"entries":[{"label":"Replace storage identity","forward":[{"path":["assets","old","storageKey"],"exists":true,"value":"absent-current"},{"path":["assets","old","id"],"exists":true,"value":"not-in-database"}],"backward":[{"path":["assets","old","storageKey"],"exists":true,"value":"content-key"},{"path":["assets","old","id"],"exists":true,"value":"undo-only"}]}]}),
    );
    // Avoid current bare fallback: only undo reconstruction finds undo-only.
    let mut v: Value =
        serde_json::from_slice(&fs::read(source.join("store/project.p.json")).unwrap()).unwrap();
    v["proj"]["assets"]["old"]["id"] = "not-in-database".into();
    write(&source.join("store/project.p.json"), &v);
    write(&source.join("store/theme.json"), &json!("dark"));
    write(
        &source.join("store/ext.synthetic.json"),
        &json!({"count":7,"unknown":true}),
    );
    write(
        &source.join("store/workspaces.json"),
        &json!([{"id":"w","name":"Synthetic workspace","layout":{}}]),
    );
    write(
        &source.join("store/agentThreads.p.json"),
        &json!({"threads":[{"id":"synthetic-thread","attachments":["a"]}]}),
    );
    write(
        &source.join("store/agentAttachment.a.json"),
        &json!({"data":"c3ludGhldGlj","mime":"text/plain"}),
    );
    write(
        &source.join("store/projectMeta.p.json"),
        &json!({"time":2,"unknown":true,"file":{"path":root.join("untouched.pmv")}}),
    );
    write(
        &source.join("extensions/synthetic/manifest.json"),
        &json!({"id":"synthetic","version":"1.0.0"}),
    );
    fs::write(
        source.join("extensions/synthetic/main.ts"),
        "// synthetic extension\n",
    )
    .unwrap();
    write(
        &source.join("Agent State/synthetic/.checkpoint.json"),
        &json!({"pending":true}),
    );
    fs::create_dir_all(source.join("Agent Workspaces/synthetic/artifacts")).unwrap();
    fs::write(
        source.join("Agent Workspaces/synthetic/artifacts/result.txt"),
        b"synthetic result",
    )
    .unwrap();
    let key = [
        b"_app://powermove\0".as_slice(),
        &[1],
        b"pm.easing.library.v1",
    ]
    .concat();
    database(
        &source.join("Local Storage/leveldb"),
        "leveldb.BytewiseComparator",
        &[(key, [&[1], b"{\"version\":1}".as_slice()].concat())],
    );
    Config {
        electron_profile: source,
        native_profile: beta_profile(&root.join("Native")),
    }
}
pub fn files(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, p: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(base, &e.path(), out);
            } else if e.file_type().unwrap().is_file() {
                out.insert(
                    e.path().strip_prefix(base).unwrap().into(),
                    fs::read(e.path()).unwrap(),
                );
            }
        }
    }
    let mut out = Default::default();
    walk(root, root, &mut out);
    out
}
pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
pub fn checkpoint(config: &Config) -> pm_first_launch::Checkpoint {
    read(
        &config
            .native_profile
            .join("first-launch-checkpoint/checkpoint.json"),
    )
}
