//! Behaviour oracle: replays the frozen-reference golden traces
//! (`tools/trace-recorder/golden`, v1.2.0) against the kernel's API layer.
//!
//! Stimuli come from the traces: each root `api.*` call (and the `api.*`
//! calls nested inside `gesture.*` adapters) becomes a kernel [`Call`];
//! callback side effects are recovered from the recorded states (the
//! project before the call vs the project when the callback finished, i.e.
//! at the nested `history.commit`/`rollback`/`cancel`), and raw writes
//! between calls become direct writes. For every replayed call we compare
//! the **return form** (value kind, booleans, `EditResult.ok/message`,
//! throw vs return, throw message) and the **post-state** (pending depth,
//! undo index, entry count, project name, layer count).
//!
//! Every difference must be listed in [`APPROVED`] with its reason; the
//! test fails on any other mismatch. The keymap and menu traces are §3.2.3
//! (the shell's resolver), not the kernel's, and are reported as skipped.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value as J;

use pm_kernel::commands::layers::{AddLayer, LayerCommand};
use pm_kernel::{
    Call, Caller, Command, DocumentStore, EditCall, HistoryCall, KernelError, Origin, Project,
    RealmMessage, Reply, Request, WorkspaceCall, WriteBody,
};
use pm_model::Op;
use pm_model::patch::{Patch, PathPart};

/// Differences approved as deliberate (trace, call index, what, why).
const APPROVED: &[(&str, &str, &str)] = &[(
    "edit-admission",
    "api.edit.apply#2",
    "message: the kernel reports its own revision counter (starts at 1), \
     the tag reported PM.proj.revision (0); same refusal and form",
)];

const SCENARIOS: &[&str] = &[
    "history-nesting",
    "history-cancel",
    "edit-cancel",
    "edit-admission",
    "external-history",
    "gesture-modes",
    "workspace-direct",
    "history-async",
];
const OUT_OF_SCOPE: &[&str] = &["keymap-fields", "keymap-order", "menu-equivalents"];

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tools/trace-recorder/golden")
        .join(format!("{name}.jsonl"))
}

/// Structural trace encoding → plain JSON (tokens become stable strings).
fn plain(v: &J) -> J {
    match v {
        J::Object(m) => {
            if let Some(t) = m.get("$token").and_then(J::as_str) {
                return J::String(format!("tok-{t}"));
            }
            if m.contains_key("$volatile") {
                return J::from(0);
            }
            match m.get("$type").and_then(J::as_str) {
                Some("object") => J::Object(
                    m["props"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.trim_start_matches('@').to_owned(), plain(v)))
                        .collect(),
                ),
                Some("array") => {
                    J::Array(m["values"].as_array().unwrap().iter().map(plain).collect())
                }
                Some("json-string") => plain(&m["value"]),
                Some("undefined") => J::Null,
                Some("function") => J::String("<fn>".into()),
                Some(_) => J::Null,
                None => J::Object(m.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
            }
        }
        J::Array(a) => J::Array(a.iter().map(plain).collect()),
        other => other.clone(),
    }
}

#[derive(Clone, Debug)]
struct State {
    project: J,
    depth: u64,
    index: i64,
    entries: usize,
}

fn state(v: &J) -> Option<State> {
    let s = plain(v);
    let h = s.get("history")?;
    Some(State {
        project: s.get("project")?.clone(),
        depth: h.get("depth")?.as_u64()?,
        index: h.get("index")?.as_i64()?,
        entries: h.get("entries")?.as_array()?.len(),
    })
}

#[derive(Debug)]
struct CallNode {
    name: String,
    args: J,
    pre: Option<State>,
    post: Option<State>,
    /// `Some(value)` for a return, `None` + `error` for a throw.
    returned: Option<J>,
    error: Option<String>,
    children: Vec<usize>,
    /// `callback.external` phases observed inside.
    external_phases: Vec<String>,
}

