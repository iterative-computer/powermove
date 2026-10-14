//! Compile a user extension into one browser ESM bundle without Node.js
//! (port of `src/main/extensions/compiler.ts`).
//!
//! compiler.ts runs esbuild with a plugin that resolves every import itself.
//! esbuild's CLI takes no plugins, so this does the plugin's work up front:
//!
//! 1. Walk the import graph from the entry. Each source is transformed the way
//!    the plugin's `onLoad` does (Svelte through the embedded compiler in
//!    `svelte.rs`, CSS into a `powermove:styles` call) and parsed with oxc to
//!    find its import specifiers.
//! 2. Every specifier is resolved under the same boundary rules
//!    (`resolve_source`) and rewritten to a file in a private stage directory:
//!    sources, `globalThis.__powermove_runtime` shims and the styles module.
//!    A refused specifier becomes a path that does not exist, so its error is
//!    reported only if esbuild actually needs the import (TypeScript drops
//!    unused and type-only imports before resolving, as with the plugin).
//! 3. esbuild.exe bundles the stage. Its metafile must list only staged files.
//!
//! Assets (staged by `scripts/windows/stage-compiler.ps1` into `<resources>/compiler`):
//! `esbuild.exe`, `svelte-compiler.js`, `runtime-exports.json`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration, ImportExpression, ImportOrExportKind,
    TSImportEqualsDeclaration, TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::manifest::{is_extension_id, is_safe_entry, limits, truncate_error};
use super::svelte;

/// A finished bundle: `<out_dir>/<id>/bundle.js` and its SHA-256 prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub bundle_path: PathBuf,
    pub hash: String,
}

const SOURCE_SUFFIXES: [&str; 6] = [".ts", ".js", ".mjs", ".svelte", ".svelte.ts", ".svelte.js"];
const RESERVED_BINDINGS: [&str; 46] = [
    "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "enum", "export", "extends", "false",
    "finally", "for", "function", "if", "implements", "import", "in", "instanceof", "interface", "let", "new", "null", "package", "private", "protected",
    "public", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "var", "void", "while", "with", "yield",
];
const STYLES_MODULE: &str = "export const styles = [];
               const listeners = new Set();
               export function add(css) { styles.push(css); for (const listener of listeners) listener(css); }
               export function acquire(listener) {
                 for (const css of styles) listener(css);
                 listeners.add(listener);
                 return () => listeners.delete(listener);
               }";
const STYLES_FILE: &str = "styles.js";
const BUNDLE_FILE: &str = "bundle.js";
const UNRESOLVED: &str = "__powermove_unresolved_";

#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

/// The staged compiler: esbuild, Svelte's compiler and the runtime export lists.
pub struct Compiler {
    dir: PathBuf,
    runtime: OnceLock<Result<Arc<RuntimeExports>, String>>,
}

#[derive(Deserialize)]
struct RuntimeExports {
    modules: HashMap<String, Vec<String>>,
}

impl Compiler {
    /// `dir` is `<resources_dir>/compiler`. Nothing is read until the first compile.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), runtime: OnceLock::new() }
    }

    fn esbuild(&self) -> Result<PathBuf> {
        let path = self.dir.join("esbuild.exe");
        if !path.is_file() {
            bail!("The mod compiler is missing ({}). Run scripts/windows/stage-compiler.ps1.", path.display());
        }
        Ok(path)
    }

    fn runtime_exports(&self) -> Result<Arc<RuntimeExports>> {
        let loaded = self.runtime.get_or_init(|| {
            let path = self.dir.join("runtime-exports.json");
            std::fs::read(&path)
                .map_err(|error| format!("The mod compiler is missing {}: {error}", path.display()))
                .and_then(|bytes| serde_json::from_slice::<RuntimeExports>(&bytes).map_err(|error| error.to_string()))
                .map(Arc::new)
        });
        loaded.clone().map_err(|error| anyhow!(error))
    }

    /// Compile `dir` (whose folder name is the extension id) into
    /// `<out_dir>/<id>/bundle.js`. Errors are user-facing and truncated like
    /// compiler.ts. Blocking: call from a blocking thread.
    pub fn compile(&self, dir: &Path, entry: &str, out_dir: &Path) -> Result<Compiled, String> {
        self.try_compile(dir, entry, out_dir).map_err(|error| truncate_error(&format!("{error:#}")))
    }

    fn try_compile(&self, dir: &Path, entry: &str, out_dir: &Path) -> Result<Compiled> {
        let id = std::path::absolute(dir)?.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_extension_id(&id) {
            bail!("Invalid extension id: {id}");
        }
        if !is_safe_entry(entry) {
            bail!("Entry must be a relative .ts, .js, or .mjs source path");
        }
        let root = dunce::canonicalize(dir).with_context(|| format!("Extension folder not found: {}", dir.display()))?;
        std::fs::create_dir_all(out_dir)?;
        let stage = Stage::create(out_dir)?;
        let mut graph = Graph::new(self, root, stage.path.clone());
        graph.run(entry)?;
        let output = graph.bundle()?;

        let target = out_dir.join(&id);
        let bundle_path = target.join("bundle.js");
        std::fs::create_dir_all(&target)?;
        let temporary = target.join(format!(".bundle-{}.tmp", uuid::Uuid::new_v4()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut file| file.write_all(&output))
            .and_then(|()| std::fs::rename(&temporary, &bundle_path));
        let _ = std::fs::remove_file(&temporary);
        written?;
        Ok(Compiled { bundle_path, hash: hex::encode(&Sha256::digest(&output)[..8]) })
    }
}

