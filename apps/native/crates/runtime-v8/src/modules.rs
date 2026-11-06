//! Native ES modules for trusted realms. Extension sources keep the exact
//! ESM semantics (live bindings, cycles, evaluation order, top-level await);
//! the embedder resolves every specifier ahead of time, so nothing here reads
//! files or runs a loader callback in JavaScript.
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Registry {
    urls: HashMap<i32, Vec<String>>,
    modules: HashMap<String, v8::Global<v8::Module>>,
    edges: HashMap<String, HashMap<String, String>>,
    sources: HashMap<String, String>,
}

fn origin<'s>(s: &mut v8::PinScope<'s, '_>, url: &str) -> Option<v8::ScriptOrigin<'s>> {
    let name = v8::String::new(s, url)?;
    Some(v8::ScriptOrigin::new(
        s,
        name.into(),
        0,
        0,
        false,
        -1,
        None,
        false,
        false,
        true,
        None,
    ))
}

fn compile<'s>(s: &mut v8::PinScope<'s, '_>, url: &str) -> Option<v8::Local<'s, v8::Module>> {
    let existing = s
        .get_slot::<Registry>()
        .and_then(|r| r.modules.get(url).cloned());
    if let Some(module) = existing {
        return Some(v8::Local::new(s, module));
    }
    let code = s.get_slot::<Registry>()?.sources.get(url)?.clone();
    let origin = origin(s, url)?;
    let text = v8::String::new(s, &code)?;
    let mut source = v8::script_compiler::Source::new(text, Some(&origin));
    let module = v8::script_compiler::compile_module(s, &mut source)?;
    let global = v8::Global::new(s, module);
    let hash = module.get_identity_hash().get();
    let registry = s.get_slot_mut::<Registry>()?;
    registry.urls.entry(hash).or_default().push(url.to_owned());
    registry.modules.insert(url.to_owned(), global);
    Some(module)
}

fn resolved<'s>(
    s: &mut v8::PinScope<'s, '_>,
    referrer: &str,
    specifier: &str,
) -> Option<v8::Local<'s, v8::Module>> {
    let target = s
        .get_slot::<Registry>()?
        .edges
        .get(referrer)?
        .get(specifier)?
        .clone();
    compile(s, &target)
}

fn resolve<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    _attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe s, context);
    let specifier = specifier.to_rust_string_lossy(s);
    let hash = referrer.get_identity_hash().get();
    let urls = s.get_slot::<Registry>()?.urls.get(&hash)?.clone();
    // Identity hashes can collide. Each candidate's edge table must name the import.
    urls.into_iter().find_map(|url| resolved(s, &url, &specifier))
}

fn dynamic<'s, 'i>(
    s: &mut v8::PinScope<'s, 'i>,
    _options: v8::Local<'s, v8::Data>,
    referrer: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    _attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let referrer = referrer.to_rust_string_lossy(s);
    let requested = specifier.to_rust_string_lossy(s);
    let resolver = v8::PromiseResolver::new(s)?;
    let promise = resolver.get_promise(s);
    let tc = std::pin::pin!(v8::TryCatch::new(s));
    let tc = &mut tc.init();
    let loaded = resolved(tc, &referrer, &requested).and_then(|module| {
        module.instantiate_module(tc, resolve)?;
        let evaluated = module.evaluate(tc)?;
        Some((module, evaluated))
    });
    let Some((module, evaluated)) = loaded else {
        let failure = tc.exception().unwrap_or_else(|| {
            let text = v8::String::new(tc, &format!("Cannot resolve import: {requested}")).unwrap();
            v8::Exception::type_error(tc, text)
        });
        tc.reset();
        resolver.reject(tc, failure);
        return Some(promise);
    };
    let namespace = module.get_module_namespace();
    // Evaluation errors and top-level await reach the import() caller this way.
    let Ok(evaluated) = v8::Local::<v8::Promise>::try_from(evaluated) else {
        resolver.resolve(tc, namespace);
        return Some(promise);
    };
    let namespaced = v8::Function::builder(
        |_: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue| {
            rv.set(a.data());
        },
    )
    .data(namespace)
    .build(tc)?;
    evaluated.then(tc, namespaced)
}

/// `__linkModules(json)`: json is `{sources:{url:code},edges:{url:{spec:url}},entry}`.
/// Returns `[evaluationPromise, namespace]`. Compile and link errors throw.
pub(crate) fn link(
    s: &mut v8::PinScope,
    a: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let text = a.get(0).to_rust_string_lossy(s);
    let graph: Value = match serde_json::from_str(&text) {
        Ok(graph) => graph,
        Err(error) => return super::vm::throw(s, &error.to_string()),
    };
    let Some(entry) = graph["entry"].as_str() else {
        return super::vm::throw(s, "Module graph entry required");
    };
    if s.get_slot::<Registry>().is_none() {
        s.set_slot(Registry::default());
    }
    {
        let registry = s.get_slot_mut::<Registry>().unwrap();
        for (url, code) in graph["sources"].as_object().into_iter().flatten() {
            if let Some(code) = code.as_str() {
                registry.sources.insert(url.clone(), code.to_owned());
            }
        }
        for (url, edges) in graph["edges"].as_object().into_iter().flatten() {
            let edges = edges
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(spec, target)| Some((spec.clone(), target.as_str()?.to_owned())));
            registry
                .edges
                .entry(url.clone())
                .or_default()
                .extend(edges);
        }
    }
    // A failed compile or link has already left its exception pending in `s`.
    let Some(module) = compile(s, entry) else {
        return;
    };
    if module.instantiate_module(s, resolve).is_none() {
        return;
    }
    let Some(evaluated) = module.evaluate(s) else {
        return;
    };
    let namespace = module.get_module_namespace();
    let result = v8::Array::new_with_elements(s, &[evaluated, namespace]);
    rv.set(result.into());
}

pub(crate) fn install(isolate: &mut v8::Isolate) {
    isolate.set_host_import_module_dynamically_callback(dynamic);
}

pub(crate) fn clear(isolate: &mut v8::Isolate) {
    isolate.remove_slot::<Registry>();
}