struct Trace {
    calls: Vec<CallNode>,
    roots: Vec<usize>,
    initial: State,
}

fn load(name: &str) -> Trace {
    let text = std::fs::read_to_string(golden(name)).unwrap();
    let rows: Vec<J> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.last().unwrap()["type"], "end");
    assert_eq!(rows.last().unwrap()["valid"], true);
    let mut calls: Vec<CallNode> = Vec::new();
    let mut by_id: HashMap<u64, usize> = HashMap::new();
    let mut roots = Vec::new();
    let mut initial = None;
    for r in &rows {
        match r["type"].as_str().unwrap() {
            "state" if initial.is_none() => {
                initial = state(&r["data"]["props"]["state"]);
            }
            "call" => {
                let idx = calls.len();
                by_id.insert(r["call"].as_u64().unwrap(), idx);
                calls.push(CallNode {
                    name: r["name"].as_str().unwrap().to_owned(),
                    args: plain(&r["args"]),
                    pre: state(&r["state"]),
                    post: None,
                    returned: None,
                    error: None,
                    children: Vec::new(),
                    external_phases: Vec::new(),
                });
                match r["parent"].as_u64() {
                    None => roots.push(idx),
                    Some(p) => {
                        let p = by_id[&p];
                        calls[p].children.push(idx);
                    }
                }
            }
            "return" => {
                let i = by_id[&r["call"].as_u64().unwrap()];
                calls[i].returned = Some(r["value"].clone());
                calls[i].post = state(&r["state"]);
            }
            "throw" => {
                let i = by_id[&r["call"].as_u64().unwrap()];
                calls[i].error = Some(
                    r["error"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                );
                calls[i].post = state(&r["state"]);
            }
            "callback.external" => {
                let phase = plain(&r["data"])["phase"].as_str().unwrap().to_owned();
                if let Some(p) = r["parent"].as_u64() {
                    let mut i = by_id[&p];
                    // Attribute to the root call.
                    while let Some(parent) =
                        (0..calls.len()).find(|&c| calls[c].children.contains(&i))
                    {
                        i = parent;
                    }
                    calls[i].external_phases.push(phase);
                }
            }
            _ => {}
        }
    }
    Trace {
        calls,
        roots,
        initial: initial.expect("initial state"),
    }
}

/// Root-level field writes between two project values (names, flags: the
/// raw writes these traces make). Layers/comps change only via commands.
fn field_writes(before: &J, after: &J) -> Vec<Op> {
    let (Some(b), Some(a)) = (before.as_object(), after.as_object()) else {
        return Vec::new();
    };
    let mut ops = Vec::new();
    for (k, v) in a {
        if matches!(k.as_str(), "layers" | "comps" | "revision" | "edits") {
            continue;
        }
        if b.get(k) != Some(v) {
            let value = pm_model::json::parse(&v.to_string()).unwrap();
            ops.push(Op::Leaf(Patch::set(
                vec![PathPart::from(k.as_str())],
                value,
            )));
        }
    }
    ops
}

fn layer_count(p: &J) -> usize {
    p.get("layers").and_then(J::as_array).map_or(0, Vec::len)
}

fn command(v: &J) -> Option<Command> {
    (v.get("type")?.as_str()? == "add_layer").then(|| {
        LayerCommand::Add(Box::new(AddLayer {
            id: v
                .get("id")
                .and_then(J::as_str)
                .map_or_else(|| pm_kernel::new_id("L"), str::to_owned),
            layer_type: v
                .get("layerType")
                .and_then(J::as_str)
                .unwrap_or("shape")
                .to_owned(),
            comp: None,
            name: v.get("name").and_then(J::as_str).map(str::to_owned),
            from: None,
            duration: None,
            content: Default::default(),
            properties: Default::default(),
            color: None,
            index: None,
            parent: None,
            blend: None,
            motion_blur: None,
            visible: None,
            shy: None,
            collapsed: None,
            time: 0.0,
        }))
        .into()
    })
}

