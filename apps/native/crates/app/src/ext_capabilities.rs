//! Layer 6 native capabilities for v4 extensions: filesystem (`fs`), persistent
//! per-extension key-value storage (`storage`), native HTTP (`net.fetch`),
//! clipboard (`clipboard`), and macOS shell/process execution (`shell`).
//!
//! All blocking IO runs on a worker thread, never on the GPUI main thread.
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Map, Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, UNIX_EPOCH},
};

const MAX_READ_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXEC_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_EXEC_TIMEOUT_MS: u64 = 30_000;
const MAX_EXEC_TIMEOUT_MS: u64 = 300_000;

/// Expands leading `~` or `~/` to the user's home directory.
pub fn expand_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty".into());
    }
    if trimmed == "~" {
        return home_dir();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    Ok(PathBuf::from(trimmed))
}

pub fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "HOME directory is not set".into())
}

fn modified_ms(meta: &fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

fn stat_json(path: &Path) -> Result<Value, String> {
    let link_meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let is_symlink = link_meta.file_type().is_symlink();
    let meta = fs::metadata(path).unwrap_or(link_meta);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    Ok(json!({
        "name": name,
        "path": path.to_string_lossy(),
        "isDir": meta.is_dir(),
        "isFile": meta.is_file(),
        "isSymlink": is_symlink,
        "isHidden": name.starts_with('.'),
        "size": meta.len(),
        "modifiedMs": modified_ms(&meta),
        "extension": extension,
    }))
}

fn storage_file(profile: &Path, extension_id: &str) -> Result<PathBuf, String> {
    let clean: String = extension_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if clean.is_empty() || clean == "." || clean == ".." {
        return Err("Invalid extension storage id".into());
    }
    Ok(profile.join("extension-storage").join(format!("{clean}.json")))
}

fn load_storage(path: &Path) -> Map<String, Value> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn save_storage(path: &Path, map: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&Value::Object(map.clone())).map_err(|e| e.to_string())?;
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Dispatches any worker-safe Layer 6 extension capability (`ext.fs.*`,
/// `ext.storage.*`, `ext.net.*`, `ext.shell.exec`).
pub fn run_worker_op(
    profile: &Path,
    panel: &str,
    operation: &str,
    input: &Value,
) -> Result<Value, String> {
    match operation {
        // ---- Filesystem ----
        "ext.fs.homeDir" => Ok(json!(home_dir()?.to_string_lossy())),
        "ext.fs.listDir" => {
            let raw = input["path"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("~");
            let dir = expand_path(raw)?;
            let include_hidden = input["includeHidden"].as_bool().unwrap_or(true);
            let entries = fs::read_dir(&dir)
                .map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
            let mut items: Vec<Value> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_hidden = name.starts_with('.');
                if !include_hidden && is_hidden {
                    continue;
                }
                let path = entry.path();
                if let Ok(item) = stat_json(&path) {
                    items.push(item);
                }
            }
            // Directories first, then case-insensitive alphabetical.
            items.sort_by(|a, b| {
                let ad = a["isDir"].as_bool().unwrap_or(false);
                let bd = b["isDir"].as_bool().unwrap_or(false);
                bd.cmp(&ad).then_with(|| {
                    a["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
                })
            });
            let parent = dir.parent().map(|p| p.to_string_lossy().into_owned());
            Ok(json!({
                "path": dir.to_string_lossy(),
                "parent": parent,
                "entries": items,
            }))
        }
        "ext.fs.stat" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            if !path.exists() && fs::symlink_metadata(&path).is_err() {
                return Ok(Value::Null);
            }
            stat_json(&path)
        }
        "ext.fs.exists" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            Ok(json!(path.exists() || fs::symlink_metadata(&path).is_ok()))
        }
        "ext.fs.readText" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            let limit = input["maxBytes"]
                .as_u64()
                .map(|n| (n as usize).min(MAX_READ_BYTES))
                .unwrap_or(MAX_READ_BYTES);
            let mut file = fs::File::open(&path)
                .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
            let mut buf = Vec::new();
            std::io::Read::by_ref(&mut file)
                .take((limit + 1) as u64)
                .read_to_end(&mut buf)
                .map_err(|e| e.to_string())?;
            if buf.len() > limit {
                return Err(format!("File exceeds limit of {limit} bytes"));
            }
            let text = String::from_utf8_lossy(&buf).into_owned();
            Ok(json!(text))
        }
        "ext.fs.readBytes" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            let limit = input["maxBytes"]
                .as_u64()
                .map(|n| (n as usize).min(MAX_READ_BYTES))
                .unwrap_or(MAX_READ_BYTES);
            let mut file = fs::File::open(&path)
                .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
            let mut buf = Vec::new();
            std::io::Read::by_ref(&mut file)
                .take((limit + 1) as u64)
                .read_to_end(&mut buf)
                .map_err(|e| e.to_string())?;
            if buf.len() > limit {
                return Err(format!("File exceeds limit of {limit} bytes"));
            }
            Ok(json!(BASE64.encode(&buf)))
        }
        "ext.fs.writeText" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            let content = input["content"].as_str().unwrap_or("");
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "ext.fs.writeBytes" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            let encoded = input["base64"].as_str().unwrap_or("");
            let bytes = BASE64.decode(encoded).map_err(|e| e.to_string())?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "ext.fs.mkdir" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "ext.fs.rename" => {
            let from = expand_path(input["from"].as_str().unwrap_or(""))?;
            let to = expand_path(input["to"].as_str().unwrap_or(""))?;
            fs::rename(&from, &to).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "ext.fs.trash" => {
            let path = expand_path(input["path"].as_str().unwrap_or(""))?;
            trash_item(&path)?;
            Ok(Value::Null)
        }

        // ---- Persistent Per-Extension KV Storage ----
        "ext.storage.get" => {
            let key = input["key"].as_str().ok_or("storage.get needs a key")?;
            let file = storage_file(profile, panel)?;
            let map = load_storage(&file);
            Ok(map.get(key).cloned().unwrap_or(Value::Null))
        }
        "ext.storage.set" => {
            let key = input["key"].as_str().ok_or("storage.set needs a key")?;
            let file = storage_file(profile, panel)?;
            let mut map = load_storage(&file);
            if input["value"].is_null() {
                map.remove(key);
            } else {
                map.insert(key.to_owned(), input["value"].clone());
            }
            save_storage(&file, &map)?;
            Ok(Value::Null)
        }
        "ext.storage.delete" => {
            let key = input["key"].as_str().ok_or("storage.delete needs a key")?;
            let file = storage_file(profile, panel)?;
            let mut map = load_storage(&file);
            map.remove(key);
            save_storage(&file, &map)?;
            Ok(Value::Null)
        }
        "ext.storage.keys" => {
            let file = storage_file(profile, panel)?;
            let map = load_storage(&file);
            let keys: Vec<Value> = map.keys().cloned().map(Value::String).collect();
            Ok(Value::Array(keys))
        }
        "ext.storage.getAll" => {
            let file = storage_file(profile, panel)?;
            Ok(Value::Object(load_storage(&file)))
        }
        "ext.storage.clear" => {
            let file = storage_file(profile, panel)?;
            if file.exists() {
                let _ = fs::remove_file(&file);
            }
            Ok(Value::Null)
        }

        // ---- Native HTTP Fetch (via macOS /usr/bin/curl) ----
        "ext.net.fetch" => run_fetch(input),

        // ---- Native Process Exec ----
        "ext.shell.exec" => run_exec(input),

        _ => Err(format!("Unknown extension worker op: {operation}")),
    }
}