/// A scratch directory under the build root, removed on drop.
struct Stage {
    path: PathBuf,
}

impl Stage {
    fn create(out_dir: &Path) -> Result<Self> {
        let path = out_dir.join(format!(".stage-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&path)?;
        // Stop esbuild from reading a package.json or tsconfig.json above the stage.
        std::fs::write(path.join("package.json"), "{}")?;
        std::fs::write(path.join("tsconfig.json"), "{}")?;
        Ok(Self { path })
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Loader {
    Js,
    Ts,
}

struct Graph<'c> {
    compiler: &'c Compiler,
    root: PathBuf,
    stage: PathBuf,
    /// Canonical source path → staged path (relative to the stage, `/`-separated).
    sources: HashMap<PathBuf, String>,
    queue: VecDeque<PathBuf>,
    source_bytes: u64,
    loaded: usize,
    runtime: HashSet<String>,
    /// Errors behind `./__powermove_unresolved_<n>` placeholders.
    unresolved: Vec<String>,
    /// Staged path → the name compiler.ts gives it in the source map.
    names: HashMap<String, String>,
}

impl<'c> Graph<'c> {
    fn new(compiler: &'c Compiler, root: PathBuf, stage: PathBuf) -> Self {
        let mut names = HashMap::new();
        names.insert(STYLES_FILE.to_owned(), "powermove-bundle:powermove:styles".to_owned());
        names.insert(BUNDLE_FILE.to_owned(), "powermove-bundle:powermove:bundle".to_owned());
        Self { compiler, root, stage, sources: HashMap::new(), queue: VecDeque::new(), source_bytes: 0, loaded: 0, runtime: HashSet::new(), unresolved: Vec::new(), names }
    }

    fn run(&mut self, entry: &str) -> Result<()> {
        // `powermove:bundle` re-exports the entry and the styles registry.
        let entry = self.resolve_source(&format!("./{entry}"), &self.root.clone())?;
        let entry = self.enqueue(entry);
        let bundle = format!(
            "export {{ default }} from {target}; export * from {target}; export {{ styles as __powermoveStyles, acquire as __powermoveAcquireStyles }} from './{STYLES_FILE}';",
            target = quote(&format!("./{entry}"))
        );
        self.write(BUNDLE_FILE, &bundle)?;
        self.write(STYLES_FILE, STYLES_MODULE)?;
        while let Some(source) = self.queue.pop_front() {
            self.load(&source)?;
        }
        Ok(())
    }

    fn enqueue(&mut self, canonical: PathBuf) -> String {
        if let Some(staged) = self.sources.get(&canonical) {
            return staged.clone();
        }
        let relative = relative_components(&self.root, &canonical).unwrap_or_default().join("/");
        // Svelte modules are JavaScript once compiled, whatever their source suffix.
        let suffix = if loader_for(&canonical) == Loader::Ts && !canonical.to_string_lossy().ends_with(".svelte.ts") { ".ts" } else { ".js" };
        let staged = format!("s/{relative}{suffix}");
        self.names.insert(staged.clone(), format!("powermove-source:{}", canonical.display()));
        self.sources.insert(canonical.clone(), staged.clone());
        self.queue.push_back(canonical);
        staged
    }

    fn load(&mut self, path: &Path) -> Result<()> {
        let size = std::fs::symlink_metadata(path)?.len();
        let too_large = || anyhow!("Extension exceeds source size limit ({} bytes)", limits::SOURCE_BYTES);
        if size > limits::SOURCE_BYTES {
            return Err(too_large());
        }
        self.source_bytes += size;
        self.loaded += 1;
        if self.loaded > limits::SOURCE_FILES {
            bail!("Extension exceeds source file limit ({})", limits::SOURCE_FILES);
        }
        if self.source_bytes > limits::SOURCE_BYTES {
            return Err(too_large());
        }

        let source = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
        let name = path.to_string_lossy();
        let filename = name.clone().into_owned();
        let (code, loader) = if name.ends_with(".css") {
            (format!("import {{ add }} from 'powermove:styles'; add({});", quote(&source)), Loader::Js)
        } else if name.ends_with(".svelte") {
            let compiled = self.svelte(svelte::Kind::Component, source, filename)?;
            let css = match compiled.css.filter(|css| !css.is_empty()) {
                Some(css) => format!("\nimport {{ add as __powermoveCss }} from 'powermove:styles'; __powermoveCss({});", quote(&css)),
                None => String::new(),
            };
            (compiled.js + &css, Loader::Js)
        } else if name.ends_with(".svelte.ts") || name.ends_with(".svelte.js") {
            // compileModule parses JavaScript only: strip a .svelte.ts module's types first.
            let script = if name.ends_with(".ts") { self.strip_types(&source)? } else { source };
            (self.svelte(svelte::Kind::Module, script, filename)?.js, Loader::Js)
        } else {
            (source, loader_for(path))
        };

        let directory = path.parent().unwrap_or(&self.root).to_path_buf();
        let staged = self.sources[path].clone();
        let code = self.rewrite_imports(&code, loader, &directory, &staged)?;
        self.write(&staged, &code)
    }

    fn svelte(&self, kind: svelte::Kind, source: String, filename: String) -> Result<svelte::Output> {
        let script = self.compiler.dir.join("svelte-compiler.js");
        svelte::compile(&script, kind, source, filename)?.map_err(|error| anyhow!(error))
    }

    /// `esbuild.transform(source, { loader: 'ts', target: 'es2022' })`.
    fn strip_types(&self, source: &str) -> Result<String> {
        let mut command = Command::new(self.compiler.esbuild()?);
        command.args(["--loader=ts", "--target=es2022", "--log-level=error", "--color=false"]);
        let output = run_with_input(command, source.as_bytes())?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Point every import specifier in `code` at a staged file.
    fn rewrite_imports(&mut self, code: &str, loader: Loader, directory: &Path, staged: &str) -> Result<String> {
        let specifiers = {
            let allocator = Allocator::default();
            let source_type = if loader == Loader::Ts { SourceType::ts() } else { SourceType::mjs() };
            let parsed = Parser::new(&allocator, code, source_type).parse();
            if parsed.fatal_error || parsed.diagnostics.has_errors() {
                // esbuild parses the file unchanged and reports the syntax error.
                return Ok(code.to_owned());
            }
            let mut collector = Specifiers::default();
            collector.visit_program(&parsed.program);
            collector.found
        };
        let depth = staged.matches('/').count();
        let mut output = String::with_capacity(code.len());
        let mut cursor = 0;
        let mut sorted = specifiers;
        sorted.sort_by_key(|specifier| specifier.start);
        for Specifier { start, end, value } in sorted {
            if start < cursor {
                continue;
            }
            let target = self.target(&value, directory);
            output.push_str(&code[cursor..start]);
            output.push_str(&quote(&format!("{}{target}", if depth == 0 { "./".to_owned() } else { "../".repeat(depth) })));
            cursor = end;
        }
        output.push_str(&code[cursor..]);
        Ok(output)
    }

    /// The staged file (relative to the stage root) an import resolves to.
    fn target(&mut self, specifier: &str, directory: &Path) -> String {
        if specifier == "powermove:styles" {
            return STYLES_FILE.into();
        }
        if specifier == "powermove:bundle" {
            return BUNDLE_FILE.into();
        }
        let resolved = if is_runtime_import(specifier) {
            self.runtime_shim(specifier)
        } else {
            self.resolve_source(specifier, directory).map(|canonical| self.enqueue(canonical))
        };
        resolved.unwrap_or_else(|error| {
            self.unresolved.push(format!("{error:#}"));
            format!("{UNRESOLVED}{}__.js", self.unresolved.len() - 1)
        })
    }

    fn runtime_shim(&mut self, specifier: &str) -> Result<String> {
        let file = format!("rt/{}.js", specifier.replace('/', "~"));
        if self.runtime.insert(specifier.to_owned()) {
            let module = runtime_module(specifier, &*self.compiler.runtime_exports()?)?;
            self.write(&file, &module)?;
            self.names.insert(file.clone(), format!("powermove-runtime:{specifier}"));
        }
        Ok(file)
    }

    /// compiler.ts `resolveSource`: relative imports that stay inside the
    /// extension, avoid hidden folders and `node_modules`, and name a file.
    fn resolve_source(&self, specifier: &str, directory: &Path) -> Result<PathBuf> {
        resolve_source(&self.root, specifier, directory)
    }

    fn write(&self, staged: &str, code: &str) -> Result<()> {
        let path = self.stage.join(staged);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, code)?;
        Ok(())
    }

    /// Run esbuild over the stage and return the bundle bytes.
    fn bundle(&self) -> Result<Vec<u8>> {
        let mut command = Command::new(self.compiler.esbuild()?);
        command.current_dir(&self.stage).args([
            BUNDLE_FILE,
            "--bundle",
            "--format=esm",
            "--platform=browser",
            "--target=es2022",
            "--sourcemap=inline",
            "--log-level=error",
            "--color=false",
            "--metafile=meta.json",
            "--outfile=out.js",
        ]);
        let output = run_with_input(command, &[])?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).into_owned();
            bail!("{}", self.explain(&message));
        }
        self.check_inputs()?;
        let bundle = std::fs::read(self.stage.join("out.js"))?;
        let bundle = self.rename_sources(bundle)?;
        if bundle.len() > limits::BUNDLE_BYTES {
            bail!("Extension exceeds bundle size limit ({} bytes)", limits::BUNDLE_BYTES);
        }
        Ok(bundle)
    }

    /// Prefer the boundary error behind a placeholder esbuild could not
    /// resolve; otherwise show esbuild's message with original file names.
    fn explain(&self, message: &str) -> String {
        if let Some(index) = message.find(UNRESOLVED) {
            let digits: String = message[index + UNRESOLVED.len()..].chars().take_while(char::is_ascii_digit).collect();
            if let Some(error) = digits.parse::<usize>().ok().and_then(|number| self.unresolved.get(number)) {
                return error.clone();
            }
        }
        let mut text = message.trim().replace("✘ [ERROR] ", "");
        for (staged, name) in &self.names {
            if let Some(source) = name.strip_prefix("powermove-source:") {
                let relative = relative_components(&self.root, Path::new(source)).unwrap_or_default().join("/");
                text = text.replace(staged.as_str(), &relative);
            }
        }
        text
    }

    /// The metafile must list only files of the stage: nothing esbuild
    /// resolved on its own may come from outside it.
    fn check_inputs(&self) -> Result<()> {
        let meta: Value = serde_json::from_slice(&std::fs::read(self.stage.join("meta.json"))?)?;
        let inputs = meta.get("inputs").and_then(Value::as_object).context("esbuild wrote no metafile")?;
        for input in inputs.keys() {
            let inside = !input.contains(':') && Path::new(input).components().all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
            if !inside || !self.names.contains_key(input.as_str()) {
                bail!("Import escapes extension directory: {input}");
            }
        }
        Ok(())
    }

    /// Name sources in the inline source map and in esbuild's `// <file>`
    /// banners as compiler.ts's namespaces do (`powermove-source:<path>`,
    /// `powermove-runtime:<specifier>`). Banner lines carry no mappings.
    fn rename_sources(&self, bundle: Vec<u8>) -> Result<Vec<u8>> {
        let text = String::from_utf8(bundle)?;
        let bundle = text
            .split_inclusive('\n')
            .map(|line| {
                let body = line.trim_end_matches(['\r', '\n']);
                match body.strip_prefix("// ").and_then(|file| self.names.get(file)) {
                    Some(name) => format!("// {name}{}", &line[body.len()..]),
                    None => line.to_owned(),
                }
            })
            .collect::<String>()
            .into_bytes();
        const MARKER: &[u8] = b"//# sourceMappingURL=data:application/json;base64,";
        let Some(start) = bundle.windows(MARKER.len()).rposition(|window| window == MARKER) else { return Ok(bundle) };
        let data_start = start + MARKER.len();
        let data_end = bundle[data_start..].iter().position(|byte| byte.is_ascii_whitespace()).map_or(bundle.len(), |offset| data_start + offset);
        let engine = base64::engine::general_purpose::STANDARD;
        let mut map: Value = serde_json::from_slice(&engine.decode(&bundle[data_start..data_end])?)?;
        if let Some(Value::Array(sources)) = map.get_mut("sources") {
            for source in sources.iter_mut() {
                let renamed = source.as_str().and_then(|text| self.names.get(text.trim_start_matches("./")));
                if let Some(name) = renamed {
                    *source = Value::String(name.clone());
                }
            }
        }
        let mut output = bundle[..data_start].to_vec();
        output.extend_from_slice(engine.encode(serde_json::to_vec(&map)?).as_bytes());
        output.extend_from_slice(&bundle[data_end..]);
        Ok(output)
    }
}

/// Specifiers esbuild would resolve, as byte ranges of their string literals.
#[derive(Default)]
struct Specifiers {
    found: Vec<Specifier>,
}

struct Specifier {
    start: usize,
    end: usize,
    value: String,
}

impl Specifiers {
    fn literal(&mut self, expression: &Expression<'_>) {
        let (span, value) = match expression {
            Expression::StringLiteral(literal) => (literal.span, literal.value.to_string()),
            Expression::TemplateLiteral(template) if template.expressions.is_empty() && template.quasis.len() == 1 => {
                let Some(cooked) = template.quasis[0].value.cooked.as_ref() else { return };
                (template.span, cooked.to_string())
            }
            _ => return,
        };
        self.found.push(Specifier { start: span.start as usize, end: span.end as usize, value });
    }

