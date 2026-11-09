//! The frozen compiler's source boundary, without esbuild or Node. Each file is
//! transformed on its own; V8 then links the graph with real ES module semantics,
//! so nothing here reimplements scope hoisting. Source text never executes here.
use crate::manifest;
use pm_runtime_v8::{SvelteCompiler, SvelteUnit, module_imports, strip_typescript, wait_until};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub const BUNDLE: &str = "powermove:bundle";
pub const STYLES: &str = "powermove:styles";
const STYLE_MODULE: &str = "export const styles = [];
const listeners = new Set();
export function add(css) { styles.push(css); for (const listener of listeners) listener(css); }
export function acquire(listener) {
  for (const css of styles) listener(css);
  listeners.add(listener);
  return () => listeners.delete(listener);
}";
const SUFFIXES: [&str; 7] = [
    ".ts",
    ".tsx",
    ".js",
    ".mjs",
    ".svelte",
    ".svelte.ts",
    ".svelte.js",
];
/// Bump with any change to module transforms; compiled graphs are cached by it.
const COMPILER: &str = "native-1";

fn runtime_import(specifier: &str, surface: bool) -> bool {
    if surface {
        return specifier == "preact"
            || specifier.starts_with("preact/")
            || specifier.starts_with("powermove/");
    }
    specifier == "powermove" || specifier == "svelte" || specifier.starts_with("svelte/")
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleGraph {
    /// Module id (extension-relative path or a `powermove:` module) to ES source.
    pub sources: BTreeMap<String, String>,
    /// Module id to `specifier -> module id`. Runtime targets are `runtime:<name>`.
    pub edges: BTreeMap<String, BTreeMap<String, String>>,
    pub hash: String,
}
impl ModuleGraph {
    pub fn to_json(&self) -> Value {
        json!({"sources":self.sources,"edges":self.edges,"hash":self.hash})
    }
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            sources: serde_json::from_value(value["sources"].clone()).ok()?,
            edges: serde_json::from_value(value["edges"].clone()).ok()?,
            hash: value["hash"].as_str()?.to_owned(),
        })
    }
}

fn hidden(path: &Path) -> bool {
    path.components().any(|c| match c {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => false,
    })
}
fn node_modules(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::Normal(n) if n == "node_modules"))
}
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => (),
            other => out.push(other),
        }
    }
    out
}

struct Sources<'a> {
    /// A v4 surface extension: JSX, and the published `powermove/*` modules.
    surface: bool,
    root: PathBuf,
    svelte: &'a SvelteCompiler,
    seen: BTreeMap<PathBuf, String>,
    bytes: usize,
    graph: ModuleGraph,
}
impl Sources<'_> {
    fn resolve(&self, specifier: &str, directory: &Path) -> Result<PathBuf, String> {
        if Path::new(specifier).is_absolute() {
            return Err(format!("Absolute imports are not allowed: {specifier}"));
        }
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return Err(format!("Unsupported bare import: {specifier}"));
        }
        let unresolved = normalize(&directory.join(specifier));
        let relative = unresolved
            .strip_prefix(&self.root)
            .map_err(|_| format!("Import escapes extension directory: {specifier}"))?;
        if hidden(relative) {
            return Err(format!(
                "Imports from hidden extension paths are not allowed: {specifier}"
            ));
        }
        if node_modules(relative) {
            return Err(format!("Import escapes extension directory: {specifier}"));
        }
        let text = unresolved.to_string_lossy().into_owned();
        let mut candidates = vec![unresolved.clone()];
        if unresolved.extension().is_some() {
            if text.ends_with(".svelte") {
                candidates.extend([format!("{text}.ts"), format!("{text}.js")].map(Into::into));
            }
        } else {
            candidates.extend(SUFFIXES.map(|s| PathBuf::from(format!("{text}{s}"))));
            candidates.extend(SUFFIXES.map(|s| unresolved.join(format!("index{s}"))));
        }
        for candidate in candidates {
            if !fs::metadata(&candidate).is_ok_and(|m| m.is_file()) {
                continue;
            }
            let canonical = fs::canonicalize(&candidate).map_err(|e| e.to_string())?;
            let relative = canonical
                .strip_prefix(&self.root)
                .map_err(|_| format!("Import escapes extension directory: {specifier}"))?;
            if hidden(relative) {
                return Err(format!(
                    "Imports from hidden extension paths are not allowed: {specifier}"
                ));
            }
            if node_modules(relative) {
                return Err(format!("Import escapes extension directory: {specifier}"));
            }
            return Ok(canonical);
        }
        Err(format!("Could not resolve source import: {specifier}"))
    }
    fn svelte(&self, source: &str, file: &Path, unit: SvelteUnit) -> Result<String, String> {
        let filename = file.to_string_lossy();
        let mut ticket = self
            .svelte
            .compile(source, &filename, unit)
            .map_err(|e| e.to_string())?;
        let result = wait_until(&mut ticket, Instant::now() + Duration::from_secs(60))
            .ok_or("Svelte compiler timed out")?;
        result.map(|o| o.javascript).map_err(|e| e.to_string())
    }
    fn transform(&self, source: &str, file: &Path) -> Result<String, String> {
        let name = file.to_string_lossy();
        if name.ends_with(".css") {
            return Ok(format!(
                "import {{ add }} from '{STYLES}'; add({});",
                json!(source)
            ));
        }
        if name.ends_with(".svelte") {
            return self.svelte(source, file, SvelteUnit::Component);
        }
        if name.ends_with(".svelte.ts") || name.ends_with(".svelte.js") {
            return self.svelte(source, file, SvelteUnit::Module);
        }
        if name.ends_with(".ts") || name.ends_with(".tsx") {
            // Diagnostics name the file as the author wrote it.
            return strip_typescript(source, &self.module_id(file)).map_err(|e| match e {
                pm_runtime_v8::Error::JavaScript(message) => message,
                other => other.to_string(),
            });
        }
        Ok(source.to_owned())
    }
    fn module_id(&self, file: &Path) -> String {
        file.strip_prefix(&self.root)
            .unwrap_or(file)
            .to_string_lossy()
            .into_owned()
    }
    fn load(&mut self, file: PathBuf) -> Result<String, String> {
        if let Some(id) = self.seen.get(&file) {
            return Ok(id.clone());
        }
        let id = self.module_id(&file);
        self.seen.insert(file.clone(), id.clone());
        if self.seen.len() > manifest::SOURCE_FILES {
            return Err(format!(
                "Extension exceeds source file limit ({})",
                manifest::SOURCE_FILES
            ));
        }
        let size = fs::symlink_metadata(&file)
            .map_err(|e| e.to_string())?
            .len() as usize;
        self.bytes = self.bytes.saturating_add(size);
        if size > manifest::SOURCE_BYTES || self.bytes > manifest::SOURCE_BYTES {
            return Err(format!(
                "Extension exceeds source size limit ({} bytes)",
                manifest::SOURCE_BYTES
            ));
        }
        let source =
            String::from_utf8_lossy(&fs::read(&file).map_err(|e| e.to_string())?).into_owned();
        let code = self.transform(&source, &file)?;
        let directory = file.parent().unwrap_or(&self.root).to_path_buf();
        self.link(&id, code, &directory)?;
        Ok(id)
    }
    fn link(&mut self, id: &str, code: String, directory: &Path) -> Result<(), String> {
        let imports = module_imports(&code).map_err(|e| e.to_string())?;
        self.graph.sources.insert(id.to_owned(), code);
        let mut edges = BTreeMap::new();
        for specifier in imports {
            let target = if specifier == STYLES {
                self.graph
                    .sources
                    .entry(STYLES.into())
                    .or_insert_with(|| STYLE_MODULE.into());
                STYLES.to_owned()
            } else if runtime_import(&specifier, self.surface) {
                format!("runtime:{specifier}")
            } else {
                let file = self.resolve(&specifier, directory)?;
                self.load(file)?
            };
            edges.insert(specifier, target);
        }
        self.graph.edges.insert(id.to_owned(), edges);
        Ok(())
    }
}

