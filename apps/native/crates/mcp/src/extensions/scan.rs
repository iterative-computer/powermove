//! Credential scan of staged source, as main's `@powermove/registry/scan`:
//! credentials never reach the library; extensions declare `vars` instead.
use regex::Regex;
use std::{collections::BTreeMap, fs, path::Path, sync::OnceLock};

const FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 4000;

struct Patterns {
    text_file: Regex,
    hard: Vec<(&'static str, Regex)>,
    openai: Regex,
    waiver: Regex,
    path_like: Regex,
    hex: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let r = |s: &str| Regex::new(s).expect("static pattern");
        Patterns {
            text_file: r(r"(?i)(?:\.(?:ts|tsx|js|jsx|mjs|svelte|json|md|txt|css|html|frag|vert|glsl|wgsl|yml|yaml|toml)|(?:^|/)\.env[^/]*)$"),
            hard: vec![
                ("an Anthropic API key", r(r"(?-u:\b)sk-ant-[A-Za-z0-9_-]{16,}(?-u:\b)")),
                ("a Stripe key", r(r"(?-u:\b)(?:sk_live_|rk_live_)[A-Za-z0-9]{16,}(?-u:\b)")),
                ("an AWS access key", r(r"(?-u:\b)AKIA[0-9A-Z]{16}(?-u:\b)")),
                ("a GitHub token", r(r"(?-u:\b)(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})(?-u:\b)")),
                ("a GitLab token", r(r"(?-u:\b)glpat-[A-Za-z0-9_-]{16,}(?-u:\b)")),
                ("a Slack token", r(r"(?-u:\b)xox[abpr]-[A-Za-z0-9-]{16,}(?-u:\b)")),
                ("a Google API key", r(r"(?-u:\b)AIza[0-9A-Za-z_-]{35}(?-u:\b)")),
                ("a JWT", r(r"(?-u:\b)eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+(?-u:\b)")),
                ("a private key", r(r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----")),
            ],
            openai: r(r"(?-u:\b)sk-[A-Za-z0-9_-]{16,}(?-u:\b)"),
            waiver: r(r"powermove-secret-ok:\s*(.{3,200})"),
            path_like: r(r"(?i)^(?:https?://|data:image/|\.?\.?/)"),
            hex: r(r"(?i)^#[0-9a-f]+$"),
        }
    })
}
/// `sk-` keys other than Anthropic's and Stripe's, at any start (the tag's lookahead).
fn openai(text: &str) -> bool {
    let mut at = 0;
    while let Some(m) = patterns().openai.find_at(text, at) {
        let rest = &m.as_str()[3..];
        if !rest.starts_with("ant-") && !rest.starts_with("live_") {
            return true;
        }
        at = m.start() + 1;
    }
    false
}
fn hard(text: &str) -> Option<&'static str> {
    let p = patterns();
    if openai(text) {
        return Some("an OpenAI API key");
    }
    p.hard
        .iter()
        .find(|(_, pattern)| pattern.is_match(text))
        .map(|(label, _)| *label)
}
fn entropy(s: &str) -> f64 {
    let mut counts = BTreeMap::new();
    let n = s.chars().count() as f64;
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}
fn waived(lines: &[&str], index: usize) -> bool {
    [Some(index), index.checked_sub(1)]
        .into_iter()
        .flatten()
        .filter_map(|i| lines.get(i))
        .filter_map(|line| line.find("//").map(|c| &line[c..]))
        .any(|comment| {
            patterns()
                .waiver
                .captures(comment)
                .is_some_and(|m| m[1].trim().len() >= 3)
        })
}
/// Quoted tokens, as `/(["'`])([^"'`\r\n]*?)\1/g` matches them.
fn quoted(line: &str) -> Vec<&str> {
    let quote = |c: char| matches!(c, '"' | '\'' | '`');
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let (mut out, mut i) = (vec![], 0);
    while i < chars.len() {
        let (start, q) = chars[i];
        if !quote(q) {
            i += 1;
            continue;
        }
        let close = (i + 1..chars.len()).find(|&k| quote(chars[k].1) || matches!(chars[k].1, '\r' | '\n'));
        match close {
            Some(k) if chars[k].1 == q => {
                out.push(&line[start + 1..chars[k].0]);
                i = k + 1;
            }
            _ => i += 1,
        }
    }
    out
}
fn suspicious(value: &str) -> bool {
    let p = patterns();
    value.encode_utf16().count() >= 32
        && !value.chars().any(char::is_whitespace)
        && value.chars().any(|c| c.is_ascii_alphabetic())
        && value.chars().any(|c| c.is_ascii_digit())
        && !p.path_like.is_match(value)
        && !value.contains(['/', '\\'])
        && !p.hex.is_match(value)
        && !value.contains([';', ':', '(', ')', ',', '{', '}', '<', '>'])
        && hard(value).is_none()
        && entropy(value) > 4.5
}
/// Blocking findings of one text file, as `(line, label)`.
pub fn scan_text(path: &str, text: &str) -> Vec<(usize, &'static str)> {
    if !patterns().text_file.is_match(path) || text.as_bytes()[..text.len().min(8192)].contains(&0) {
        return vec![];
    }
    let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let mut findings = vec![];
    for (i, line) in lines.iter().enumerate() {
        if let Some(label) = hard(line) {
            findings.push((i + 1, label));
        }
        for value in quoted(line) {
            if suspicious(value) && !waived(&lines, i) {
                findings.push((i + 1, "a secret"));
            }
        }
    }
    findings
}
/// Every blocking credential in an extension folder, as agent-facing lines.
pub fn scan_extension(id: &str, root: &Path) -> std::io::Result<Vec<String>> {
    fn walk(dir: &Path, prefix: &str, seen: &mut usize, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
        let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "node_modules" || name == ".git" {
                continue;
            }
            *seen += 1;
            if *seen > MAX_ENTRIES {
                return Err(std::io::Error::other(format!(
                    "{} has more than {MAX_ENTRIES} files; nothing was published.",
                    dir.display()
                )));
            }
            let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.is_dir() {
                walk(&entry.path(), &relative, seen, out)?;
            } else if meta.is_file() && meta.len() <= FILE_BYTES && patterns().text_file.is_match(&relative) {
                let bytes = fs::read(entry.path())?;
                if !bytes[..bytes.len().min(8192)].contains(&0) {
                    out.push((relative, String::from_utf8_lossy(&bytes).into_owned()));
                }
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(root, "", &mut 0, &mut files)?;
    Ok(files
        .iter()
        .flat_map(|(path, text)| {
            scan_text(path, text).into_iter().map(move |(line, label)| {
                format!("{id}/{path}:{line} looks like {label}. Declare it in manifest vars and read api.vars. Nothing was published.")
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::scan_text;
    #[test]
    fn credentials_block_and_ordinary_source_does_not() {
        // Built at runtime so no credential-shaped literal sits in the repository.
        let key = format!("sk-{}", "a1B2".repeat(6));
        let line = format!("const k = \"{key}\";");
        assert_eq!(scan_text("index.ts", &line), vec![(1, "an OpenAI API key")]);
        let anthropic = format!("x = '{}{}'", "sk-ant-", "Zz9".repeat(8));
        assert_eq!(scan_text("a.js", &anthropic)[0].1, "an Anthropic API key");
        let alphabet: Vec<char> = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMN0123456789".chars().collect();
        let random: String = (0..32).map(|i| alphabet[(i * 7) % alphabet.len()]).collect();
        assert_eq!(scan_text("a.ts", &format!("t = `{random}`")), vec![(1, "a secret")]);
        let waived = format!("t = '{random}' // powermove-secret-ok: public fixture id");
        assert!(scan_text("a.ts", &waived).is_empty());
        assert!(scan_text("logo.png", &line).is_empty());
        for ordinary in [
            "import { view } from './panel';",
            "const url = 'https://example.com/a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6';",
            "color: '#a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8'",
            "const sk = 'sk-ant-'; const s = \"short9\";",
        ] {
            assert!(scan_text("a.ts", ordinary).is_empty(), "{ordinary}");
        }
    }
}
