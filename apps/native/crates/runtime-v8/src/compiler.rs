//! Node-free Svelte front end. Import resolution/runtime singleton binding remains
//! the module loader's job; this preserves the compiler output and diagnostics.
use crate::{CacheKey, CodeCache, Error, Realm, RealmKind, Result, RuntimePool, Ticket};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

pub const SVELTE_VERSION: &str = "5.56.10";
#[derive(Debug, Clone, Copy)]
pub enum SvelteUnit {
    Component,
    Module,
}
impl SvelteUnit {
    fn name(self) -> &'static str {
        match self {
            Self::Component => "component",
            Self::Module => "module",
        }
    }
}
#[derive(Debug, Clone)]
pub struct SvelteOutput {
    /// Raw ESM including source maps and warnings. Never executes extension code.
    pub compiled: Value,
    /// CSS acquisition matches v1.2.0's powermove:styles module contract.
    pub javascript: String,
    pub cache_hit: bool,
}
#[derive(Clone)]
pub struct SvelteCompiler {
    realm: Realm,
    cache: CodeCache,
    max_source: usize,
}
impl SvelteCompiler {
    pub fn new(pool: &RuntimePool, cache_bytes: usize, max_source: usize) -> Result<Self> {
        Ok(Self {
            realm: pool.realm(RealmKind::Compiler)?,
            cache: CodeCache::new(cache_bytes),
            max_source,
        })
    }
    /// TypeScript modules are parsed/transformed natively on the compiler owner;
    /// components use Svelte's own TypeScript support. Neither path executes source.
    pub fn compile(&self, source: &str, filename: &str, unit: SvelteUnit) -> Result<SvelteTicket> {
        if source.len() > self.max_source || filename.len() > 4096 {
            return Err(Error::Quota("Svelte source bytes"));
        }
        let identity = json!([
            filename,
            unit.name(),
            "client",
            "external",
            true,
            "oxc-0.152.0-es2022"
        ])
        .to_string();
        let key = CacheKey::new(
            SVELTE_VERSION,
            crate::vm::SVELTE.as_bytes(),
            source.as_bytes(),
            &identity,
        );
        let hit = self
            .cache
            .get(key)
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|e| Error::Host(e.to_string())));
        let pending = if hit.is_none() {
            Some(
                self.realm
                    .eval_svelte(source.into(), filename.into(), unit)?,
            )
        } else {
            None
        };
        Ok(SvelteTicket {
            pending,
            hit,
            key,
            cache: self.cache.clone(),
        })
    }
    pub fn realm(&self) -> &Realm {
        &self.realm
    }
}
pub struct SvelteTicket {
    pending: Option<Ticket<crate::Evaluation>>,
    hit: Option<Result<Value>>,
    key: CacheKey,
    cache: CodeCache,
}
impl SvelteTicket {
    fn output(&self, compiled: Value, hit: bool) -> Result<SvelteOutput> {
        if !compiled["diagnostic"].is_null() {
            return Err(Error::JavaScript(compiled["diagnostic"].to_string()));
        }
        let mut javascript = compiled["js"]["code"]
            .as_str()
            .ok_or(Error::Invalid("Missing Svelte JavaScript"))?
            .to_owned();
        if let Some(css) = compiled["css"]["code"].as_str().filter(|s| !s.is_empty()) {
            javascript.push_str(&format!(
                "\nimport {{ add as __powermoveCss }} from 'powermove:styles'; __powermoveCss({});",
                json!(css)
            ));
        }
        if !hit {
            self.cache.insert(
                self.key,
                serde_json::to_vec(&compiled).map_err(|e| Error::Host(e.to_string()))?,
            );
        }
        Ok(SvelteOutput {
            compiled,
            javascript,
            cache_hit: hit,
        })
    }
    pub fn try_take(&mut self) -> Option<Result<SvelteOutput>> {
        if let Some(hit) = self.hit.take() {
            return Some(hit.and_then(|v| self.output(v, true)));
        }
        let result = self.pending.as_ref()?.try_take()?;
        self.pending = None;
        Some(
            result
                .and_then(|v| {
                    serde_json::from_str(&v.value).map_err(|e| Error::Host(e.to_string()))
                })
                .and_then(|v| self.output(v, false)),
        )
    }
}
impl Future for SvelteTicket {
    type Output = Result<SvelteOutput>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(hit) = self.hit.take() {
            return Poll::Ready(hit.and_then(|v| self.output(v, true)));
        }
        let Some(ticket) = self.pending.as_mut() else {
            return Poll::Ready(Err(Error::Closed));
        };
        match Pin::new(ticket).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.pending = None;
                Poll::Ready(
                    result
                        .and_then(|v| {
                            serde_json::from_str(&v.value).map_err(|e| Error::Host(e.to_string()))
                        })
                        .and_then(|v| self.output(v, false)),
                )
            }
        }
    }
}