fn trash_item(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let fm = NSFileManager::defaultManager();
        fm.trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|e| format!("Could not move to Trash: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())
        } else {
            fs::remove_file(path).map_err(|e| e.to_string())
        }
    }
}

fn run_fetch(input: &Value) -> Result<Value, String> {
    let url = input["url"].as_str().ok_or("net.fetch needs a url")?;
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("net.fetch only supports http:// and https:// URLs".into());
    }
    let method = input["method"]
        .as_str()
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let timeout_secs = input["timeoutMs"]
        .as_u64()
        .map(|ms| (ms.max(1000) as f64 / 1000.0).min(120.0))
        .unwrap_or(30.0);

    let header_file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let body_out = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;

    let mut cmd = Command::new("/usr/bin/curl");
    cmd.arg("-sS")
        .arg("-L")
        .arg("--max-time")
        .arg(format!("{timeout_secs:.1}"))
        .arg("-X")
        .arg(&method)
        .arg("-D")
        .arg(header_file.path())
        .arg("-o")
        .arg(body_out.path())
        .arg("-w")
        .arg("%{http_code}");

    if let Some(headers) = input["headers"].as_object() {
        for (k, v) in headers {
            if let Some(val) = v.as_str()
                && !k.contains('\r')
                && !k.contains('\n')
                && !val.contains('\r')
                && !val.contains('\n')
            {
                cmd.arg("-H").arg(format!("{k}: {val}"));
            }
        }
    }

    let body_bytes: Option<Vec<u8>> = if let Some(b64) = input["bodyBase64"].as_str() {
        Some(BASE64.decode(b64).map_err(|e| e.to_string())?)
    } else {
        input["body"].as_str().map(|s| s.as_bytes().to_vec())
    };
    if body_bytes.is_some() {
        cmd.arg("--data-binary").arg("@-");
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.arg(url);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("fetch failed: {e}"))?;
    if let (Some(mut stdin), Some(bytes)) = (child.stdin.take(), body_bytes) {
        let _ = stdin.write_all(&bytes);
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if err.is_empty() {
            "Network request failed".into()
        } else {
            err
        });
    }

    let status: u16 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    let raw_headers = fs::read_to_string(header_file.path()).unwrap_or_default();
    let mut headers = Map::new();
    let mut status_text = String::new();
    // Use the final response header block (after any redirects).
    if let Some(last_block) = raw_headers
        .split("\r\n\r\n")
        .filter(|b| !b.trim().is_empty())
        .last()
    {
        for (idx, line) in last_block.lines().enumerate() {
            if idx == 0 {
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                status_text = parts.get(2).copied().unwrap_or("").trim().to_owned();
            } else if let Some((k, v)) = line.split_once(':') {
                headers.insert(k.trim().to_lowercase(), json!(v.trim()));
            }
        }
    }
    let body = fs::read(body_out.path()).map_err(|e| e.to_string())?;
    if body.len() > MAX_READ_BYTES {
        return Err("Response body exceeded 16 MiB limit".into());
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    Ok(json!({
        "ok": (200..300).contains(&status),
        "status": status,
        "statusText": status_text,
        "headers": headers,
        "text": text,
        "bodyBase64": BASE64.encode(&body),
    }))
}

