//! Per-project agent workspaces (port of `codex/workspace.ts`,
//! `codex/artifacts.ts` and the stage half of `codex/change-history.ts`;
//! publishing the stage is `changes`).
//!
//! `<userData>/Agent Workspaces/<project>` is the agent's working directory:
//! `inputs/` (project snapshot, attachments, reference images),
//! `powermove-api/` (the API pack), `artifacts/<run>/` (deliverables) and
//! `.powermove/extension-runs/<run>/` (a private copy of the user's
//! extensions — the agent never receives the live folder). Sessions and
//! result files are the host's own state under `Agent State/`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ids::is_extension_id;

pub const CODEX_IMAGES: usize = 12;
pub const CODEX_ATTACHMENTS: usize = 6;
const SESSION_CONTRACT_VERSION: u32 = 2;
const MAX_STAGE_FILES: usize = 4_000;

/// A failure the agent can repair by returning a corrected result.
#[derive(Debug)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

pub fn safe_component(value: &str, fallback: &str) -> String {
    let mut cleaned = String::new();
    for character in value.chars() {
        let keep = character.is_ascii_alphanumeric() || character == '_' || character == '-';
        if keep {
            cleaned.push(character);
        } else if !cleaned.ends_with('-') {
            cleaned.push('-');
        }
    }
    let cleaned = cleaned.trim_matches('-');
    let cleaned = if cleaned.is_empty() { fallback } else { cleaned };
    cleaned.chars().take(120).collect()
}

pub fn workspace_root(user_data: &Path, id: &str) -> PathBuf {
    user_data.join("Agent Workspaces").join(safe_component(id, "project"))
}

/// The host's run files live beside the workspace, never in it.
pub fn state_root(user_data: &Path, root: &Path) -> PathBuf {
    user_data.join("Agent State").join(root.file_name().unwrap_or_default())
}

pub fn session_path(user_data: &Path, root: &Path, authority: &str, thread_id: Option<&str>, provider: &str) -> PathBuf {
    let provider_part = if provider == "chatgpt" { String::new() } else { format!("-{provider}") };
    let name = format!("session-v{SESSION_CONTRACT_VERSION}{provider_part}-{authority}.txt");
    let state = state_root(user_data, root);
    match thread_id {
        Some(thread) => state.join("threads").join(thread).join(name),
        None => state.join(name),
    }
}

/// A native session id becomes a CLI argument; anything else is dropped.
pub fn valid_session_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=200).contains(&bytes.len()) && bytes[0].is_ascii_alphanumeric() && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

pub async fn read_session(path: &Path) -> Option<String> {
    let value = tokio::fs::read_to_string(path).await.ok()?;
    let value = value.trim();
    valid_session_id(value).then(|| value.to_owned())
}

pub async fn write_session(path: &Path, id: &str) {
    if !valid_session_id(id.trim()) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(path, id.trim()).await;
}

pub async fn clear_session(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

/// File extension for reference image bytes.
pub fn image_extension(data: &[u8]) -> &'static str {
    if data.starts_with(b"\xFF\xD8\xFF") {
        "jpg"
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if data.starts_with(b"GIF8") {
        "gif"
    } else {
        "png"
    }
}

pub struct Inputs<'a> {
    pub project_json: &'a str,
    pub images: &'a [Bytes],
    pub attachments: &'a [(String, Bytes)],
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub run_id: String,
    pub run_directory: PathBuf,
    /// The run-private extension stage the agent may edit.
    pub extensions_dir: PathBuf,
    pub baseline: BTreeMap<String, String>,
    pub schema_path: PathBuf,
    pub output_path: PathBuf,
    pub session_path: PathBuf,
    pub image_paths: Vec<PathBuf>,
}

pub struct Prepare<'a> {
    pub user_data: &'a Path,
    pub workspace_id: &'a str,
    pub authority: &'a str,
    pub thread_id: Option<&'a str>,
    pub provider: &'a str,
    pub schema: &'a Value,
    pub live_extensions: &'a Path,
    pub api_pack: &'a [(String, String)],
}