    fn string(&mut self, span: oxc_span::Span, value: &str) {
        self.found.push(Specifier { start: span.start as usize, end: span.end as usize, value: value.to_owned() });
    }
}

impl<'a> Visit<'a> for Specifiers {
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        if declaration.import_kind != ImportOrExportKind::Type {
            self.string(declaration.source.span, &declaration.source.value);
        }
    }

    fn visit_export_from_declaration(&mut self, declaration: &ExportFromDeclaration<'a>) {
        if declaration.export_kind != ImportOrExportKind::Type {
            self.string(declaration.source.span, &declaration.source.value);
        }
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        if declaration.export_kind != ImportOrExportKind::Type {
            self.string(declaration.source.span, &declaration.source.value);
        }
    }

    fn visit_ts_import_equals_declaration(&mut self, declaration: &TSImportEqualsDeclaration<'a>) {
        if let (TSModuleReference::ExternalModuleReference(reference), false) = (&declaration.module_reference, declaration.import_kind == ImportOrExportKind::Type) {
            self.string(reference.expression.span, &reference.expression.value);
        }
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        self.literal(&expression.source);
        walk::walk_import_expression(self, expression);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let is_require = match &call.callee {
            Expression::Identifier(identifier) => identifier.name == "require",
            Expression::StaticMemberExpression(member) => {
                member.property.name == "resolve" && matches!(&member.object, Expression::Identifier(identifier) if identifier.name == "require")
            }
            _ => false,
        };
        if let Some(argument) = call.arguments.first().filter(|_| is_require && call.arguments.len() == 1).and_then(|argument| argument.as_expression()) {
            self.literal(argument);
        }
        walk::walk_call_expression(self, call);
    }
}

