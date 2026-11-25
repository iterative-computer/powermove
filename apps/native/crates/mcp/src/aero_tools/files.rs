use super::*;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;

const MAX_SOURCE: u64 = 2 << 20;
const MAX_RESULTS: usize = 100;

fn child(root: &Path, relative: &str, create_parents: bool) -> Result<PathBuf> {
    let invalid = || {
        Error::invalid(
            "Use a path relative to this session's extension stage, with no links or parent components.",
        )
    };
    if !root.symlink_metadata()?.is_dir()
        || relative
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(invalid());
    }
    let mut parts = Path::new(relative).components().peekable();
    let mut path = root.to_owned();
    while let Some(part) = parts.next() {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        path.push(name);
        match path.symlink_metadata() {
            Ok(meta)
                if (parts.peek().is_some() && meta.is_dir())
                    || (parts.peek().is_none() && (meta.is_dir() || meta.is_file())) => {}
            Ok(_) => return Err(invalid()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if parts.peek().is_some() {
                    if create_parents {
                        fs::create_dir(&path)?;
                    } else {
                        return Err(error.into());
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

fn read(path: &Path) -> Result<String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::invalid("Choose a staged source file."));
    }
    let mut text = String::new();
    file.take(MAX_SOURCE + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_SOURCE {
        return Err(Error::invalid("Staged text files are limited to 2 MiB."));
    }
    Ok(text)
}

fn write(path: &Path, text: &str) -> Result<Value> {
    if text.len() as u64 > MAX_SOURCE {
        return Err(Error::invalid("Staged text files are limited to 2 MiB."));
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(json!({"hash":format!("{:x}", Sha256::digest(text.as_bytes())),"bytes":text.len()}))
}

fn collect(
    root: &Path,
    dir: &Path,
    needle: Option<&str>,
    rows: &mut Vec<Value>,
    remaining: &mut usize,
) -> Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_unstable_by_key(|entry| entry.file_name());
    for entry in entries {
        if rows.len() == MAX_RESULTS || *remaining == 0 {
            break;
        }
        *remaining -= 1;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            collect(root, &path, needle, rows, remaining)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).unwrap();
            if let Some(needle) = needle {
                if let Ok(text) = read(&path) {
                    for (i, line) in text
                        .lines()
                        .enumerate()
                        .filter(|(_, line)| line.contains(needle))
                    {
                        rows.push(json!({"path":relative,"line":i+1,"text":line.chars().take(400).collect::<String>()}));
                        if rows.len() == MAX_RESULTS {
                            break;
                        }
                    }
                }
            } else {
                rows.push(json!({"path":relative,"bytes":entry.metadata()?.len()}));
            }
        }
    }
    Ok(())
}

pub(super) fn call(stage: &Stage, op: &str, args: &Value) -> Result<Value> {
    let path = args["path"].as_str().unwrap_or("");
    if op == "discard" {
        if !extensions::extension_id(path) {
            return Err(Error::invalid(
                "Name one draft extension id in path to discard.",
            ));
        }
        let folder = child(&stage.staging, path, false)?;
        if stage.baseline.contains_key(path) {
            return Err(Error::invalid(
                "Use file edits to restore a modified baseline package.",
            ));
        }
        fs::remove_dir_all(&folder)?;
        return Ok(json!({"discarded":path}));
    }
    if ["list", "grep"].contains(&op) {
        let dir = if path.is_empty() {
            stage.staging.clone()
        } else {
            child(&stage.staging, path, false)?
        };
        let needle = if op == "grep" {
            Some(
                args["text"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::invalid("Pass a nonempty text search."))?,
            )
        } else {
            None
        };
        let mut rows = Vec::new();
        let mut remaining = 4096;
        collect(&stage.staging, &dir, needle, &mut rows, &mut remaining)?;
        return Ok(json!({"items":rows,"truncated":rows.len() == MAX_RESULTS || remaining == 0}));
    }
    if !path
        .split_once('/')
        .is_some_and(|(id, _)| extensions::extension_id(id))
    {
        return Err(Error::invalid(
            "Use a staged package path such as my-extension/index.ts.",
        ));
    }
    if path.split('/').skip(1).any(|part| part.starts_with('.')) {
        return Err(Error::invalid(
            "Hidden extension metadata is managed by Powermove.",
        ));
    }
    let entry = child(&stage.staging, path, op == "write")?;
    match op {
        "read" => Ok(json!({"path":path,"text":read(&entry)?})),
        "write" => write(&entry, text(args, "text")?),
        "edit" => {
            let old = read(&entry)?;
            let from = text(args, "find")?;
            let to = text(args, "replace")?;
            if from.is_empty() || old.matches(from).count() != 1 {
                return Err(Error::invalid(
                    "The search text must occur exactly once. Read the file and choose a more specific match.",
                ));
            }
            write(&entry, &old.replacen(from, to, 1))
        }
        _ => Err(Error::invalid(
            "Choose files op read, write, edit, list, grep or discard.",
        )),
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key]
        .as_str()
        .ok_or_else(|| Error::invalid(format!("Supply {key} as text.")))
}

pub(super) fn dirty(stage: &Stage) -> Result<Vec<String>> {
    let now = extensions::hashes(std::slice::from_ref(&stage.staging))?;
    Ok(now
        .keys()
        .chain(stage.baseline.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|&id| now.get(id) != stage.baseline.get(id))
        .cloned()
        .collect())
}