fn run_exec(input: &Value) -> Result<Value, String> {
    let program = input["program"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("shell.exec needs a program")?;
    let args: Vec<String> = input["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let timeout_ms = input["timeoutMs"]
        .as_u64()
        .unwrap_or(DEFAULT_EXEC_TIMEOUT_MS)
        .clamp(100, MAX_EXEC_TIMEOUT_MS);

    let mut cmd = Command::new(program);
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = input["cwd"].as_str().filter(|s| !s.is_empty()) {
        cmd.current_dir(expand_path(cwd)?);
    }
    if let Some(env) = input["env"].as_object() {
        for (k, v) in env {
            if let Some(val) = v.as_str() {
                cmd.env(k, val);
            }
        }
    }
    if input["stdin"].is_string() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start {program}: {e}"))?;
    if let (Some(mut stdin), Some(text)) = (child.stdin.take(), input["stdin"].as_str()) {
        let _ = stdin.write_all(text.as_bytes());
    }

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stdout_pipe {
            let _ = pipe.take(MAX_EXEC_OUTPUT_BYTES as u64).read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stderr_pipe {
            let _ = pipe.take(MAX_EXEC_OUTPUT_BYTES as u64).read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    Ok(json!({
        "code": status.and_then(|s| s.code()).unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
        "timedOut": timed_out,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_list_read_write_and_hidden_files() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("profile");
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let file = work.join("hello.txt");
        let hidden = work.join(".secret");

        run_worker_op(
            &profile,
            "file-browser",
            "ext.fs.writeText",
            &json!({"path": file.to_string_lossy(), "content": "hi powermove"}),
        )
        .unwrap();
        run_worker_op(
            &profile,
            "file-browser",
            "ext.fs.writeText",
            &json!({"path": hidden.to_string_lossy(), "content": "dotfile"}),
        )
        .unwrap();

        let text = run_worker_op(
            &profile,
            "file-browser",
            "ext.fs.readText",
            &json!({"path": file.to_string_lossy()}),
        )
        .unwrap();
        assert_eq!(text, json!("hi powermove"));

        let listed = run_worker_op(
            &profile,
            "file-browser",
            "ext.fs.listDir",
            &json!({"path": work.to_string_lossy(), "includeHidden": true}),
        )
        .unwrap();
        let names: Vec<&str> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert_eq!(names, vec![".secret", "hello.txt"]);

        let visible_only = run_worker_op(
            &profile,
            "file-browser",
            "ext.fs.listDir",
            &json!({"path": work.to_string_lossy(), "includeHidden": false}),
        )
        .unwrap();
        let vis_names: Vec<&str> = visible_only["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert_eq!(vis_names, vec!["hello.txt"]);
    }

    #[test]
    fn persistent_storage_round_trips_and_clears() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path();
        run_worker_op(
            profile,
            "my-ext",
            "ext.storage.set",
            &json!({"key": "lastFolder", "value": "/Users/test"}),
        )
        .unwrap();
        assert_eq!(
            run_worker_op(profile, "my-ext", "ext.storage.get", &json!({"key": "lastFolder"}))
                .unwrap(),
            json!("/Users/test")
        );
        assert_eq!(
            run_worker_op(profile, "my-ext", "ext.storage.keys", &json!({})).unwrap(),
            json!(["lastFolder"])
        );
        run_worker_op(profile, "my-ext", "ext.storage.clear", &json!({})).unwrap();
        assert_eq!(
            run_worker_op(profile, "my-ext", "ext.storage.get", &json!({"key": "lastFolder"}))
                .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn shell_exec_runs_commands_with_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_worker_op(
            tmp.path(),
            "my-ext",
            "ext.shell.exec",
            &json!({"program": "/bin/echo", "args": ["hello"]}),
        )
        .unwrap();
        assert_eq!(out["code"], 0);
        assert_eq!(out["stdout"], "hello\n");
        assert_eq!(out["timedOut"], false);
    }
}