fn loader_for(path: &Path) -> Loader {
    let name = path.to_string_lossy();
    if name.ends_with(".ts") { Loader::Ts } else { Loader::Js }
}

fn is_runtime_import(specifier: &str) -> bool {
    specifier == "powermove" || specifier == "svelte" || specifier.starts_with("svelte/")
}

/// The shim compiler.ts's `runtimeModule` generates for one runtime specifier.
fn runtime_module(specifier: &str, exports: &RuntimeExports) -> Result<String> {
    if specifier == "svelte/internal/disclose-version" {
        return Ok("export {}".into());
    }
    let Some(keys) = exports.modules.get(specifier) else {
        bail!("Unsupported Svelte runtime import: {specifier}");
    };
    let reserved = |key: &&String| RESERVED_BINDINGS.contains(&key.as_str());
    let mut lines = vec![format!("const m = globalThis.__powermove_runtime[{}];", quote(specifier)), "export default m;".to_owned()];
    lines.extend(keys.iter().filter(|key| *key != "default" && is_identifier(key) && !reserved(key)).map(|key| format!("export const {key} = m.{key};")));
    lines.extend(
        keys.iter()
            .filter(|key| reserved(key) && *key != "default")
            .enumerate()
            .map(|(index, key)| format!("const __reserved_{index} = m.{key}; export {{ __reserved_{index} as {key} }};")),
    );
    Ok(lines.join("\n"))
}