fn safe_pack_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\\') && !name.contains('\0') && !Path::new(name).is_absolute() && name.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// `prepareAgentWorkspace`.
pub async fn prepare(options: &Prepare<'_>, inputs: &Inputs<'_>) -> Result<Workspace> {
    let root = workspace_root(options.user_data, options.workspace_id);
    let state = state_root(options.user_data, &root);
    let run_id = safe_component(&format!("{}-{}", chrono_seconds(), uuid::Uuid::new_v4()), "run");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(&state).await?;
    let schema_path = state.join("result-schema.json");
    tokio::fs::write(&schema_path, format!("{}\n", serde_json::to_string_pretty(options.schema)?)).await?;

    // Inputs: this run's snapshot, attachments and references replace any earlier ones.
    let inputs_dir = root.join("inputs");
    for folder in ["attachments", "references"] {
        let directory = inputs_dir.join(folder);
        let _ = tokio::fs::remove_dir_all(&directory).await;
        tokio::fs::create_dir_all(&directory).await?;
    }
    tokio::fs::write(inputs_dir.join("powermove-project.json"), inputs.project_json).await?;
    for (index, (name, data)) in inputs.attachments.iter().take(CODEX_ATTACHMENTS).enumerate() {
        tokio::fs::write(inputs_dir.join("attachments").join(safe_component(name, &format!("attachment-{index}"))), data).await?;
    }
    let mut image_paths = Vec::new();
    for (index, image) in inputs.images.iter().take(CODEX_IMAGES).enumerate() {
        let path = inputs_dir.join("references").join(format!("reference-{index}.{}", image_extension(image)));
        tokio::fs::write(&path, image).await?;
        image_paths.push(path);
    }

    // The API pack is rebuilt whole so stale files never linger.
    let pack = root.join("powermove-api");
    let _ = tokio::fs::remove_dir_all(&pack).await;
    for (name, text) in options.api_pack.iter().filter(|(name, _)| safe_pack_name(name)) {
        let target = pack.join(name);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(target, text).await?;
    }

    let run_directory = root.join("artifacts").join(&run_id);
    tokio::fs::create_dir_all(&run_directory).await?;
    let extensions_dir = root.join(".powermove").join("extension-runs").join(&run_id);
    tokio::fs::create_dir_all(options.live_extensions).await?;
    let live = options.live_extensions.to_path_buf();
    let stage = extensions_dir.clone();
    let baseline = tokio::task::spawn_blocking(move || -> Result<BTreeMap<String, String>> {
        copy_tree(&live, &stage, &mut 0)?;
        extension_hashes(&stage)
    })
    .await??;

    Ok(Workspace {
        session_path: session_path(options.user_data, &root, options.authority, options.thread_id, options.provider),
        output_path: state.join(format!("result-{run_id}.json")),
        root,
        run_id,
        run_directory,
        extensions_dir,
        baseline,
        schema_path,
        image_paths,
    })
}

fn chrono_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0)
}

/// Copy folders and regular files only (`copyRegularTree`). A link or
/// special file aborts the copy instead of being followed or skipped, so
/// what is copied always hashes the same as its source. `count` caps entries.
pub fn copy_tree(from: &Path, to: &Path, count: &mut usize) -> Result<()> {
    std::fs::create_dir_all(to)?;
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        *count += 1;
        if *count > MAX_STAGE_FILES {
            bail!("The extensions folder has more than {MAX_STAGE_FILES} entries.");
        }
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            bail!("Extension staging does not allow symbolic links: {}", entry.file_name().to_string_lossy());
        } else if kind.is_dir() {
            copy_tree(&entry.path(), &target, count)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        } else {
            bail!("Extension staging does not allow special files: {}", entry.file_name().to_string_lossy());
        }
    }
    Ok(())
}

fn hash_directory(directory: &Path, prefix: &str, hasher: &mut Sha256) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!("Extension contains an unsupported symbolic link: {relative}");
        } else if kind.is_dir() {
            hasher.update(format!("d:{relative}\0"));
            hash_directory(&entry.path(), &relative, hasher)?;
        } else if kind.is_file() {
            hasher.update(format!("f:{relative}\0"));
            hasher.update(Sha256::digest(std::fs::read(entry.path())?));
        } else {
            bail!("Extension contains an unsupported file: {relative}");
        }
    }
    Ok(())
}

