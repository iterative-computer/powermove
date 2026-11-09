//! Publishing an extension from this Mac to the Store (S079–S082; v1.2.0
//! main/cloud/publish.ts). The folder is read and frozen once: the snapshot
//! the confirmation names is the tree that uploads. Secrets block, waived
//! random strings carry their reasons, undeclared permissions refuse, and an
//! extension must have run cleanly here (the native stand-in for v1's sandbox
//! check). Upload is the registry's git protocol: ask which objects it lacks,
//! send them as loose objects, then put the release. Worker threads only.
use crate::cloud::Http;
use pm_services::surfaces::{
    publish_tree::{self, Input, Kind},
    publishing::{Draft, Finding, Plan, SandboxState},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// What a publish needs after `prepare`, held on the worker side.
pub struct Prepared {
    pub plan: Plan,
    handle: String,
    publisher_id: String,
    folder: PathBuf,
    /// The release the next one follows (repository, release, commit).
    existing: Option<(String, String, Option<String>)>,
}
/// The frozen release a confirmation names.
pub struct Frozen {
    pub coordinate: String,
    pub version: String,
    pub tree_sha: String,
    pub files: usize,
    pub bytes: usize,
    objects: Vec<publish_tree::Object>,
    commit_sha: String,
    body: Value,
    local_manifest: Option<(Vec<u8>, Vec<u8>)>,
}
impl Frozen {
    /// v1's confirmation: `Publish maker/x 1.2.0?`, then the snapshot.
    pub fn confirmation(&self) -> (String, String) {
        (
            format!("Publish {} {}?", self.coordinate, self.version),
            format!(
                "Snapshot {}. {} {}, {}.",
                &self.tree_sha[..12],
                self.files,
                if self.files == 1 { "file" } else { "files" },
                size_text(self.bytes)
            ),
        )
    }
}
fn size_text(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024. / 1024.)
    }
}

