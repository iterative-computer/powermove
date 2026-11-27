//! The parity corpus on disk (`docs/engine/corpus.md`). Captures live outside
//! the repository, under `~/Library/Application Support/Powermove Engine
//! Corpus/<capture-id>/`.

use std::path::{Path, PathBuf};

use pm_model::json::{Value, parse};

pub fn root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Application Support/Powermove Engine Corpus"))
}

/// The newest capture labelled `label` (`v1`, `v0`) under the root.
fn newest(root: &Path, label: &str) -> Option<PathBuf> {
    let prefix = format!("{label}-");
    let mut captures: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
                && p.join("projects").is_dir()
        })
        .collect();
    captures.sort();
    captures.pop()
}

/// The capture to compare against: corpus v1 when there is one, else v0.
pub fn default_capture() -> Option<PathBuf> {
    let root = root()?;
    newest(&root, "v1").or_else(|| newest(&root, "v0"))
}

/// One captured project: its name (`<group>/<name>` or `real/<slug>`), its
/// directory, and its evaluation dumps in time order.
pub struct Project {
    pub name: String,
    pub dir: PathBuf,
    pub dumps: Vec<PathBuf>,
}

impl Project {
    /// The project as the app held it after loading (`loaded.json`).
    pub fn loaded(&self) -> Result<Value, String> {
        let text = std::fs::read_to_string(self.dir.join("loaded.json"))
            .map_err(|e| format!("{}: {e}", self.dir.join("loaded.json").display()))?;
        parse(&text).map_err(|e| format!("{}: {e}", self.dir.join("loaded.json").display()))
    }
}

pub fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn dumps(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<(f64, PathBuf)> = std::fs::read_dir(dir.join("eval"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .map(|p| {
                    let t = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(|s| s.strip_prefix('t'))
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(f64::INFINITY);
                    (t, p)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, p)| p).collect()
}

/// Every project with a loaded document and evaluation dumps:
/// `projects/<group>/<name>` and `real/<slug>`, sorted by name.
pub fn projects(capture: &Path) -> Vec<Project> {
    let mut out = Vec::new();
    for top in ["projects", "real"] {
        let Ok(groups) = std::fs::read_dir(capture.join(top)) else {
            continue;
        };
        for group in groups.filter_map(|e| e.ok()) {
            let gp = group.path();
            if gp.join("loaded.json").is_file() {
                out.push(Project {
                    name: format!("{top}/{}", group.file_name().to_string_lossy()),
                    dumps: dumps(&gp),
                    dir: gp,
                });
                continue;
            }
            let Ok(names) = std::fs::read_dir(&gp) else {
                continue;
            };
            for name in names.filter_map(|e| e.ok()) {
                let p = name.path();
                if p.join("loaded.json").is_file() && p.join("eval").is_dir() {
                    out.push(Project {
                        name: format!(
                            "{}/{}",
                            group.file_name().to_string_lossy(),
                            name.file_name().to_string_lossy()
                        ),
                        dumps: dumps(&p),
                        dir: p,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
