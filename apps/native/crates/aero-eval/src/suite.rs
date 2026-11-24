//! The versioned task file (`data/suite.json`) and its checks: every kind has
//! its tasks, fixtures match their pinned hashes, prompts read like a person
//! and never name a tool, and every task has a programmatic grader.
use crate::graders::GraderSpec;
use crate::graders::rubric::Rubric;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const FORMAT: &str = "aero-eval-suite";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub format: String,
    /// Bumped whenever a task, fixture or grader changes: runs of different
    /// versions are never pooled.
    pub version: u32,
    pub tasks: Vec<Task>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Cut,
    Grade,
    AddEffect,
    NewEffect,
    NewTransition,
    BuildPanel,
    FixExtension,
    Restyle,
    MultiStep,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Cut,
        Kind::Grade,
        Kind::AddEffect,
        Kind::NewEffect,
        Kind::NewTransition,
        Kind::BuildPanel,
        Kind::FixExtension,
        Kind::Restyle,
        Kind::MultiStep,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Cut => "cut",
            Kind::Grade => "grade",
            Kind::AddEffect => "add-effect",
            Kind::NewEffect => "new-effect",
            Kind::NewTransition => "new-transition",
            Kind::BuildPanel => "build-panel",
            Kind::FixExtension => "fix-extension",
            Kind::Restyle => "restyle",
            Kind::MultiStep => "multi-step",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Breakage {
    Compile,
    Runtime,
    Output,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: String,
    pub kind: Kind,
    /// What the person types, as a pro editor would say it.
    pub prompt: String,
    pub fixture: Fixture,
    pub graders: Vec<GraderSpec>,
    pub limits: Limits,
    /// For fix-extension tasks: what is broken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breakage: Option<Breakage>,
    /// Why the task is in the suite (for reviewers; never shown to a harness).
    pub why: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// Project file under `data/fixtures/`.
    pub project: String,
    /// Extension folders under `data/fixtures/` installed into the clean
    /// profile before the run (a fix task's broken extension).
    #[serde(default)]
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Wall-clock limit; the harness is stopped at it and the run fails.
    pub wall_secs: u64,
    /// Tool-call limit; more calls than this fails the run.
    pub steps: u32,
}

/// Words a prompt may not contain: tool names and implementation vocabulary a
/// person typing into the agent panel would not use.
const HINT_WORDS: [&str; 14] = [
    "mcp",
    "json",
    "api",
    "glsl",
    "staging",
    "manifest",
    "register",
    "kernel",
    "typescript",
    "svelte",
    "uniform",
    "frag",
    "commit",
    "rollback",
];