fn body(v: &J) -> WriteBody {
    match v {
        J::Array(items) => WriteBody::Commands(items.iter().filter_map(command).collect()),
        other => WriteBody::Commands(command(other).into_iter().collect()),
    }
}

struct Replayer {
    project: Project,
    page: Caller,
    handles: u64,
    report: Vec<String>,
    mismatches: Vec<String>,
    replayed: usize,
    matched: usize,
}

impl Replayer {
    fn run(&mut self, call: impl Into<Call>) -> pm_kernel::Ack {
        let handled = self.project.handle(Request::new(self.page, call));
        handled.reply.expect("immediate")
    }

    fn kernel_state(&self) -> (u64, i64, usize, String, usize) {
        let store = self.project.store();
        let doc = store.snapshot().document.to_value().unwrap();
        let comp = doc
            .get("compId")
            .and_then(|c| c.as_str())
            .map(str::to_owned);
        let layers = comp
            .and_then(|c| {
                doc.get("comps")?
                    .get(&c)?
                    .get("layers")?
                    .as_array()
                    .map(Vec::len)
            })
            .unwrap_or(0);
        (
            self.project.open_entry().map_or(0, |(_, _, d)| d as u64),
            store.history().applied() as i64 - 1,
            store.history().len(),
            doc.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_owned(),
            layers,
        )
    }

    fn check(
        &mut self,
        scenario: &str,
        key: &str,
        form_ok: Result<(), String>,
        post: &Option<State>,
    ) {
        self.replayed += 1;
        let mut problems = Vec::new();
        if let Err(e) = form_ok {
            problems.push(e);
        }
        if let Some(post) = post {
            let (depth, index, entries, name, layers) = self.kernel_state();
            let want_name = post
                .project
                .get("name")
                .and_then(J::as_str)
                .unwrap_or_default();
            let want = (
                post.depth,
                post.index,
                post.entries,
                want_name.to_owned(),
                layer_count(&post.project),
            );
            let got = (depth, index, entries, name, layers);
            if got != want {
                problems.push(format!(
                    "state (depth, index, entries, name, layers): reference {want:?}, kernel {got:?}"
                ));
            }
        }
        if problems.is_empty() {
            self.matched += 1;
            self.report.push(format!("  match     {key}"));
            return;
        }
        let approved = APPROVED
            .iter()
            .find(|(s, k, _)| *s == scenario && *k == key)
            .map(|(_, _, why)| *why);
        match approved {
            Some(why) => self.report.push(format!(
                "  approved  {key}: {} ({why})",
                problems.join("; ")
            )),
            None => {
                self.report
                    .push(format!("  MISMATCH  {key}: {}", problems.join("; ")));
                self.mismatches
                    .push(format!("{scenario} {key}: {}", problems.join("; ")));
            }
        }
    }
}

fn kind(v: &Option<J>) -> &'static str {
    match v {
        None => "throw",
        Some(J::Object(m)) => match m.get("$type").and_then(J::as_str) {
            Some("undefined") => "undefined",
            Some("json-string") => "json-string",
            _ if m.contains_key("$token") => "token",
            _ => "object",
        },
        Some(J::Null) => "null",
        Some(J::Bool(_)) => "bool",
        Some(_) => "value",
    }
}

fn expect(cond: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what()) }
}

/// The callback's writes: project before the call vs project at the first
/// nested commit/rollback/cancel (when the callback had finished).
fn callback_writes(t: &Trace, i: usize) -> Vec<Op> {
    fn find(t: &Trace, i: usize) -> Option<&State> {
        for &c in &t.calls[i].children {
            let n = &t.calls[c].name;
            if n == "history.commit" || n == "history.rollback" || n == "history.cancel" {
                return t.calls[c].pre.as_ref();
            }
            if let Some(s) = find(t, c) {
                return Some(s);
            }
        }
        None
    }
    match (t.calls[i].pre.as_ref(), find(t, i)) {
        (Some(pre), Some(done)) => field_writes(&pre.project, &done.project),
        _ => Vec::new(),
    }
}

