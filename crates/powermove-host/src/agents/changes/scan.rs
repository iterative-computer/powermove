//! Credential scan of staged extension sources (port of `scanText` /
//! `scanFiles` in `packages/registry/src/scan.ts`). Credentials never reach
//! the library: values are declared in manifest vars and read at runtime.

use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Result, bail};
use regex::Regex;

const SCAN_FILE_BYTES: u64 = 2 * 1024 * 1024;
const SCAN_MAX_ENTRIES: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub kind: &'static str,
}

/// `(kind, label, pattern)`. `openai_key` excludes `sk-ant-` / `sk-live_` in code: no look-ahead.
static HARD: LazyLock<Vec<(&'static str, &'static str, Regex)>> = LazyLock::new(|| {
    [
        ("anthropic_key", "an Anthropic API key", r"\bsk-ant-[A-Za-z0-9_-]{16,}\b"),
        ("stripe_key", "a Stripe key", r"\b(?:sk_live_|rk_live_)[A-Za-z0-9]{16,}\b"),
        ("openai_key", "an OpenAI API key", r"\bsk-([A-Za-z0-9_-]{16,})\b"),
        ("aws_access_key", "an AWS access key", r"\bAKIA[0-9A-Z]{16}\b"),
        ("github_token", "a GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b"),
        ("gitlab_token", "a GitLab token", r"\bglpat-[A-Za-z0-9_-]{16,}\b"),
        ("slack_token", "a Slack token", r"\bxox[abpr]-[A-Za-z0-9-]{16,}\b"),
        ("google_api_key", "a Google API key", r"\bAIza[0-9A-Za-z_-]{35}\b"),
        ("jwt", "a JWT", r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b"),
        ("pem_private_key", "a private key", r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----"),
    ]
    .into_iter()
    // JavaScript's `\b` is ASCII.
    .map(|(kind, label, pattern)| (kind, label, Regex::new(&format!("(?-u:{pattern})")).unwrap()))
    .collect()
});

static TEXT_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:\.(?:ts|js|mjs|svelte|json|md|txt|css|html|frag|vert|glsl|wgsl|yml|yaml|toml)|(?:^|/)\.env[^/]*)$").unwrap());
static WAIVER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"powermove-secret-ok:\s*(.{3,200})").unwrap());

pub fn label(kind: &str) -> &'static str {
    HARD.iter().find(|(name, ..)| *name == kind).map_or("a secret", |(_, label, _)| label)
}

fn hard_match(kind: &str, pattern: &Regex, text: &str) -> bool {
    if kind != "openai_key" {
        return pattern.is_match(text);
    }
    pattern.captures_iter(text).any(|captures| !captures[1].starts_with("ant-") && !captures[1].starts_with("live_"))
}

fn entropy(value: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let mut length = 0usize;
    for character in value.chars() {
        *counts.entry(character).or_insert(0usize) += 1;
        length += 1;
    }
    counts.values().map(|count| *count as f64 / length as f64).map(|p| -p * p.log2()).sum()
}

fn waived(lines: &[&str], index: usize) -> bool {
    [Some(lines[index]), index.checked_sub(1).map(|previous| lines[previous])].into_iter().flatten().any(|line| {
        line.find("//").and_then(|comment| WAIVER.captures(&line[comment..])).is_some_and(|captures| captures[1].trim().chars().count() >= 3)
    })
}

/// Quoted values on one line: `(["'`])([^"'`\r\n]*?)\1` without a back-reference.
fn quoted(line: &str) -> Vec<&str> {
    let quotes: Vec<(usize, char)> = line.char_indices().filter(|(_, character)| matches!(character, '"' | '\'' | '`')).collect();
    let mut values = Vec::new();
    let mut index = 0;
    while index + 1 < quotes.len() {
        let ((open, quote), (close, next)) = (quotes[index], quotes[index + 1]);
        if quote == next {
            values.push(&line[open + 1..close]);
            index += 2;
        } else {
            index += 1;
        }
    }
    values
}