/// Every regular file under `folder` (links and dot folders are not followed).
fn read_folder(folder: &Path) -> Result<Vec<Input>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<Input>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                out.push(Input {
                    path: relative.to_string_lossy().replace('\\', "/"),
                    bytes: std::fs::read(&path)?,
                });
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(folder, folder, &mut out)
        .map_err(|_| "Powermove couldn’t read this extension’s folder.".to_owned())?;
    Ok(out)
}
fn provenance(profile: &Path) -> Value {
    std::fs::read(profile.join("extensions-provenance.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}
fn bump(version: &str) -> String {
    let mut parts: Vec<u64> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 {
        return "1.0.0".into();
    }
    parts[2] += 1;
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

/// The extension built and activated in this app (its last host record).
pub fn healthy(kernel: &dyn pm_compat_runner::StoreAuthority, id: &str) -> bool {
    kernel
        .state()
        .extensions
        .get(id)
        .and_then(|e| e.source.get("hostRecord"))
        .and_then(|r| r.get("health"))
        .and_then(|h| h.get("state"))
        == Some(&pm_model::Value::from("ok"))
}
/// Reads the folder, the account and the Store's page for this extension.
/// `healthy`: the extension built and activated in this app.
pub fn prepare(
    http: &mut Http,
    profile: &Path,
    id: &str,
    healthy: bool,
) -> Result<Prepared, String> {
    if !pm_compat_runner::manifest::extension_id(id) {
        return Err("That extension isn’t on this Mac.".into());
    }
    let folder = profile.join("extensions").join(id);
    if pm_compat_runner::store_host::marker(&folder).is_some() {
        return Err("Only extensions made on this Mac can be published.".into());
    }
    let me = http
        .call("GET", "/v1/me", None, true)
        .map_err(|_| "Sign in to publish.".to_owned())?;
    let (Some(handle), Some(publisher_id)) = (
        me["publisher"]["handle"].as_str(),
        me["publisher"]["id"].as_str(),
    ) else {
        return Err("Choose a handle in Settings › Accounts before you publish.".into());
    };
    let inputs = read_folder(&folder)?;
    let manifest: Value = inputs
        .iter()
        .find(|i| i.path == "manifest.json")
        .and_then(|i| serde_json::from_slice(&i.bytes).ok())
        .ok_or("This extension’s manifest.json isn’t valid JSON. Fix it and try again.")?;
    if manifest["id"] != id {
        return Err(format!(
            "manifest.json says its id is “{}”, but its folder is “{id}”. Make them match.",
            manifest["id"].as_str().unwrap_or("")
        ));
    }
    let record = &provenance(profile)[id]["published"];
    let detail = match http.call("GET", &format!("/v1/store/x/{handle}/{id}"), None, false) {
        Ok(detail) => Some(detail),
        Err(e) if e.code == pm_services::ServiceErrorCode::NotFound => None,
        Err(_) => return Err("Can’t reach the Store. Check your connection and try again.".into()),
    };
    let existing = detail.as_ref().map(|d| {
        let latest = d["latest"]["id"].as_str().unwrap_or_default().to_owned();
        let commit = d["releases"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["id"] == latest.as_str())
            .and_then(|r| r["commitSha"].as_str())
            .or(record["commitSha"].as_str())
            .map(str::to_owned);
        (
            d["repoId"].as_str().unwrap_or_default().to_owned(),
            latest,
            commit,
        )
    });
    let last_version = detail
        .as_ref()
        .and_then(|d| d["latest"]["version"].as_str())
        .map(str::to_owned);
    let snap = publish_tree::snapshot(&inputs).map_err(|(p, path)| p.text(&path))?;
    let texts: Vec<(String, String)> = inputs
        .iter()
        .filter_map(|i| Some((i.path.clone(), String::from_utf8(i.bytes.clone()).ok()?)))
        .collect();
    let secrets: Vec<_> = texts
        .iter()
        .flat_map(|(path, text)| publish_tree::scan_secrets(path, text))
        .collect();
    let plan = Plan {
        local_id: id.into(),
        coordinate: format!("{handle}/{id}"),
        suggested_version: match &last_version {
            Some(last) => bump(last),
            None => manifest["version"].as_str().unwrap_or("1.0.0").to_owned(),
        },
        last_version,
        first_publish: detail.is_none(),
        listing: json!({"name":manifest["name"],"tagline":manifest["description"].as_str().unwrap_or(""),
            "category":category(&manifest)}),
        blocked_findings: secrets
            .iter()
            .filter(|f| f.waived.is_none() && f.kind != "high_entropy")
            .map(|f| json!({"path":f.path,"line":f.line,"kind":f.kind}))
            .collect(),
        permission_findings: publish_tree::permission_findings(&texts, &manifest)
            .into_iter()
            .map(|(path, line, needs, text)| json!({"path":path,"line":line,"needs":needs,"text":text}))
            .collect(),
        waivable_findings: secrets
            .iter()
            .filter(|f| f.kind == "high_entropy")
            .map(|f| Finding { path: f.path.clone(), line: f.line })
            .collect(),
        sandbox: if manifest["permissions"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p == "full-access"))
        {
            SandboxState::FullAccess
        } else if healthy {
            SandboxState::Passed
        } else {
            SandboxState::CouldNotRun
        },
    };
    let _ = snap;
    Ok(Prepared {
        plan,
        handle: handle.into(),
        publisher_id: publisher_id.into(),
        folder,
        existing,
    })
}
/// A first listing's category, from what the manifest contributes.
fn category(manifest: &Value) -> &'static str {
    let contributes = &manifest["contributes"];
    for (key, category) in [
        ("effects", "effects"),
        ("transitions", "transitions"),
        ("layers", "layers"),
        ("themes", "themes"),
        ("panels", "panels"),
        ("tools", "tools"),
    ] {
        if contributes.get(key).is_some() {
            return category;
        }
    }
    "commands"
}
/// Waivers carry the reason the code gives (`// powermove-secret-ok: …`).
pub fn draft_reasons(prepared: &Prepared, draft: &mut Draft) {
    let Ok(inputs) = read_folder(&prepared.folder) else {
        return;
    };
    for input in inputs {
        let Ok(text) = String::from_utf8(input.bytes) else {
            continue;
        };
        for finding in publish_tree::scan_secrets(&input.path, &text) {
            if let Some(reason) = finding.waived {
                draft
                    .reasons
                    .insert(format!("{}:{}", finding.path, finding.line), reason);
            }
        }
    }
}
/// The release to upload for a validated form, frozen now.
pub fn freeze(prepared: &Prepared, form: &Value, now: u64) -> Result<Frozen, String> {
    let version = form["version"].as_str().unwrap_or_default().to_owned();
    let mut inputs = read_folder(&prepared.folder)?;
    let mut local_manifest = None;
    if let Some(input) = inputs.iter_mut().find(|i| i.path == "manifest.json") {
        let mut raw: Value = serde_json::from_slice(&input.bytes)
            .map_err(|_| "manifest.json isn’t valid JSON. Fix it and try again.")?;
        if raw["version"] != version.as_str() {
            raw["version"] = json!(version);
            let text = publish_tree::manifest_text(&raw).into_bytes();
            local_manifest = Some((input.bytes.clone(), text.clone()));
            input.bytes = text;
        }
    }
    let snap = publish_tree::snapshot(&inputs).map_err(|(p, path)| p.text(&path))?;
    let coordinate = prepared.plan.coordinate.clone();
    let parent = prepared.existing.as_ref().and_then(|e| e.2.as_deref());
    let commit = publish_tree::encode_commit(
        &snap.tree_sha,
        parent,
        &prepared.handle,
        now,
        &format!("{coordinate} {version}"),
    );
    let commit_sha = publish_tree::hash_object(Kind::Commit, &commit);
    let mut objects = snap.objects;
    objects.push(publish_tree::Object {
        sha: commit_sha.clone(),
        kind: Kind::Commit,
        body: commit,
    });
    let mut body = form.clone();
    body["commitSha"] = json!(commit_sha);
    if let Some((_, release, _)) = &prepared.existing {
        body["basedOnReleaseId"] = json!(release);
    }
    Ok(Frozen {
        coordinate,
        version,
        tree_sha: snap.tree_sha,
        files: snap.files.len(),
        bytes: snap.total_bytes,
        objects,
        commit_sha,
        body,
        local_manifest,
    })
}
/// Uploads what the registry lacks, puts the release (again once if its
/// objects were collected meanwhile), and records it on this Mac.
pub fn upload(
    http: &mut Http,
    profile: &Path,
    prepared: &Prepared,
    frozen: Frozen,
) -> Result<String, String> {
    let offline = |_| "Can’t reach the Store. Check your connection and try again.".to_owned();
    let mut release = None;
    for attempt in 0..2 {
        let shas: Vec<&str> = frozen.objects.iter().map(|o| o.sha.as_str()).collect();
        let mut missing = std::collections::BTreeSet::new();
        for chunk in shas.chunks(1000) {
            let mut body = json!({ "shas": chunk });
            if let Some((repo, ..)) = &prepared.existing {
                body["repoId"] = json!(repo);
            }
            let answer = http
                .call("POST", "/v1/objects/missing", Some(body), true)
                .map_err(offline)?;
            for sha in answer["missing"].as_array().into_iter().flatten() {
                missing.extend(sha.as_str().map(str::to_owned));
            }
        }
        let parts: Vec<(String, Vec<u8>)> = frozen
            .objects
            .iter()
            .filter(|o| missing.contains(&o.sha))
            .map(|o| (o.sha.clone(), publish_tree::encode_loose(o.kind, &o.body)))
            .collect();
        for batch in batches(parts) {
            let answer = http.multipart("/v1/objects", &batch).map_err(offline)?;
            if let Some(code) = answer["rejected"][0]["code"].as_str() {
                return Err(format!(
                    "The Store refused one of the files ({code}). Try again."
                ));
            }
        }
        let path = format!(
            "/v1/repos/{}/{}/releases",
            prepared.handle, prepared.plan.local_id
        );
        match http.call("PUT", &path, Some(frozen.body.clone()), true) {
            Ok(answer) => {
                release = Some(answer);
                break;
            }
            Err(e)
                if attempt == 0
                    && matches!(
                        e.to_string().as_str(),
                        "api:object_missing" | "api:commit_missing"
                    ) => {}
            Err(e) => return Err(explain(&e.to_string())),
        }
    }
    let release = release.ok_or("The Store didn’t take the release. Try again.")?;
    let release_id = release["release"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut record = provenance(profile);
    record[&prepared.plan.local_id]["published"] = json!({
        "repoId": release["repo"]["repoId"], "releaseId": release_id,
        "version": frozen.version, "commitSha": frozen.commit_sha,
        "ownerPublisherId": prepared.publisher_id, "coordinate": frozen.coordinate,
        "publishedTreeSha": frozen.tree_sha});
    let _ = std::fs::write(
        profile.join("extensions-provenance.json"),
        serde_json::to_vec_pretty(&record).unwrap_or_default(),
    );
    // The published version goes back into manifest.json, unless it was
    // edited meanwhile (an edit wins).
    if let Some((before, after)) = frozen.local_manifest {
        let target = prepared.folder.join("manifest.json");
        if std::fs::read(&target).ok().as_deref() == Some(&before[..]) {
            let temporary = prepared.folder.join(".manifest.json.publish");
            if std::fs::write(&temporary, &after).is_ok() {
                let _ = std::fs::rename(&temporary, &target);
            }
        }
    }
    Ok(format!(
        "Published {} {}.",
        frozen.coordinate, frozen.version
    ))
}
/// At most 64 parts and 24 MB per request (the registry's upload envelope).
fn batches(parts: Vec<(String, Vec<u8>)>) -> Vec<Vec<(String, Vec<u8>)>> {
    let mut out: Vec<Vec<(String, Vec<u8>)>> = Vec::new();
    let mut size = 0;
    for part in parts {
        if out
            .last()
            .is_none_or(|b| b.len() >= 64 || size + part.1.len() > 24 * 1024 * 1024)
        {
            out.push(Vec::new());
            size = 0;
        }
        size += part.1.len();
        out.last_mut().unwrap().push(part);
    }
    out
}
/// A refusal from the registry, as one sentence.
pub fn explain(code: &str) -> String {
    match code {
        "api:version_exists" => "That version is already on the Store. Choose a higher one.".into(),
        "api:scan_blocked" => {
            "The Store found what looks like a secret in the files. Remove it and try again.".into()
        }
        "api:permission_undeclared" => {
            "The code uses a permission manifest.json doesn’t declare.".into()
        }
        "api:not_owner" | "api:handle_taken" => {
            "Someone else owns this extension on the Store.".into()
        }
        "api:rate_limited" => "Too many requests. Wait a moment and try again.".into(),
        "api:client_too_old" => {
            "This version of Powermove is too old for the Store. Update Powermove and try again."
                .into()
        }
        "network" => "Can’t reach the Store. Check your connection and try again.".into(),
        _ => "The Store didn’t take the release. Try again.".into(),
    }
}
/// The first sentence a plan's problems need the person to fix.
pub fn plan_problem(plan: &Plan) -> Option<String> {
    if let Some(finding) = plan.blocked_findings.first() {
        return Some(format!(
            "{}:{} looks like a secret ({}). Remove it before publishing.",
            finding["path"].as_str().unwrap_or(""),
            finding["line"],
            finding["kind"].as_str().unwrap_or("")
        ));
    }
    if let Some(finding) = plan.permission_findings.first() {
        return finding["text"].as_str().map(str::to_owned);
    }
    if plan.sandbox == SandboxState::CouldNotRun {
        return Some("Turn this extension on and make sure it runs before you publish it.".into());
    }
    None
}