fn replay_api(
    r: &mut Replayer,
    scenario: &str,
    t: &Trace,
    i: usize,
    key: &str,
    post: &Option<State>,
) {
    let c = &t.calls[i];
    let args = c.args.as_array().cloned().unwrap_or_default();
    let label = args
        .first()
        .and_then(J::as_str)
        .unwrap_or_default()
        .to_owned();
    let want = kind(&c.returned);
    let ret = c.returned.as_ref().map(plain);
    let form: Result<(), String> = match c.name.as_str() {
        "api.history.begin" => {
            let ack = r.run(HistoryCall::Begin { label });
            let got = match ack.reply {
                Reply::Before(Some(_)) => "json-string",
                Reply::Before(None) => "null",
                _ => "other",
            };
            expect(got == want, || format!("returns {got}, reference {want}"))
        }
        "api.history.commit" => {
            let ack = r.run(HistoryCall::Commit { label: None });
            let got = matches!(ack.reply, Reply::Bool(true));
            expect(ret == Some(J::Bool(got)), || {
                format!("returns {got}, reference {ret:?}")
            })
        }
        "api.history.cancel" => {
            let ack = r.run(HistoryCall::Cancel);
            expect(
                matches!(ack.reply, Reply::Unit) && want == "undefined",
                || "form".into(),
            )
        }
        "api.history.do" => {
            // A realm-run callback: begin, its writes, then commit or rollback.
            r.run(HistoryCall::Begin {
                label: label.clone(),
            });
            let ops = callback_writes(t, i);
            if !ops.is_empty() {
                r.run(Call::DirectWrite { ops });
            }
            if c.error.is_some() {
                r.run(HistoryCall::Rollback);
                expect(want == "throw", || "the reference returned".into())
            } else {
                r.run(HistoryCall::Commit { label: None });
                expect(want != "throw", || "the reference threw".into())
            }
        }
        "api.history.undo" | "api.history.redo" => {
            let call = if c.name.ends_with("undo") {
                HistoryCall::Undo
            } else {
                HistoryCall::Redo
            };
            let ack = r.run(call);
            let result = match ack.reply {
                Reply::Continuation(k) => {
                    // The page runs the closure inline; the trace says whether it threw.
                    let ok = c.error.is_none();
                    let done = r.run(Call::ExternalDone {
                        ticket: k.ticket,
                        ok,
                        message: c.error.clone(),
                    });
                    done.reply
                }
                other => other,
            };
            match (result, &c.error) {
                (Reply::Bool(b), None) => {
                    expect(ret == Some(J::Bool(b)), || format!("returns {b}"))
                }
                (Reply::Throw(KernelError::ExternalFailed(m)), Some(e)) => {
                    expect(&m == e, || format!("throws {m:?}, reference {e:?}"))
                }
                (other, _) => Err(format!("returns {other:?}, reference {want}")),
            }
        }
        "api.history.external" => {
            let is_fn = |v: Option<&J>| v.and_then(J::as_str) == Some("<fn>");
            if !(is_fn(args.get(1)) && is_fn(args.get(2))) {
                // The facade refuses non-function closures before the kernel.
                expect(want == "null", || "the reference accepted".into())
            } else {
                r.handles += 1;
                let cleanup = is_fn(args.get(3).and_then(|o| o.get("cleanup")));
                let ack = r.run(HistoryCall::External {
                    label,
                    handle: r.handles,
                    bytes: 0,
                    cleanup,
                });
                expect(
                    matches!(ack.reply, Reply::Entry(Some(_))) && want == "token",
                    || "form".into(),
                )
            }
        }
        "history.clear" => {
            r.run(HistoryCall::Clear);
            let cleanups = r
                .project
                .take_outbox()
                .iter()
                .filter(|m| matches!(m, RealmMessage::Cleanup { .. }))
                .count();
            let want_cleanups = c.external_phases.iter().filter(|p| *p == "cleanup").count();
            expect(cleanups == want_cleanups, || {
                format!("{cleanups} cleanups, reference {want_cleanups}")
            })
        }
        "api.edit.begin" => {
            let ack = r.run(EditCall::Begin { label });
            match (&ack.reply, &c.error) {
                (Reply::Unit, None) => Ok(()),
                (Reply::Throw(e), Some(m)) => expect(&e.to_string() == m, || {
                    format!("throws {e}, reference {m:?}")
                }),
                (other, _) => Err(format!("returns {other:?}, reference {want}")),
            }
        }
        "api.edit.cancel" => {
            let ack = r.run(EditCall::Cancel);
            let got = matches!(ack.reply, Reply::Bool(true));
            expect(ret == Some(J::Bool(got)), || {
                format!("returns {got}, reference {ret:?}")
            })
        }
        "api.edit.mutate" | "api.edit.dispatch" | "api.edit.commit" | "api.edit.apply" => {
            let reply =
                if c.name == "api.edit.mutate" && args.get(1).and_then(J::as_str) != Some("<fn>") {
                    // Not a function: the facade's `A structural edit action is required`.
                    None
                } else {
                    let call = match c.name.as_str() {
                        "api.edit.mutate" => EditCall::Mutate {
                            label,
                            ops: callback_writes(t, i),
                        },
                        "api.edit.dispatch" => EditCall::Dispatch {
                            body: body(&args[0]),
                        },
                        "api.edit.commit" => EditCall::Commit { label: None },
                        _ => EditCall::Apply {
                            body: body(&args[0]),
                            base_revision: args
                                .get(1)
                                .and_then(|m| m.get("baseRevision"))
                                .and_then(J::as_u64),
                            label: None,
                        },
                    };
                    Some(r.run(call).reply)
                };
            let want_ok = ret.as_ref().and_then(|v| v.get("ok")).and_then(J::as_bool);
            let want_msg = ret
                .as_ref()
                .and_then(|v| v.get("message"))
                .and_then(J::as_str);
            match reply {
                None => expect(
                    want_ok == Some(false)
                        && want_msg == Some("A structural edit action is required"),
                    || "facade refusal".into(),
                ),
                Some(Reply::Edit(res)) => expect(
                    Some(res.ok) == want_ok && res.message.as_deref() == want_msg,
                    || {
                        format!(
                            "EditResult ok={} {:?}, reference ok={want_ok:?} {want_msg:?}",
                            res.ok, res.message
                        )
                    },
                ),
                Some(other) => Err(format!("returns {other:?}")),
            }
        }
        "api.workspace.mutate" => {
            let next = serde_json::to_string(ret.as_ref().unwrap_or(&J::Null)).unwrap();
            let ack = r.run(WorkspaceCall::Mutate {
                next: pm_model::json::parse(&next).unwrap(),
                threw: c.error.clone(),
            });
            match (&ack.reply, &c.error) {
                (Reply::Workspace(_), None) => Ok(()),
                (Reply::Throw(KernelError::Callback(m)), Some(e)) => {
                    expect(m == e, || "message".into())
                }
                (other, _) => Err(format!("returns {other:?}")),
            }
        }
        "PM.proj.direct" => {
            let ops = field_writes(
                &c.pre.as_ref().unwrap().project,
                &c.post.as_ref().unwrap().project,
            );
            let ack = r.run(Call::DirectWrite { ops });
            expect(
                matches!(ack.reply, Reply::Applied { changed: true, .. }),
                || "refused".into(),
            )
        }
        other => Err(format!("unmapped call {other}")),
    };
    r.check(scenario, key, form, post);
}