impl Suite {
    pub fn load(data_root: &Path) -> Result<Self, String> {
        let path = data_root.join("suite.json");
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Every problem found, so a broken suite is fixed in one pass.
    /// `tool_names` are the tool names a harness can see (the MCP catalog).
    pub fn validate(&self, data_root: &Path, tool_names: &[String]) -> Vec<String> {
        let mut problems = Vec::new();
        if self.format != FORMAT {
            problems.push(format!("format is {}, expected {FORMAT}", self.format));
        }
        if self.version == 0 {
            problems.push("version must be at least 1".into());
        }
        let pinned = match load_pins(data_root) {
            Ok(p) => p,
            Err(e) => {
                problems.push(e);
                BTreeMap::new()
            }
        };
        let mut ids = BTreeSet::new();
        let mut per_kind: BTreeMap<Kind, usize> = BTreeMap::new();
        let mut breakages = BTreeSet::new();
        for t in &self.tasks {
            let p = |m: String| format!("{}: {m}", t.id);
            if !ids.insert(t.id.as_str()) {
                problems.push(p("id appears twice".into()));
            }
            if t.id.is_empty()
                || !t
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                problems.push(p("id must be lowercase letters, digits and dashes".into()));
            }
            *per_kind.entry(t.kind).or_default() += 1;
            problems.extend(prompt_problems(&t.prompt, tool_names).into_iter().map(p));
            if t.why.trim().is_empty() {
                problems.push(p("say why the task exists".into()));
            }
            match (t.kind, t.breakage) {
                (Kind::FixExtension, Some(b)) => {
                    breakages.insert(b);
                    if t.fixture.extensions.is_empty() {
                        problems.push(p("a fix task ships its broken extension".into()));
                    }
                }
                (Kind::FixExtension, None) => {
                    problems.push(p("a fix task names its breakage".into()))
                }
                (_, Some(_)) => problems.push(p("only fix tasks have a breakage".into())),
                _ => {}
            }
            for f in std::iter::once(&t.fixture.project).chain(&t.fixture.extensions) {
                let path = data_root.join("fixtures").join(f);
                if !path.exists() {
                    problems.push(p(format!("fixture {f} does not exist")));
                    continue;
                }
                match tree_hash(&path) {
                    Err(e) => problems.push(p(e)),
                    Ok(h) => match pinned.get(f) {
                        None => problems.push(p(format!("fixture {f} is not pinned in manifest"))),
                        Some(want) if want != &h => problems.push(p(format!(
                            "fixture {f} changed (hash {h}, pinned {want}); regenerate and bump the suite version"
                        ))),
                        Some(_) => {}
                    },
                }
            }
            if t.limits.wall_secs == 0 || t.limits.wall_secs > 3600 || t.limits.steps == 0 {
                problems.push(p("limits must be 1..3600 s and at least one step".into()));
            }
            if t.graders.is_empty() {
                problems.push(p("no graders".into()));
            }
            if t.graders.iter().all(|g| matches!(g, GraderSpec::Rubric(_))) {
                problems.push(p("needs at least one programmatic grader".into()));
            }
            for g in &t.graders {
                if let GraderSpec::Rubric(r) = g
                    && let Err(e) = Rubric::load(&data_root.join(&r.rubric))
                {
                    problems.push(p(e));
                }
                if g.judges_render() && g.frame_times().is_empty() {
                    problems.push(p(format!("{} grader names no frame times", g.name())));
                }
                if let GraderSpec::Frames(s) = g
                    && let crate::graders::frames::Reference::File(f) = &s.reference
                    && !f.starts_with("references/")
                {
                    problems.push(p("reference frames live under references/".into()));
                }
            }
        }
        for k in Kind::ALL {
            let n = per_kind.get(&k).copied().unwrap_or(0);
            if n < 2 {
                problems.push(format!(
                    "kind {} has {n} tasks, needs at least 2",
                    k.as_str()
                ));
            }
        }
        for b in [Breakage::Compile, Breakage::Runtime, Breakage::Output] {
            if !breakages.contains(&b) {
                problems.push(format!("no fix task for a {b:?} breakage"));
            }
        }
        problems
    }
}

/// Problems with a prompt: empty, too long, or hinting at the tools.
pub fn prompt_problems(prompt: &str, tool_names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let lower = prompt.to_lowercase();
    if prompt.trim().is_empty() {
        out.push("empty prompt".into());
    }
    if prompt.chars().count() > 400 {
        out.push("prompt is longer than a person types (400 characters)".into());
    }
    let words: BTreeSet<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .map(|w| w.trim_matches('.'))
        .filter(|w| !w.is_empty())
        .collect();
    for w in HINT_WORDS {
        if words.contains(w) {
            out.push(format!("prompt says \"{w}\", which hints at the tools"));
        }
    }
    for t in tool_names {
        if lower.contains(&t.to_lowercase()) {
            out.push(format!("prompt names the tool {t}"));
        }
    }
    if lower.contains('_') || lower.contains('`') {
        out.push("prompt contains identifiers (underscores or backticks)".into());
    }
    out
}

/// Pins in `data/fixtures/manifest.json`, written by `generate.py`.
pub fn load_pins(data_root: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = data_root.join("fixtures/manifest.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    v.get("pins")
        .and_then(|p| p.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .ok_or_else(|| format!("{}: no pins", path.display()))
}

/// SHA-256 of a file, or of a folder as sorted `path\0sha\n` lines (the same
/// rule `generate.py` and the runner's extension change detection use).
pub fn tree_hash(path: &Path) -> Result<String, String> {
    if path.is_file() {
        return std::fs::read(path)
            .map(|b| crate::sha256_hex(&b))
            .map_err(|e| format!("{}: {e}", path.display()));
    }
    let mut files = Vec::new();
    walk(path, path, &mut files)?;
    files.sort();
    let mut text = String::new();
    for (rel, sha) in files {
        text.push_str(&format!("{rel}\0{sha}\n"));
    }
    Ok(crate::sha256_hex(text.as_bytes()))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in rd {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out)?;
        } else {
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            out.push((rel, crate::sha256_hex(&bytes)));
        }
    }
    Ok(())
}

/// Tool names from the MCP catalog fixture (what Codex and Claude Code see)
/// plus Aero's own tool names that are not ordinary words.
pub fn tool_names(catalog: &Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(catalog).map_err(|e| format!("{}: {e}", catalog.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = v["tools"]
        .as_array()
        .ok_or("catalog has no tools")?
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect();
    names.extend(["panel_begin", "panel_build"].map(String::from));
    Ok(names)
}