fn tree(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = BTreeSet::new();
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "node_modules" {
                continue;
            }
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                files.insert(entry.path());
            }
        }
    }
    Ok(files)
}
/// Cheap identity of a tree for change detection: names, sizes and mtimes.
pub fn fingerprint(root: &Path) -> Result<Vec<(PathBuf, u64, std::time::SystemTime)>, String> {
    tree(root)?
        .into_iter()
        .map(|file| {
            let meta = fs::metadata(&file).map_err(|e| e.to_string())?;
            Ok((
                file,
                meta.len(),
                meta.modified().map_err(|e| e.to_string())?,
            ))
        })
        .collect()
}
fn digest_tree(root: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update(COMPILER);
    hash.update(pm_runtime_v8::SVELTE_VERSION);
    for file in tree(root)? {
        let name = file.strip_prefix(root).unwrap_or(&file).to_string_lossy();
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        let bytes = fs::read(&file).map_err(|e| e.to_string())?;
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(&bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Compile one extension directory, reusing `cache` when its tree is unchanged.
pub fn compile(
    directory: &Path,
    entry: &str,
    api_version: u64,
    svelte: &SvelteCompiler,
    cache: Option<&Path>,
) -> Result<ModuleGraph, String> {
    let root = fs::canonicalize(directory).map_err(|e| e.to_string())?;
    let id = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !manifest::safe_entry(entry) {
        return Err("Entry must be a relative .ts, .js, or .mjs source path".into());
    }
    let hash = digest_tree(&root)?;
    let cached = cache.map(|c| c.join(format!("{}-{entry}.json", &hash[..32]).replace('/', "_")));
    if let Some(graph) = cached
        .as_ref()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .and_then(|v| ModuleGraph::from_json(&v))
    {
        return Ok(graph);
    }
    let mut sources = Sources {
        surface: api_version >= 4,
        root: root.clone(),
        svelte,
        seen: BTreeMap::new(),
        bytes: 0,
        graph: ModuleGraph {
            sources: BTreeMap::new(),
            edges: BTreeMap::new(),
            hash,
        },
    };
    let entry_path = json!(format!("./{entry}"));
    let bundle = if api_version >= 4 {
        format!("export * from {entry_path};")
    } else {
        format!(
            "export {{ default }} from {entry_path}; export * from {entry_path}; export {{ styles as __powermoveStyles, acquire as __powermoveAcquireStyles }} from '{STYLES}';"
        )
    };
    sources
        .link(BUNDLE, bundle, &root)
        .map_err(|e| if id.is_empty() { e } else { truncate(e) })?;
    let total: usize = sources.graph.sources.values().map(String::len).sum();
    if total > manifest::BUNDLE_BYTES {
        return Err(format!(
            "Extension exceeds bundle size limit ({} bytes)",
            manifest::BUNDLE_BYTES
        ));
    }
    if let Some(path) = cached {
        // A lost cache write only costs the next launch a recompile.
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let temporary = path.with_extension("tmp");
        if fs::write(&temporary, sources.graph.to_json().to_string()).is_ok() {
            let _ = fs::rename(&temporary, &path);
        }
    }
    Ok(sources.graph)
}
fn truncate(error: String) -> String {
    error.chars().take(manifest::ERROR_CHARS).collect()
}
