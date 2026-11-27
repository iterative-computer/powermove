//! S079–S082 publish payload: the registry's git snapshot of an extension
//! folder, its loose objects and commit, and the secret and permission scans
//! the shipped app ran before uploading (v1.2.0 packages/registry snapshot.ts,
//! git/objects.ts, scan.ts and main/cloud/permission-scan.ts). Pure CPU.
use regex::Regex;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, sync::LazyLock};

/// `REGISTRY_LIMITS` of the shipped registry.
pub const MAX_FILES: usize = 400;
pub const MAX_TREE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PATH_BYTES: usize = 200;
/// Commit identities use the handle at this domain (v1 `HANDLE_MAIL_DOMAIN`).
pub const HANDLE_MAIL_DOMAIN: &str = "users.trypowermove.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Blob,
    Tree,
    Commit,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
        }
    }
}
fn object_bytes(kind: Kind, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("{} {}\0", kind.name(), body.len()).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
/// The git object id: SHA-1 of `<type> <size>\0<body>`.
pub fn hash_object(kind: Kind, body: &[u8]) -> String {
    hex(&Sha1::digest(object_bytes(kind, body)))
}
/// A zlib-deflated loose object, as the registry's upload route takes it.
pub fn encode_loose(kind: Kind, body: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(&object_bytes(kind, body));
    encoder.finish().unwrap_or_default()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotProblem {
    BadPath,
    PathTooLong,
    DuplicatePath,
    CaseCollision,
    FileDirCollision,
    TooManyFiles,
    FileTooLarge,
    TreeTooLarge,
    EmptyTree,
}
impl SnapshotProblem {
    /// v1.2.0 `snapshotText`, for the person publishing.
    pub fn text(self, path: &str) -> String {
        match self {
            Self::BadPath => {
                format!("“{path}” isn’t a name the Store accepts. Rename it and try again.")
            }
            Self::PathTooLong => format!("“{path}” has too long a name. Shorten it and try again."),
            Self::DuplicatePath | Self::CaseCollision => {
                format!("Two files are named “{path}” apart from letter case. Rename one.")
            }
            Self::FileDirCollision => format!("“{path}” is both a file and a folder. Rename one."),
            Self::TooManyFiles => format!("The Store takes up to {MAX_FILES} files."),
            Self::FileTooLarge => {
                format!("“{path}” is larger than 2 MB, the most one file can be.")
            }
            Self::TreeTooLarge => {
                "The extension is larger than 8 MB, the most the Store takes.".into()
            }
            Self::EmptyTree => "This extension has no files to publish.".into(),
        }
    }
}
pub struct Input {
    pub path: String,
    pub bytes: Vec<u8>,
}
pub struct Object {
    pub sha: String,
    pub kind: Kind,
    pub body: Vec<u8>,
}
pub struct Snapshot {
    pub tree_sha: String,
    pub files: Vec<(String, String, usize)>,
    pub objects: Vec<Object>,
    pub total_bytes: usize,
}
/// Dot-segments and `node_modules` never publish.
pub fn excluded(path: &str) -> bool {
    path.split('/')
        .any(|s| s.starts_with('.') || s == "node_modules")
}
#[derive(Default)]
struct Node {
    files: BTreeMap<String, String>,
    dirs: BTreeMap<String, Node>,
}
/// The registry's tree of `inputs`: excluded paths dropped, limits checked,
/// blobs and trees in upload order.
pub fn snapshot(inputs: &[Input]) -> Result<Snapshot, (SnapshotProblem, String)> {
    use SnapshotProblem as P;
    let mut selected: Vec<&Input> = Vec::new();
    let mut exact = std::collections::BTreeSet::new();
    let mut folded = std::collections::BTreeSet::new();
    for input in inputs {
        let path = input.path.as_str();
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.contains('\0')
            || path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            return Err((P::BadPath, path.into()));
        }
        if excluded(path) {
            continue;
        }
        if path.len() > MAX_PATH_BYTES {
            return Err((P::PathTooLong, path.into()));
        }
        if !exact.insert(path) {
            return Err((P::DuplicatePath, path.into()));
        }
        if !folded.insert(path.to_lowercase()) {
            return Err((P::CaseCollision, path.into()));
        }
        selected.push(input);
    }
    if selected.is_empty() {
        return Err((P::EmptyTree, String::new()));
    }
    if selected.len() > MAX_FILES {
        return Err((P::TooManyFiles, String::new()));
    }
    selected.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut snapshot = Snapshot {
        tree_sha: String::new(),
        files: Vec::new(),
        objects: Vec::new(),
        total_bytes: 0,
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut root = Node::default();
    for input in selected {
        if input.bytes.len() > MAX_FILE_BYTES {
            return Err((P::FileTooLarge, input.path.clone()));
        }
        snapshot.total_bytes += input.bytes.len();
        if snapshot.total_bytes > MAX_TREE_BYTES {
            return Err((P::TreeTooLarge, input.path.clone()));
        }
        let segments: Vec<&str> = input.path.split('/').collect();
        let mut cursor = &mut root;
        for segment in &segments[..segments.len() - 1] {
            if cursor.files.contains_key(*segment) {
                return Err((P::FileDirCollision, input.path.clone()));
            }
            cursor = cursor.dirs.entry((*segment).into()).or_default();
        }
        let name = segments[segments.len() - 1];
        if cursor.dirs.contains_key(name) {
            return Err((P::FileDirCollision, input.path.clone()));
        }
        let sha = hash_object(Kind::Blob, &input.bytes);
        cursor.files.insert(name.into(), sha.clone());
        snapshot
            .files
            .push((input.path.clone(), sha.clone(), input.bytes.len()));
        if seen.insert(sha.clone()) {
            snapshot.objects.push(Object {
                sha,
                kind: Kind::Blob,
                body: input.bytes.clone(),
            });
        }
    }
    fn write(
        node: &Node,
        out: &mut Snapshot,
        seen: &mut std::collections::BTreeSet<String>,
    ) -> String {
        // Git order: names compared bytewise, a folder's with a trailing `/`.
        let mut entries: Vec<(Vec<u8>, String, &str, String)> = Vec::new();
        for (name, sha) in &node.files {
            entries.push((
                name.as_bytes().to_vec(),
                name.clone(),
                "100644",
                sha.clone(),
            ));
        }
        for (name, child) in &node.dirs {
            let sha = write(child, out, seen);
            let mut key = name.as_bytes().to_vec();
            key.push(b'/');
            entries.push((key, name.clone(), "40000", sha));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut body = Vec::new();
        for (_, name, mode, sha) in entries {
            body.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            body.extend(
                (0..20).map(|i| u8::from_str_radix(&sha[2 * i..2 * i + 2], 16).unwrap_or(0)),
            );
        }
        let sha = hash_object(Kind::Tree, &body);
        if seen.insert(sha.clone()) {
            out.objects.push(Object {
                sha: sha.clone(),
                kind: Kind::Tree,
                body,
            });
        }
        sha
    }
    snapshot.tree_sha = write(&root, &mut snapshot, &mut seen);
    Ok(snapshot)
}
/// The release commit: `tree`, optional parent, the handle as author and
/// committer at `time` (UTC), message `<coordinate> <version>`.
pub fn encode_commit(
    tree: &str,
    parent: Option<&str>,
    handle: &str,
    time: u64,
    message: &str,
) -> Vec<u8> {
    let person = format!("{handle} <{handle}@{HANDLE_MAIL_DOMAIN}> {time} +0000");
    let mut text = format!("tree {tree}\n");
    if let Some(parent) = parent {
        text.push_str(&format!("parent {parent}\n"));
    }
    text.push_str(&format!("author {person}\ncommitter {person}\n\n{message}"));
    text.into_bytes()
}
/// `manifest.json` as the shipped app wrote it: two-space JSON and a newline.
pub fn manifest_text(raw: &Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(raw).unwrap_or_default()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub line: u32,
    pub kind: &'static str,
    /// A `// powermove-secret-ok: <reason>` beside it.
    pub waived: Option<String>,
}
static TEXT_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\.(?:ts|js|mjs|svelte|json|md|txt|css|html|frag|vert|glsl|wgsl|yml|yaml|toml)|(?:^|/)\.env[^/]*)$").unwrap()
});
static HARD: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        ("anthropic_key", r"\bsk-ant-[A-Za-z0-9_-]{16,}\b"),
        ("stripe_key", r"\b(?:sk_live_|rk_live_)[A-Za-z0-9]{16,}\b"),
        // The shipped pattern excludes `sk-ant-`/`sk-live_` by lookahead.
        ("openai_key", r"\bsk-[A-Za-z0-9_-]{16,}\b"),
        ("aws_access_key", r"\bAKIA[0-9A-Z]{16}\b"),
        (
            "github_token",
            r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
        ),
        ("gitlab_token", r"\bglpat-[A-Za-z0-9_-]{16,}\b"),
        ("slack_token", r"\bxox[abpr]-[A-Za-z0-9-]{16,}\b"),
        ("google_api_key", r"\bAIza[0-9A-Za-z_-]{35}\b"),
        (
            "jwt",
            r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b",
        ),
        (
            "pem_private_key",
            r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----",
        ),
    ]
    .into_iter()
    .map(|(kind, pattern)| (kind, Regex::new(pattern).unwrap()))
    .collect()
});
fn hard(line: &str) -> Vec<&'static str> {
    HARD.iter()
        .filter(|(kind, pattern)| {
            pattern.find_iter(line).any(|m| {
                let text = m.as_str();
                *kind != "openai_key"
                    || !(text.starts_with("sk-ant-") || text.starts_with("sk-live_"))
            })
        })
        .map(|(kind, _)| *kind)
        .collect()
}
fn entropy(s: &str) -> f64 {
    let mut counts = BTreeMap::new();
    let n = s.chars().count() as f64;
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    counts.values().fold(0., |bits, &c| {
        let p = c as f64 / n;
        bits - p * p.log2()
    })
}
fn waiver(lines: &[&str], index: usize) -> Option<String> {
    static WAIVER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"powermove-secret-ok:\s*(.{3,200})").unwrap());
    [Some(lines[index]), index.checked_sub(1).map(|i| lines[i])]
        .into_iter()
        .flatten()
        .find_map(|line| {
            let comment = line.find("//")?;
            let reason = WAIVER.captures(&line[comment..])?.get(1)?.as_str().trim();
            (reason.chars().count() >= 3).then(|| reason.to_owned())
        })
}
/// Secrets in text files (v1 `scanText`): hard findings block; long random
/// quoted values need a reason unless a waiver comment gives one.
pub fn scan_secrets(path: &str, text: &str) -> Vec<Finding> {
    static QUOTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#""([^"'`\r\n]*?)"|'([^"'`\r\n]*?)'|`([^"'`\r\n]*?)`"#).unwrap()
    });
    if !TEXT_FILE.is_match(path)
        || text
            .get(..8192.min(text.len()))
            .is_some_and(|h| h.contains('\0'))
    {
        return Vec::new();
    }
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let number = i as u32 + 1;
        for kind in hard(line) {
            out.push(Finding {
                path: path.into(),
                line: number,
                kind,
                waived: None,
            });
        }
        for captures in QUOTED.captures_iter(line) {
            let value = (1..=3)
                .find_map(|g| captures.get(g))
                .map_or("", |m| m.as_str());
            if value.chars().count() < 32
                || value.chars().any(char::is_whitespace)
                || !value.chars().any(|c| c.is_ascii_alphabetic())
                || !value.chars().any(|c| c.is_ascii_digit())
            {
                continue;
            }
            let lower = value.to_ascii_lowercase();
            if lower.starts_with("http://")
                || lower.starts_with("https://")
                || lower.starts_with("data:image/")
                || value.starts_with("./")
                || value.starts_with("../")
                || value.contains('/')
                || value.contains('\\')
                || (value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit()))
                || value.chars().any(|c| ";:(),{}<>".contains(c))
                || !hard(value).is_empty()
                || entropy(value) <= 4.5
            {
                continue;
            }
            out.push(Finding {
                path: path.into(),
                line: number,
                kind: "high_entropy",
                waived: waiver(&lines, i),
            });
        }
    }
    out
}
/// Code that needs a permission the manifest doesn't declare (v1
/// `permissionFindings`): (path, line, permission, sentence).
pub fn permission_findings(
    files: &[(String, String)],
    manifest: &Value,
) -> Vec<(String, u32, &'static str, String)> {
    static SOURCE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\.(?:[cm]?[jt]sx?|svelte)$").unwrap());
    static NETWORK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:fetch\s*\(|new\s+WebSocket\s*\(|XMLHttpRequest\b|new\s+EventSource\s*\(|navigator\s*\.\s*sendBeacon\b)").unwrap()
    });
    static CLIPBOARD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bnavigator\s*\.\s*clipboard\b").unwrap());
    static TRUSTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\bapi\s*\.\s*(?:render|host|services|inspector|anim|model|history|edit|groups|uiState|dnd|workspace|ui\s*\.\s*(?:controls|modal|menu|drag|gesture|mount)|media\s*\.\s*(?:importFiles|assets|audio|fonts))\b").unwrap()
    });
    let api = manifest["apiVersion"].as_u64().unwrap_or(1);
    let declared: Vec<&str> = if api < 3 {
        Vec::new()
    } else {
        manifest["permissions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    };
    let hint = |permission: &str| {
        let list = format!("`permissions: [\"{permission}\"]`");
        if api < 3 {
            format!("Set `apiVersion: 3` and add {list} to manifest.json.")
        } else {
            format!("Add {list} to manifest.json.")
        }
    };
    let mut out = Vec::new();
    for (path, text) in files.iter().filter(|(p, _)| SOURCE.is_match(p)) {
        for (i, line) in text.split('\n').enumerate() {
            let number = i as u32 + 1;
            for (permission, pattern) in [("network", &*NETWORK), ("clipboard", &*CLIPBOARD)] {
                if declared.contains(&permission) {
                    continue;
                }
                if let Some(m) = pattern.find(line) {
                    let name = m.as_str().split_whitespace().collect::<Vec<_>>().join(" ");
                    let name = name.trim_end_matches('(').trim_end().to_owned()
                        + if name.ends_with('(') { "()" } else { "" };
                    out.push((path.clone(), number, permission,
                        format!("Uses {name} at {path}:{number} but doesn't declare the {permission} permission. {}", hint(permission))));
                }
            }
            if !declared.contains(&"full-access") {
                for m in TRUSTED.find_iter(line) {
                    let name: String = m.as_str().chars().filter(|c| !c.is_whitespace()).collect();
                    out.push((path.clone(), number, "full-access",
                        format!("Uses {name} (full access) at {path}:{number} but doesn't declare the full-access permission. {}", hint("full-access"))));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn objects_hash_like_git() {
        // `git hash-object` of "hello\n" and of the empty tree.
        assert_eq!(
            hash_object(Kind::Blob, b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(
            hash_object(Kind::Tree, b""),
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
        let loose = encode_loose(Kind::Blob, b"hello\n");
        let mut inflated = Vec::new();
        use std::io::Read;
        flate2::read::ZlibDecoder::new(&loose[..])
            .read_to_end(&mut inflated)
            .unwrap();
        assert_eq!(inflated, b"blob 6\0hello\n");
    }
    #[test]
    fn a_tree_is_the_registry_snapshot_and_refuses_what_it_refuses() {
        let input = |path: &str, bytes: &[u8]| Input {
            path: path.into(),
            bytes: bytes.to_vec(),
        };
        let snap = snapshot(&[
            input("manifest.json", b"{}"),
            input("src/index.ts", b"export {}"),
            input(".git/HEAD", b"x"),
            input("node_modules/a.js", b"x"),
        ])
        .unwrap();
        assert_eq!(snap.files.len(), 2);
        // Two blobs, the src tree and the root tree.
        assert_eq!(snap.objects.len(), 4);
        // `git mktree` of the same entries agrees (folder sorts as "src/").
        let src = hash_object(Kind::Tree, &{
            let mut b = b"100644 index.ts\0".to_vec();
            b.extend((0..20).map(|i| {
                u8::from_str_radix(&hash_object(Kind::Blob, b"export {}")[2 * i..2 * i + 2], 16)
                    .unwrap()
            }));
            b
        });
        assert_eq!(snap.objects[2].sha, src);
        for (paths, problem) in [
            (vec!["a/../b"], SnapshotProblem::BadPath),
            (vec!["A.ts", "a.ts"], SnapshotProblem::CaseCollision),
            (vec!["a", "a/b"], SnapshotProblem::FileDirCollision),
            (vec![".hidden"], SnapshotProblem::EmptyTree),
        ] {
            let inputs: Vec<_> = paths.iter().map(|p| input(p, b"x")).collect();
            assert_eq!(snapshot(&inputs).err().map(|e| e.0), Some(problem));
        }
        let commit = String::from_utf8(encode_commit(
            &snap.tree_sha,
            None,
            "maker",
            1,
            "maker/x 1.0.0",
        ))
        .unwrap();
        assert!(commit.starts_with(&format!(
            "tree {}\nauthor maker <maker@users.trypowermove.com> 1 +0000\n",
            snap.tree_sha
        )));
    }
    #[test]
    fn scans_find_secrets_waivers_and_undeclared_permissions() {
        let key = format!("const k = \"sk-ant-{}\";", "a".repeat(20));
        assert_eq!(scan_secrets("index.ts", &key)[0].kind, "anthropic_key");
        assert!(scan_secrets("image.png", &key).is_empty());
        let random = "const t = \"Zq8Xw2Lp9Rt4Vn7Ks1Mb6Jh3Gf5Dc0Ae2Bq9Wy\";";
        let found = scan_secrets("index.ts", random);
        assert_eq!(
            (found[0].kind, found[0].waived.is_none()),
            ("high_entropy", true)
        );
        let waived = format!("{random} // powermove-secret-ok: test fixture id");
        assert_eq!(
            scan_secrets("index.ts", &waived)[0].waived.as_deref(),
            Some("test fixture id")
        );
        let files = vec![(
            "index.ts".to_owned(),
            "await fetch(url)\nnavigator.clipboard.writeText(x)\napi.render.raster()".to_owned(),
        )];
        let manifest = serde_json::json!({"apiVersion":3,"permissions":["clipboard"]});
        let findings = permission_findings(&files, &manifest);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].2, "network");
        assert!(findings[0].3.starts_with("Uses fetch() at index.ts:1"));
        assert_eq!(findings[1].2, "full-access");
    }
}