/// Content hash of one extension folder.
pub fn folder_hash(directory: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_directory(directory, "", &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// Content hash of every extension folder under `root`.
pub fn extension_hashes(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(root) else { return Ok(hashes) };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type()?.is_dir() || !is_extension_id(&name) {
            continue;
        }
        hashes.insert(name, folder_hash(&entry.path())?);
    }
    Ok(hashes)
}

/// Drop the run's extension stage once its result is final.
pub async fn discard_stage(workspace: &Workspace) {
    let _ = tokio::fs::remove_dir_all(&workspace.extensions_dir).await;
}

/// Drop a run's artifact folder when the agent left nothing in it.
pub async fn remove_empty_run_directory(workspace: &Workspace) {
    let _ = tokio::fs::remove_dir(&workspace.run_directory).await;
}

/* ── artifacts ───────────────────────────────────────────────── */

pub fn mime_type(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "aac" => "audio/aac",
        "aif" | "aiff" => "audio/aiff",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "css" => "text/css",
        "csv" => "text/csv",
        "flac" => "audio/flac",
        "gif" => "image/gif",
        "htm" | "html" => "text/html",
        "jpeg" | "jpg" => "image/jpeg",
        "js" => "text/javascript",
        "json" | "map" => "application/json",
        "m4a" => "audio/mp4",
        "m4v" | "mp4" => "video/mp4",
        "md" => "text/markdown",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "oga" | "ogg" => "audio/ogg",
        "ogv" => "video/ogg",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "ttf" => "font/ttf",
        "ts" => "text/typescript",
        "txt" => "text/plain",
        "wasm" => "application/wasm",
        "wav" => "audio/wav",
        "webm" => "video/webm",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "xml" => "application/xml",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn reject_unsafe_relative(relative: &str) -> Result<()> {
    let lower = relative.to_ascii_lowercase();
    if relative.is_empty() || relative.contains('\0') || relative.contains('\\') || lower.contains("%2f") || lower.contains("%5c") || relative.contains(':') {
        bail!("Invalid artifact path.");
    }
    if Path::new(relative).is_absolute() || relative.starts_with('/') || relative.split('/').any(|segment| segment == "..") {
        bail!("Artifact path escapes its workspace.");
    }
    Ok(())
}

/// Resolve an artifact path and prove its target is a regular file under the real root.
pub fn validated_artifact_path(root: &Path, relative: &str) -> Result<PathBuf> {
    reject_unsafe_relative(relative)?;
    let real_root = std::fs::canonicalize(root).context("Artifact not found.")?;
    let candidate = std::fs::canonicalize(real_root.join(relative)).context("Artifact not found.")?;
    match candidate.strip_prefix(&real_root) {
        Ok(rest) if !rest.as_os_str().is_empty() && rest.components().all(|part| matches!(part, Component::Normal(_))) => {}
        _ => bail!("Artifact path escapes its workspace."),
    }
    if !candidate.is_file() {
        bail!("Artifact is not a regular file.");
    }
    Ok(candidate)
}

/// `collectArtifacts`: every file the run left, flagged for timeline import as requested.
pub fn collect_artifacts(run_directory: &Path, run_id: &str, requested: &[Value]) -> Result<Vec<Value>, ValidationError> {
    let mut imports = Vec::new();
    for entry in requested.iter().filter(|entry| entry["importToTimeline"] == true) {
        let Some(path) = entry["path"].as_str() else { return Err(ValidationError("An import requires an artifact path.".into())) };
        reject_unsafe_relative(path).map_err(|error| ValidationError(error.to_string()))?;
        imports.push(path.trim_start_matches("./").to_owned());
    }
    let mut discovered = Vec::new();
    fn walk(directory: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(directory) else { return };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let local = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&entry.path(), &local, out),
                Ok(kind) if kind.is_file() => out.push((local, entry.path())),
                _ => {}
            }
        }
    }
    walk(run_directory, "", &mut discovered);
    let mut import_files = Vec::new();
    for requested in &imports {
        let exact: Vec<&String> = discovered.iter().map(|(local, _)| local).filter(|local| requested == *local || *requested == format!("{run_id}/{local}") || *requested == format!("artifacts/{run_id}/{local}")).collect();
        let matches = if exact.is_empty() { discovered.iter().map(|(local, _)| local).filter(|local| local.rsplit('/').next() == Some(requested.as_str())).collect() } else { exact };
        if matches.len() != 1 {
            return Err(ValidationError(if matches.is_empty() {
                format!("Requested import was not found: {requested}. Put the file under artifacts/{run_id}/ and correct its path in the completion report.")
            } else {
                format!("Import path is ambiguous: {requested}. Use the full path under artifacts/{run_id}/.")
            }));
        }
        import_files.push(matches[0].clone());
    }
    discovered.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(discovered
        .into_iter()
        .filter_map(|(local, full)| {
            let size = std::fs::metadata(&full).ok().filter(|metadata| metadata.is_file())?.len();
            Some(json!({
                "path": format!("{run_id}/{local}"),
                "name": local.rsplit('/').next().unwrap_or(&local),
                "size": size,
                "mime": mime_type(&full),
                "importToTimeline": import_files.contains(&local),
            }))
        })
        .collect())
}