/// `/^[A-Za-z_$][\w$]*$/`
fn is_identifier(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_alphabetic() || first == b'_' || first == b'$')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$')
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

/// Node's `path.isAbsolute` on Windows: rooted (`\x`, `/x`) or drive-absolute (`C:\x`).
fn is_absolute_specifier(specifier: &str) -> bool {
    let bytes = specifier.as_bytes();
    matches!(bytes.first(), Some(b'/' | b'\\')) || (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'/' | b'\\'))
}

/// `path.resolve(directory, specifier)`: lexical, `..` pops a component.
fn lexical_join(directory: &Path, specifier: &str) -> PathBuf {
    let mut path = directory.to_path_buf();
    for part in specifier.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                path.pop();
            }
            part => path.push(part),
        }
    }
    path
}

/// `path.relative(root, path)` split into components, or `None` when `path`
/// is not inside `root`. Windows paths compare case-insensitively, as Node does.
pub(crate) fn relative_components(root: &Path, path: &Path) -> Option<Vec<String>> {
    let key = |component: Component<'_>| component.as_os_str().to_string_lossy().to_lowercase();
    let mut root_parts = root.components();
    let mut path_parts = path.components();
    loop {
        match (root_parts.next(), path_parts.clone().next()) {
            (None, _) => break,
            (Some(left), Some(right)) if key(left) == key(right) => {
                path_parts.next();
            }
            _ => return None,
        }
    }
    Some(path_parts.map(|component| component.as_os_str().to_string_lossy().into_owned()).collect())
}

