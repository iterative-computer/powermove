//! Svelte's compiler without Node: its prebuilt UMD build (`svelte/compiler`'s
//! `compiler/index.js`) evaluated once in an embedded QuickJS engine.
//!
//! QuickJS contexts cannot move between threads, so one worker thread owns
//! the engine and serves compile jobs for the life of the process.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context as _, Result, anyhow};
use parking_lot::Mutex;
use rquickjs::{Context, Function, Runtime};
use serde::Deserialize;

/// What to compile: a component (`compile`) or a rune module (`compileModule`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Component,
    Module,
}

pub struct Output {
    pub js: String,
    pub css: Option<String>,
}

type Job = (Kind, String, String, mpsc::Sender<Result<Output, String>>);

struct Worker {
    script: PathBuf,
    jobs: mpsc::Sender<Job>,
}

static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// The options compiler.ts passes; `css: 'external'` only applies to components.
const GLUE: &str = r#"(kind, source, filename) => {
  try {
    const options = { generate: 'client', filename, runes: true };
    const out = kind === 'component'
      ? svelte.compile(source, { ...options, css: 'external' })
      : svelte.compileModule(source, options);
    return JSON.stringify({ js: out.js.code, css: out.css ? out.css.code : null });
  } catch (error) {
    return JSON.stringify({ error: error && error.message ? String(error.message) : String(error) });
  }
}"#;

#[derive(Deserialize)]
struct Reply {
    js: Option<String>,
    css: Option<String>,
    error: Option<String>,
}

/// Compile one Svelte source with the compiler at `script`. A Svelte error
/// (bad markup, unknown rune) comes back as `Err` with the compiler's message.
pub fn compile(script: &Path, kind: Kind, source: String, filename: String) -> Result<Result<Output, String>> {
    let (reply, answer) = mpsc::channel();
    {
        let mut worker = WORKER.lock();
        if worker.as_ref().is_none_or(|worker| worker.script != script) {
            *worker = Some(start(script)?);
        }
        let job = (kind, source, filename, reply);
        if let Err(mpsc::SendError(job)) = worker.as_ref().expect("worker started").jobs.send(job) {
            // The engine thread died (a panic in QuickJS); start a fresh one.
            let fresh = start(script)?;
            fresh.jobs.send(job).map_err(|_| anyhow!("Svelte compiler stopped"))?;
            *worker = Some(fresh);
        }
    }
    answer.recv().context("Svelte compiler stopped")
}

fn start(script: &Path) -> Result<Worker> {
    let code = std::fs::read(script).with_context(|| format!("read Svelte compiler {}", script.display()))?;
    let (jobs, queue) = mpsc::channel::<Job>();
    let (ready, started) = mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("svelte-compiler".into())
        // The Svelte parser and printer recurse deeply on large components.
        .stack_size(32 * 1024 * 1024)
        .spawn(move || run(code, queue, ready))
        .context("start Svelte compiler thread")?;
    started.recv().context("Svelte compiler thread stopped")?.map_err(|error| anyhow!("load Svelte compiler: {error}"))?;
    Ok(Worker { script: script.to_path_buf(), jobs })
}

fn run(code: Vec<u8>, queue: mpsc::Receiver<Job>, ready: mpsc::Sender<Result<(), String>>) {
    let setup = || -> Result<(Runtime, Context), String> {
        let runtime = Runtime::new().map_err(|error| error.to_string())?;
        runtime.set_max_stack_size(24 * 1024 * 1024);
        let context = Context::full(&runtime).map_err(|error| error.to_string())?;
        context.with(|ctx| -> Result<(), String> {
            ctx.eval::<(), _>(code).map_err(|error| exception(&ctx, error))?;
            let glue: Function = ctx.eval(GLUE).map_err(|error| exception(&ctx, error))?;
            ctx.globals().set("__powermove_compile", glue).map_err(|error| error.to_string())
        })?;
        Ok((runtime, context))
    };
    let (_runtime, context) = match setup() {
        Ok(engine) => {
            let _ = ready.send(Ok(()));
            engine
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    for (kind, source, filename, reply) in queue {
        let result = context.with(|ctx| -> Result<Output, String> {
            let glue: Function = ctx.globals().get("__powermove_compile").map_err(|error| error.to_string())?;
            let kind = if kind == Kind::Component { "component" } else { "module" };
            let text: String = glue.call((kind, source, filename)).map_err(|error| exception(&ctx, error))?;
            let reply: Reply = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            match (reply.error, reply.js) {
                (Some(error), _) => Err(error),
                (None, Some(js)) => Ok(Output { js, css: reply.css }),
                (None, None) => Err("Svelte produced no JavaScript".into()),
            }
        });
        let _ = reply.send(result);
    }
}

fn exception(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    if error.is_exception() {
        let value = ctx.catch();
        if let Some(exception) = value.as_exception() {
            return exception.message().unwrap_or_else(|| "JavaScript exception".into());
        }
        if let Some(text) = value.as_string().and_then(|text| text.to_string().ok()) {
            return text;
        }
    }
    error.to_string()
}