/// Remove workspaces of projects that no longer exist (`sweepOrphanWorkspaces`).
pub async fn sweep(user_data: &Path, live: &[String], busy: &[String]) -> Vec<String> {
    let base = user_data.join("Agent Workspaces");
    let keep: Vec<String> = live.iter().chain(busy).map(|id| safe_component(id, "project")).collect();
    let mut removed = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(&base).await else { return removed };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Thread and subagent workspaces belong to their project; keep them.
        if keep.contains(&name) || name.starts_with("thread-") || name.starts_with("subagent-") || name.starts_with("mcp-") || name == "powermove-global" {
            continue;
        }
        if entry.file_type().await.is_ok_and(|kind| kind.is_dir()) && tokio::fs::remove_dir_all(entry.path()).await.is_ok() {
            let _ = tokio::fs::remove_dir_all(user_data.join("Agent State").join(&name)).await;
            removed.push(name);
        }
    }
    removed
}

pub async fn forget(user_data: &Path, project_id: &str) {
    let root = workspace_root(user_data, project_id);
    let _ = tokio::fs::remove_dir_all(state_root(user_data, &root)).await;
    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_and_sessions_are_safe() {
        assert_eq!(safe_component("../a b//c", "x"), "a-b-c");
        assert_eq!(safe_component("///", "x"), "x");
        assert!(valid_session_id("019a-thread_1"));
        assert!(!valid_session_id("--resume"));
        let root = Path::new("data").join("Agent Workspaces").join("p1");
        let session = session_path(Path::new("data"), &root, "project", Some("t1"), "claude");
        assert!(session.ends_with(Path::new("Agent State").join("p1").join("threads").join("t1").join("session-v2-claude-project.txt")));
    }

    #[tokio::test]
    async fn prepares_inputs_stage_and_reports_changes() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("extensions");
        std::fs::create_dir_all(live.join("hello-world")).unwrap();
        std::fs::write(live.join("hello-world").join("manifest.json"), "{}").unwrap();
        let pack = vec![("api.ts".to_owned(), "export {}".to_owned()), ("../escape.ts".to_owned(), "no".to_owned())];
        let workspace = prepare(
            &Prepare { user_data: directory.path(), workspace_id: "project-1", authority: "project", thread_id: None, provider: "chatgpt", schema: &json!({ "type": "object" }), live_extensions: &live, api_pack: &pack },
            &Inputs { project_json: "{\"revision\":1}", images: &[Bytes::from_static(b"\xFF\xD8\xFFx")], attachments: &[("notes.txt".into(), Bytes::from_static(b"hi"))] },
        )
        .await
        .unwrap();
        assert!(workspace.root.join("inputs").join("powermove-project.json").is_file());
        assert!(workspace.root.join("inputs").join("attachments").join("notes-txt").is_file());
        assert!(workspace.image_paths[0].ends_with("reference-0.jpg"));
        assert!(workspace.root.join("powermove-api").join("api.ts").is_file());
        assert!(!directory.path().join("Agent Workspaces").join("escape.ts").exists());
        assert!(workspace.extensions_dir.join("hello-world").join("manifest.json").is_file());

        assert_eq!(workspace.baseline.keys().collect::<Vec<_>>(), ["hello-world"]);
        std::fs::write(workspace.extensions_dir.join("hello-world").join("index.ts"), "x").unwrap();
        std::fs::create_dir_all(workspace.extensions_dir.join("new-one")).unwrap();
        let after = extension_hashes(&workspace.extensions_dir).unwrap();
        assert_ne!(after["hello-world"], workspace.baseline["hello-world"]);
        assert!(after.contains_key("new-one"));
    }

    #[test]
    fn collects_artifacts_and_validates_imports() {
        let directory = tempfile::tempdir().unwrap();
        let run = directory.path().join("run-1");
        std::fs::create_dir_all(run.join("clips")).unwrap();
        std::fs::write(run.join("clips").join("a.mp4"), b"x").unwrap();
        std::fs::write(run.join(".hidden"), b"x").unwrap();
        let artifacts = collect_artifacts(&run, "run-1", &[json!({ "path": "artifacts/run-1/clips/a.mp4", "importToTimeline": true })]).unwrap();
        assert_eq!(artifacts, vec![json!({ "path": "run-1/clips/a.mp4", "name": "a.mp4", "size": 1, "mime": "video/mp4", "importToTimeline": true })]);
        assert!(collect_artifacts(&run, "run-1", &[json!({ "path": "missing.mp4", "importToTimeline": true })]).is_err());
        assert!(collect_artifacts(&run, "run-1", &[json!({ "path": "../x", "importToTimeline": true })]).is_err());
        assert!(validated_artifact_path(directory.path(), "run-1/clips/a.mp4").is_ok());
        assert!(validated_artifact_path(directory.path(), "run-1/../run-1/clips/a.mp4").is_err());
        assert!(validated_artifact_path(directory.path(), "run-1/clips").is_err());
    }
}