/// Unwaived findings in one file (`scanText`, minus waived ones).
pub fn scan_text(path: &str, text: &str) -> Vec<Finding> {
    if !TEXT_FILE.is_match(path) || text.as_bytes()[..text.len().min(8192)].contains(&0) {
        return Vec::new();
    }
    let lines: Vec<&str> = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    let mut findings = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        for (kind, _, pattern) in HARD.iter() {
            if hard_match(kind, pattern, line) {
                findings.push(Finding { path: path.to_owned(), line: index + 1, kind });
            }
        }
        for value in quoted(line) {
            if value.chars().count() < 32 || value.chars().any(char::is_whitespace) || !value.chars().any(|c| c.is_ascii_alphabetic()) || !value.chars().any(|c| c.is_ascii_digit()) {
                continue;
            }
            let lower = value.to_ascii_lowercase();
            let hex_color = value.starts_with('#') && value.len() > 1 && value[1..].chars().all(|c| c.is_ascii_hexdigit());
            if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:image/") || value.starts_with("./") || value.starts_with("../") || value.contains('/') || value.contains('\\') || hex_color {
                continue;
            }
            if value.contains([';', ':', '(', ')', ',', '{', '}', '<', '>']) || HARD.iter().any(|(kind, _, pattern)| hard_match(kind, pattern, value)) || entropy(value) <= 4.5 {
                continue;
            }
            if !waived(&lines, index) {
                findings.push(Finding { path: path.to_owned(), line: index + 1, kind: "high_entropy" });
            }
        }
    }
    findings
}

/// Every unwaived finding under one staged extension folder (`readStagedText` + `scanFiles`).
pub fn scan_folder(root: &Path) -> Result<Vec<Finding>> {
    fn walk(directory: &Path, prefix: &str, visited: &mut usize, findings: &mut Vec<Finding>) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "node_modules" || name == ".git" {
                continue;
            }
            // Refusing beats a silent stop that would let a credential through.
            *visited += 1;
            if *visited > SCAN_MAX_ENTRIES {
                bail!("This extension has more than {SCAN_MAX_ENTRIES} files; nothing was published.");
            }
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                walk(&entry.path(), &relative, visited, findings)?;
            } else if metadata.is_file() && metadata.len() <= SCAN_FILE_BYTES && TEXT_FILE.is_match(&relative) {
                let bytes = std::fs::read(entry.path())?;
                findings.extend(scan_text(&relative, &String::from_utf8_lossy(&bytes)));
            }
        }
        Ok(())
    }
    let mut findings = Vec::new();
    walk(root, "", &mut 0, &mut findings)?;
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_credentials_but_not_ordinary_code() {
        let found = |text: &str| scan_text("index.ts", text).into_iter().map(|finding| finding.kind).collect::<Vec<_>>();
        // Synthetic values, assembled here so no literal looks like a credential.
        let tail = "x".repeat(20);
        let random: String = ('a'..='q').zip('A'..='Q').zip(('0'..='9').cycle()).flat_map(|((lower, upper), digit)| [lower, upper, digit]).take(40).collect();
        assert_eq!(found(&format!("const key = 'sk-{tail}';")), ["openai_key"]);
        assert_eq!(found(&format!("const key = 'sk-ant-{tail}';")), ["anthropic_key"]);
        assert_eq!(found(&format!("const value = \"{random}\";")), ["high_entropy"]);
        assert!(found(&format!("// powermove-secret-ok: test fixture\nconst value = \"{random}\";")).is_empty());
        assert!(found(&format!("const url = 'https://example.com/{random}';")).is_empty());
        assert!(found("api.effects.register({ id: 'tint', frag: `uniform float amount;` });").is_empty());
        assert!(scan_text("image.png", &format!("sk-{tail}")).is_empty());
    }
}