fn has_hidden_component(components: &[String]) -> bool {
    components.iter().any(|component| component != "." && component != ".." && component.starts_with('.'))
}

fn has_node_modules_component(components: &[String]) -> bool {
    components.iter().any(|component| component == "node_modules")
}

pub(crate) fn resolve_source(root: &Path, specifier: &str, directory: &Path) -> Result<PathBuf> {
    if is_absolute_specifier(specifier) {
        bail!("Absolute imports are not allowed: {specifier}");
    }
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        bail!("Unsupported bare import: {specifier}");
    }
    let escapes = || anyhow!("Import escapes extension directory: {specifier}");
    let unresolved = lexical_join(directory, specifier);
    let inside = relative_components(root, &unresolved);
    if inside.as_ref().is_some_and(|components| has_hidden_component(components)) {
        bail!("Imports from hidden extension paths are not allowed: {specifier}");
    }
    if inside.as_ref().is_none_or(|components| has_node_modules_component(components)) {
        return Err(escapes());
    }

    let name = unresolved.to_string_lossy().into_owned();
    let has_extension = unresolved
        .file_name()
        .map(|file| file.to_string_lossy().into_owned())
        .is_some_and(|file| file.rfind('.').is_some_and(|dot| dot > 0));
    let candidates: Vec<PathBuf> = if has_extension {
        let mut list = vec![unresolved.clone()];
        if name.ends_with(".svelte") {
            list.push(PathBuf::from(format!("{name}.ts")));
            list.push(PathBuf::from(format!("{name}.js")));
        }
        list
    } else {
        std::iter::once(unresolved.clone())
            .chain(SOURCE_SUFFIXES.iter().map(|suffix| PathBuf::from(format!("{name}{suffix}"))))
            .chain(SOURCE_SUFFIXES.iter().map(|suffix| unresolved.join(format!("index{suffix}"))))
            .collect()
    };

    for candidate in candidates {
        if !std::fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        let Ok(canonical) = dunce::canonicalize(&candidate) else { continue };
        match relative_components(root, &canonical) {
            // A hidden target is skipped like compiler.ts's swallowed error.
            Some(components) if has_hidden_component(&components) => continue,
            Some(components) if !has_node_modules_component(&components) => return Ok(canonical),
            _ => return Err(escapes()),
        }
    }
    bail!("Could not resolve source import: {specifier}")
}