fn replay(scenario: &str) -> Replayer {
    let t = load(scenario);
    let project = pm_model::json::parse(&t.initial.project.to_string()).unwrap();
    let mut r = Replayer {
        project: Project::new(DocumentStore::new_untitled(project)),
        page: Caller::realm(1, 1, Origin::Extension),
        handles: 0,
        report: Vec::new(),
        mismatches: Vec::new(),
        replayed: 0,
        matched: 0,
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut previous = t.initial.clone();
    for &root in &t.roots {
        let c = &t.calls[root];
        // Raw writes since the previous call (e.g. `PM.proj.name = 'kept'`).
        if let Some(pre) = &c.pre {
            let ops = field_writes(&previous.project, &pre.project);
            if !ops.is_empty() {
                r.run(Call::DirectWrite { ops });
            }
        }
        if c.name == "PM.touch" {
            // Cache invalidation only: the kernel has no evaluator caches to
            // clear (revisions key every cache).
            r.report
                .push("  n/a       PM.touch (no kernel effect)".into());
            if let Some(post) = &c.post {
                previous = post.clone();
            }
            continue;
        }
        // Adapters (EditGesture) are replayed through the API calls they made.
        let targets: Vec<usize> = if c.name.starts_with("gesture.") {
            c.children
                .iter()
                .copied()
                .filter(|&k| t.calls[k].name.starts_with("api."))
                .collect()
        } else {
            vec![root]
        };
        if targets.is_empty() {
            match c.name.as_str() {
                name if name.starts_with("gesture.") => {
                    // set-mode raw setter or local mode: no API call.
                    let ops = field_writes(
                        &c.pre.as_ref().unwrap().project,
                        &c.post.as_ref().unwrap().project,
                    );
                    if !ops.is_empty() {
                        r.run(Call::DirectWrite { ops });
                    }
                    let n = counts.entry(name.to_owned()).or_default();
                    *n += 1;
                    let key = format!("{name}#{n} (setter only)");
                    r.check(scenario, &key, Ok(()), &c.post);
                }
                other => {
                    let n = counts.entry(other.to_owned()).or_default();
                    *n += 1;
                    let key = format!("{other}#{n}");
                    replay_api(&mut r, scenario, &t, root, &key, &c.post);
                }
            }
        }
        for k in targets {
            let name = t.calls[k].name.clone();
            let n = counts.entry(name.clone()).or_default();
            *n += 1;
            let key = if k == root {
                format!("{name}#{n}")
            } else {
                format!("{name}#{n} (via {})", c.name)
            };
            // Post-state: the adapter's own return state.
            replay_api(&mut r, scenario, &t, k, &key, &c.post);
        }
        if let Some(post) = &c.post {
            previous = post.clone();
        }
    }
    r
}

#[test]
fn golden_traces_replay_against_the_kernel() {
    let mut total = 0;
    let mut matched = 0;
    let mut mismatches = Vec::new();
    let mut out = String::from("trace replay (v1.2.0 goldens → pm-kernel API layer)\n");
    for scenario in SCENARIOS {
        let r = replay(scenario);
        total += r.replayed;
        matched += r.matched;
        out.push_str(&format!("{scenario}: {}/{} match\n", r.matched, r.replayed));
        for line in &r.report {
            out.push_str(line);
            out.push('\n');
        }
        mismatches.extend(r.mismatches);
    }
    for s in OUT_OF_SCOPE {
        out.push_str(&format!(
            "{s}: skipped (keyboard/menu resolver, §3.2.3; not the kernel)\n"
        ));
    }
    out.push_str(&format!(
        "total: {matched}/{total} match, {} unapproved mismatches\n",
        mismatches.len()
    ));
    eprintln!("{out}");
    assert!(
        mismatches.is_empty(),
        "unapproved mismatches:\n{}",
        mismatches.join("\n")
    );
}