/// Runs on the compiler owner, never on the UI caller.
pub(crate) fn compile_expression(source: &str, filename: &str, unit: SvelteUnit) -> Result<String> {
    let source = if matches!(unit, SvelteUnit::Module) && filename.ends_with(".ts") {
        strip_typescript(source, filename)?
    } else {
        source.to_owned()
    };
    let options = match unit {
        SvelteUnit::Component => {
            json!({"filename":filename,"generate":"client","css":"external","runes":true})
        }
        SvelteUnit::Module => json!({"filename":filename,"generate":"client","runes":true}),
    };
    let method = match unit {
        SvelteUnit::Component => "compile",
        SvelteUnit::Module => "compileModule",
    };
    Ok(format!(
        "(()=>{{try{{return JSON.stringify(svelte.{method}({}, {options}))}}catch(e){{return JSON.stringify({{diagnostic:{{name:e.name,code:e.code,message:e.message,filename:e.filename,start:e.start,end:e.end,frame:e.frame}}}})}}}})()",
        json!(source)
    ))
}
/// One line per diagnostic, as an editor shows it: `file:line:column: message`.
/// A macro, since the diagnostic type lives in a crate this one does not name.
macro_rules! located {
    ($filename:expr, $source:expr, $diagnostics:expr) => {
        $diagnostics
            .iter()
            .map(|d| {
                let at = d
                    .labels
                    .iter()
                    .find(|l| l.primary())
                    .or(d.labels.first())
                    .map(|l| line_column($source, l.offset()))
                    .unwrap_or_default();
                let help = d
                    .help
                    .as_ref()
                    .map(|h| format!(" ({h})"))
                    .unwrap_or_default();
                format!("{}{at}: {}{help}", $filename, d.message)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
}
fn line_column(source: &str, offset: u32) -> String {
    let before = &source[..(offset as usize).min(source.len())];
    let line = before.matches('\n').count() + 1;
    let column = before[before.rfind('\n').map_or(0, |i| i + 1)..]
        .chars()
        .count()
        + 1;
    format!(":{line}:{column}")
}
/// Runtime imports of one JavaScript module, in the order the graph must load
/// them. Type-only imports were already removed by `strip_typescript`.
pub fn module_imports(source: &str) -> Result<Vec<String>> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::mjs()).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(Error::JavaScript(located!(
            "compiled module",
            source,
            &parsed.diagnostics
        )));
    }
    let mut imports: Vec<String> = parsed
        .module_record
        .requested_modules
        .keys()
        .map(|name| name.as_str().to_owned())
        .collect();
    imports.sort();
    for dynamic in &parsed.module_record.dynamic_imports {
        let request =
            &source[dynamic.module_request.start as usize..dynamic.module_request.end as usize];
        // A computed specifier stays a runtime refusal, as the frozen compiler leaves it.
        let literal = request
            .strip_prefix(['"', '\''])
            .and_then(|rest| rest.strip_suffix(['"', '\'']))
            .filter(|text| !text.contains(['\\', '"', '\'']));
        if let Some(name) = literal.filter(|name| !imports.iter().any(|i| i == name)) {
            imports.push(name.to_owned());
        }
    }
    Ok(imports)
}

/// TypeScript to ES2022 module text, with the native parser. No source executes.
pub fn strip_typescript(source: &str, filename: &str) -> Result<String> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_transformer::{TransformOptions, Transformer};
    let allocator = Allocator::default();
    let jsx = filename.ends_with(".tsx");
    let kind = if jsx {
        SourceType::tsx()
    } else {
        SourceType::ts()
    };
    let parsed = Parser::new(&allocator, source, kind).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(Error::JavaScript(located!(
            filename,
            source,
            &parsed.diagnostics
        )));
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .with_enum_eval(true)
        .build(&program);
    if !semantic.diagnostics.is_empty() {
        return Err(Error::JavaScript(located!(
            filename,
            source,
            &semantic.diagnostics
        )));
    }
    let mut options =
        TransformOptions::from_target("es2022").map_err(|e| Error::JavaScript(e.to_string()))?;
    if jsx {
        // v4 surfaces: the published `powermove` JSX aliases.
        options.jsx.runtime = oxc_transformer::JsxRuntime::Automatic;
        options.jsx.import_source = Some("powermove".into());
    }
    let transformed = Transformer::new(&allocator, std::path::Path::new(filename), &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    if !transformed.diagnostics.is_empty() {
        return Err(Error::JavaScript(located!(
            filename,
            source,
            &transformed.diagnostics
        )));
    }
    Ok(oxc_codegen::Codegen::new().build(&program).code)
}