fn run_with_input(mut command: Command, input: &[u8]) -> Result<std::process::Output> {
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(NO_WINDOW);
    }
    let mut child = command.spawn().context("start esbuild")?;
    let mut stdin = child.stdin.take().context("esbuild stdin")?;
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().context("run esbuild")?;
    let _ = writer.join();
    Ok(output)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Instant;

    /// The staged compiler, or `None` (and the test is skipped) when
    /// `scripts/windows/stage-compiler.ps1` has not run.
    pub(crate) fn staged_compiler() -> Option<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/powermove-resources/compiler");
        if dir.join("esbuild.exe").is_file() && dir.join("svelte-compiler.js").is_file() {
            return Some(dir);
        }
        eprintln!("skipped: run scripts/windows/stage-compiler.ps1 to stage the mod compiler");
        None
    }

    pub(crate) fn write(root: &Path, files: &[(&str, &str)]) {
        for (relative, text) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    #[test]
    fn refuses_imports_outside_the_boundary() {
        let root = tempfile::tempdir().unwrap();
        let extension = root.path().join("my-mod");
        write(&extension, &[("index.ts", ""), ("lib/util.ts", ""), (".secret/key.ts", ""), ("node_modules/pkg/index.js", "")]);
        write(root.path(), &[("outside.ts", "")]);
        let extension = dunce::canonicalize(&extension).unwrap();
        let resolve = |specifier: &str| resolve_source(&extension, specifier, &extension).map_err(|error| error.to_string());

        assert_eq!(resolve("./lib/util").unwrap(), extension.join("lib").join("util.ts"));
        assert_eq!(resolve("./lib/../index.ts").unwrap(), extension.join("index.ts"));
        assert_eq!(resolve("../outside").unwrap_err(), "Import escapes extension directory: ../outside");
        assert_eq!(resolve("./lib/../../outside.ts").unwrap_err(), "Import escapes extension directory: ./lib/../../outside.ts");
        assert_eq!(resolve("lodash").unwrap_err(), "Unsupported bare import: lodash");
        assert_eq!(resolve("@scope/pkg").unwrap_err(), "Unsupported bare import: @scope/pkg");
        assert_eq!(resolve("./.secret/key").unwrap_err(), "Imports from hidden extension paths are not allowed: ./.secret/key");
        assert_eq!(resolve("./node_modules/pkg/index.js").unwrap_err(), "Import escapes extension directory: ./node_modules/pkg/index.js");
        assert_eq!(resolve("/etc/passwd").unwrap_err(), "Absolute imports are not allowed: /etc/passwd");
        assert_eq!(resolve(r"C:\Windows\win.ini").unwrap_err(), r"Absolute imports are not allowed: C:\Windows\win.ini");
        assert_eq!(resolve("./missing").unwrap_err(), "Could not resolve source import: ./missing");
    }

    #[test]
    fn generates_runtime_shims_like_compiler_ts() {
        let exports = RuntimeExports { modules: HashMap::from([("svelte".to_owned(), vec!["mount".into(), "default".into(), "if".into(), "bad-name".into()])]) };
        assert_eq!(
            runtime_module("svelte", &exports).unwrap(),
            "const m = globalThis.__powermove_runtime[\"svelte\"];\nexport default m;\nexport const mount = m.mount;\nconst __reserved_0 = m.if; export { __reserved_0 as if };"
        );
        assert_eq!(runtime_module("svelte/internal/disclose-version", &exports).unwrap(), "export {}");
        assert_eq!(runtime_module("svelte/legacy", &exports).unwrap_err().to_string(), "Unsupported Svelte runtime import: svelte/legacy");
    }

    #[test]
    fn refuses_escaping_imports_during_a_build() {
        let Some(compiler_dir) = staged_compiler() else { return };
        let compiler = Compiler::new(compiler_dir);
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("build");
        write(root.path(), &[("outside.ts", "export const leak = 1;")]);
        let cases = [
            ("import { leak } from '../outside'; export default () => leak;", "Import escapes extension directory: ../outside"),
            ("import pad from 'left-pad'; export default pad;", "Unsupported bare import: left-pad"),
            ("import x from './.hidden/x'; export default x;", "Imports from hidden extension paths are not allowed: ./.hidden/x"),
            ("import x from './node_modules/x/index.js'; export default x;", "Import escapes extension directory: ./node_modules/x/index.js"),
            ("export default await import('../outside');", "Import escapes extension directory: ../outside"),
        ];
        for (index, (source, expected)) in cases.iter().enumerate() {
            let extension = root.path().join(format!("case-{index}"));
            write(&extension, &[("index.ts", source), (".hidden/x.ts", "export default 1;"), ("node_modules/x/index.js", "export default 1;")]);
            assert_eq!(compiler.compile(&extension, "index.ts", &out).unwrap_err(), *expected, "{source}");
        }
        // Type-only imports are erased before anything resolves, as with the esbuild plugin.
        let typed = root.path().join("typed");
        write(&typed, &[("index.ts", "import type { Api } from 'powermove'; import type { X } from '../outside'; export default (api: Api): X => api as never;")]);
        compiler.compile(&typed, "index.ts", &out).unwrap();
    }

    #[test]
    fn compiles_the_documented_samples() {
        let Some(compiler_dir) = staged_compiler() else { return };
        let compiler = Compiler::new(compiler_dir);
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/docs/samples");
        let out = tempfile::tempdir().unwrap();
        for (id, shim) in [("media-browser", Some("globalThis.__powermove_runtime[\"svelte/internal/client\"]")), ("gradient-tint", None)] {
            let started = Instant::now();
            let compiled = compiler.compile(&samples.join(id), "index.ts", out.path()).unwrap_or_else(|error| panic!("{id}: {error}"));
            eprintln!("compiled {id} in {:?}", started.elapsed());
            assert_eq!(compiled.bundle_path, out.path().join(id).join("bundle.js"));
            assert_eq!(compiled.hash.len(), 16);
            let bundle = std::fs::read_to_string(&compiled.bundle_path).unwrap();
            assert!(bundle.contains("__powermoveStyles") && bundle.contains("__powermoveAcquireStyles"), "{id}: styles exports");
            assert!(bundle.contains("sourceMappingURL=data:application/json;base64,"), "{id}: inline source map");
            assert!(bundle.contains("// powermove-source:") && !bundle.contains("// s/"), "{id}: banners name the original files");
            assert!(!bundle.contains("from \"svelte") && !bundle.contains("from \"powermove\""), "{id}: runtime imports must be shimmed");
            if let Some(shim) = shim {
                assert!(bundle.contains(shim), "{id}: missing {shim}");
            }
        }
    }

    /// The renderer's runtime-globals fixture reaches every Svelte entry and a `.svelte.ts` rune module.
    #[test]
    fn shims_every_svelte_runtime_entry() {
        let Some(compiler_dir) = staged_compiler() else { return };
        let compiler = Compiler::new(compiler_dir);
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/test/fixtures/svelte-surface");
        let out = tempfile::tempdir().unwrap();
        let compiled = compiler.compile(&fixture, "index.ts", out.path()).unwrap();
        let bundle = std::fs::read_to_string(compiled.bundle_path).unwrap();
        let specifiers = ["svelte", "svelte/animate", "svelte/attachments", "svelte/easing", "svelte/events", "svelte/internal/client", "svelte/motion", "svelte/reactivity", "svelte/reactivity/window", "svelte/store", "svelte/transition"];
        for specifier in specifiers {
            assert!(bundle.contains(&format!("globalThis.__powermove_runtime[{}]", quote(specifier))), "missing {specifier}");
        }
        assert!(!bundle.contains("$state("), "rune module was not compiled");
        assert!(!bundle.lines().any(|line| line.trim_start().starts_with("import ")), "bundle keeps an import");

        // Lazily imported CSS still goes through the styles registry.
        let lazy = out.path().join("src").join("lazy-styles");
        write(&lazy, &[("late.css", ".late { color: green; }"), ("index.ts", "export default () => import('./late.css');")]);
        let bundle = std::fs::read_to_string(compiler.compile(&lazy, "index.ts", out.path()).unwrap().bundle_path).unwrap();
        assert!(bundle.contains(r#"add(".late { color: green; }")"#) && !bundle.contains("document.createElement"));
    }
}
