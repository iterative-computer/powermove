use crate::{Error, Result};
use serde_json::{Value, json};
use std::{collections::HashSet, ffi::OsString, io::Read, path::PathBuf};

pub const USAGE: &str = "pm-cli <open|info|timeline|validate> PROJECT [--args JSON | --args-file FILE|-]\npm-cli apply PROJECT --output NEW.pmv [--expected-revision N] --args '{\"commands\":[...]}'\npm-cli render-frames PROJECT [--args '{\"times\":[0],\"width\":720}'] [--backend native|test-double]\npm-cli export PROJECT --output NEW.mp4 [--args JSON] [--backend native|test-double]\npm-cli migrate PROFILE --dry-run --scratch NEW_DIRECTORY\nUse --expected-source-hash SHA256 to bind a request across CLI processes.\nAll output is one MCP result JSON on stdout. Paths are explicit; no real profile is discovered.\nExit: 0 success, 1 operation/validation failure, 2 usage/arguments, 3 unavailable backend, 4 revision conflict, 5 I/O.\nLinux native rendering/export requires macOS; test-double produces labelled synthetic pixels/manifest only.";

#[derive(Debug)]
pub struct Options {
    pub command: String,
    pub project: PathBuf,
    pub arguments: Value,
    pub output: Option<PathBuf>,
    pub expected_revision: Option<u64>,
    pub expected_source_hash: Option<String>,
    pub backend: String,
    pub scratch: Option<PathBuf>,
}
impl Options {
    pub fn parse(raw: impl IntoIterator<Item = OsString>, stdin: &mut dyn Read) -> Result<Self> {
        let mut it = raw.into_iter();
        let command = it.next().unwrap_or_else(|| "help".into());
        let command = command
            .to_str()
            .ok_or_else(|| Error::invalid("Non-UTF8 command"))?;
        if matches!(command, "help" | "--help" | "-h") {
            if it.next().is_some() {
                return Err(Error::invalid("Unexpected help arguments"));
            }
            return Ok(Self {
                command: "help".into(),
                project: PathBuf::new(),
                arguments: json!({}),
                output: None,
                expected_revision: None,
                expected_source_hash: None,
                backend: "native".into(),
                scratch: None,
            });
        }
        if ![
            "open",
            "info",
            "timeline",
            "apply",
            "render-frames",
            "export",
            "migrate",
            "validate",
        ]
        .contains(&command)
        {
            return Err(Error::invalid(format!("Unknown command: {command}")));
        }
        let project = PathBuf::from(
            it.next()
                .ok_or_else(|| Error::invalid("Expected project/profile path"))?,
        );
        let mut result = Self {
            command: command.into(),
            project,
            arguments: json!({}),
            output: None,
            expected_revision: None,
            expected_source_hash: None,
            backend: "native".into(),
            scratch: None,
        };
        let mut seen = HashSet::new();
        while let Some(flag) = it.next() {
            let flag = flag
                .to_str()
                .ok_or_else(|| Error::invalid("Non-UTF8 flag"))?;
            let key = if flag == "--args-file" {
                "--args"
            } else {
                flag
            };
            if !seen.insert(key.to_owned()) {
                return Err(Error::invalid(format!("Duplicate option: {flag}")));
            }
            if flag == "--dry-run" {
                continue;
            }
            let value = it
                .next()
                .ok_or_else(|| Error::invalid(format!("Missing value: {flag}")))?;
            match flag {
                "--output" => result.output = Some(value.into()),
                "--scratch" => result.scratch = Some(value.into()),
                "--expected-source-hash" => {
                    let hash = value
                        .into_string()
                        .map_err(|_| Error::invalid("Invalid SHA-256"))?;
                    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(Error::invalid("Expected a 64-digit SHA-256"));
                    }
                    result.expected_source_hash = Some(hash.to_ascii_lowercase());
                }
                "--expected-revision" => {
                    result.expected_revision = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(|| Error::invalid("Expected unsigned revision"))?,
                    )
                }
                "--backend" => {
                    result.backend = value
                        .into_string()
                        .map_err(|_| Error::invalid("Invalid backend"))?
                }
                "--args" => {
                    result.arguments = serde_json::from_slice(value.as_encoded_bytes())
                        .map_err(|e| Error::invalid(e.to_string()))?
                }
                "--args-file" => {
                    let mut bytes = Vec::new();
                    if value == "-" {
                        stdin.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                    } else {
                        std::fs::File::open(value)?
                            .take(16 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes)?;
                    }
                    if bytes.len() > 16 * 1024 * 1024 {
                        return Err(Error::invalid("Arguments exceed 16 MiB"));
                    }
                    result.arguments = serde_json::from_slice(&bytes)
                        .map_err(|e| Error::invalid(e.to_string()))?;
                }
                _ => return Err(Error::invalid(format!("Unknown option: {flag}"))),
            }
        }
        if !result.arguments.is_object() {
            return Err(Error::invalid("Arguments must be a JSON object"));
        }
        if !["native", "test-double"].contains(&result.backend.as_str()) {
            return Err(Error::invalid("Unknown backend"));
        }
        let allowed: &[&str] = match command {
            "apply" => &["--output", "--args", "--expected-revision"],
            "export" => &["--output", "--args", "--expected-revision", "--backend"],
            "render-frames" => &["--args", "--expected-revision", "--backend"],
            "migrate" => &["--scratch", "--dry-run"],
            "validate" => &["--expected-revision"],
            _ => &["--args", "--expected-revision"],
        };
        if seen.iter().any(|s| {
            !(allowed.contains(&s.as_str())
                || s == "--expected-source-hash" && command != "migrate")
        }) {
            return Err(Error::invalid("Option does not apply to this command"));
        }
        if matches!(command, "apply" | "export") && result.output.is_none() {
            return Err(Error::invalid(
                "--output is required; existing destinations are refused",
            ));
        }
        if command == "migrate" && (!seen.contains("--dry-run") || result.scratch.is_none()) {
            return Err(Error::invalid(
                "migrate requires --dry-run --scratch NEW_DIRECTORY",
            ));
        }
        Ok(result)
    }
}
