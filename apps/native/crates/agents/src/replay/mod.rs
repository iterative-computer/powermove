//! P4 test-only replay engine, included by integration tests with `#[path]`.
//! Golden bytes come exclusively from the frozen TypeScript oracle.
use pm_agents::{Access, ApprovalMode, Effects, Event, approvals::Authority, claude, codex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

pub const BASELINE: &str = "07ed5fa17ee640659cb075f9cad1af2095011d60";
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub baseline: String,
    pub commit: String,
    pub synthetic: bool,
    pub sources: Vec<Source>,
    pub recorder: Source,
    pub capture: Source,
    pub dependencies: BTreeMap<String, String>,
    pub cases: Vec<Case>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub file: String,
    pub sha256: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    pub surface: String,
    pub file: String,
    pub sha256: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub kind: String,
    pub value: Value,
    /// Last stimulus observed, starting at one. Detects output that moves
    /// across incoming/action boundaries even when channel order is unchanged.
    #[serde(default)]
    pub after: usize,
}
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observations {
    pub wire: Vec<Message>,
    /// All native events are retained, including lifecycle/inspection additions.
    pub events: Vec<Message>,
    pub lifecycle: Vec<Message>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Difference {
    pub path: String,
    /// Option distinguishes an absent value from an explicit JSON null.
    #[serde(with = "presence")]
    pub expected: Option<Value>,
    #[serde(with = "presence")]
    pub actual: Option<Value>,
}
mod presence {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;
    #[derive(Serialize, Deserialize)]
    enum Presence {
        Missing,
        Present(Value),
    }
    pub fn serialize<S: Serializer>(v: &Option<Value>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            None => Presence::Missing,
            Some(v) => Presence::Present(v.clone()),
        }
        .serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
        Ok(match Presence::deserialize(d)? {
            Presence::Missing => None,
            Presence::Present(v) => Some(v),
        })
    }
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub name: String,
    pub surface: String,
    pub reference_messages: usize,
    pub native_messages: usize,
    pub identical: bool,
    pub wire_identical: bool,
    pub events_identical: bool,
    pub differences: Vec<Difference>,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn load_manifest(root: &Path) -> Manifest {
    let m: Manifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m.version, 1, "unsupported replay format");
    assert_eq!(m.baseline, "v1.2.0");
    assert_eq!(m.commit, BASELINE);
    assert!(m.synthetic);
    assert_eq!(
        m.dependencies,
        BTreeMap::from([
            ("typescript".into(), "5.9.3".into()),
            ("zod".into(), "4.6.5".into())
        ])
    );
    let mut names = std::collections::BTreeSet::new();
    for c in &m.cases {
        assert!(names.insert(&c.name), "duplicate case {}", c.name);
        assert!(matches!(
            c.surface.as_str(),
            "codex" | "claude" | "exec" | "mcp"
        ));
        let p = Path::new(&c.file);
        assert!(
            p.components()
                .all(|p| matches!(p, std::path::Component::Normal(_)))
        );
        assert_eq!(
            sha256(&fs::read(root.join(p)).unwrap()),
            c.sha256,
            "fixture changed: {}",
            c.name
        );
    }
    m
}

/// Exact JSON equality: object key order is insignificant; array order, ids,
/// strings (including serialized tool content), omission and null remain exact.
/// Every differing leaf is reported, rather than stopping at the first message.
pub fn diff(expected: &Value, actual: &Value) -> Vec<Difference> {
    fn walk(path: String, a: Option<&Value>, b: Option<&Value>, out: &mut Vec<Difference>) {
        if a == b {
            return;
        }
        match (a, b) {
            (Some(Value::Object(a)), Some(Value::Object(b))) => {
                let keys = a
                    .keys()
                    .chain(b.keys())
                    .collect::<std::collections::BTreeSet<_>>();
                for k in keys {
                    walk(
                        format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                        a.get(k),
                        b.get(k),
                        out,
                    );
                }
            }
            (Some(Value::Array(a)), Some(Value::Array(b))) => {
                for i in 0..a.len().max(b.len()) {
                    walk(format!("{path}/{i}"), a.get(i), b.get(i), out);
                }
            }
            _ => out.push(Difference {
                path,
                expected: a.cloned(),
                actual: b.cloned(),
            }),
        }
    }
    let mut out = vec![];
    walk(String::new(), Some(expected), Some(actual), &mut out);
    out
}
fn msg(kind: &str, value: Value) -> Message {
    Message {
        kind: kind.into(),
        value,
        after: 0,
    }
}
fn absorb(e: Effects, out: &mut Observations) {
    out.wire
        .extend(e.outgoing.into_iter().map(|v| msg("out", v)));
    for ev in e.events {
        let v = match ev {
            Event::Trace { run, value } => {
                assert_eq!(run, "oracle-run");
                msg("trace", value)
            }
            Event::Progress { run, text } => {
                assert_eq!(run, "oracle-run");
                msg("progress", json!(text))
            }
            Event::Completed { run, result } => {
                assert_eq!(run, "oracle-run");
                msg("result", serde_json::to_value(result).unwrap())
            }
            Event::Session { run, id } => {
                assert_eq!(run, "oracle-run");
                out.lifecycle.push(msg("session", json!(id)));
                continue;
            }
            Event::Ready => {
                out.lifecycle.push(msg("ready", Value::Null));
                continue;
            }
            Event::Notification(v) => {
                out.lifecycle.push(msg("notification", v));
                continue;
            }
            Event::Warning(v) => msg("warning", json!(v)),
            Event::Stopped(v) => {
                out.lifecycle.push(msg("stopped", json!(v)));
                continue;
            }
        };
        out.events.push(v);
    }
    if e.close_stdin {
        out.lifecycle.push(msg("close-stdin", json!(true)));
    }
}

pub fn provider(case: &Case, fixture: &Value) -> (Observations, Observations) {
    let (expected, actual, _) = provider_inner(case, fixture, false);
    (expected, actual)
}
/// Exercise the production callback/inspection adapter as well as the raw
/// service observations. Full-observation strict identity remains a distinct gate.
pub fn provider_callbacks(
    case: &Case,
    fixture: &Value,
) -> (Observations, Observations, Observations) {
    provider_inner(case, fixture, true)
}
fn provider_inner(
    case: &Case,
    fixture: &Value,
    callbacks: bool,
) -> (Observations, Observations, Observations) {
    assert_eq!(fixture["baseline"], "v1.2.0");
    assert!(fixture["source"].is_string());
    let frames: Vec<Message> = serde_json::from_value(fixture["frames"].clone()).unwrap();
    let mut c = codex::Client::default();
    let authority = || Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap();
    let mut cl = claude::Client::new("oracle-run".into(), authority(), 0);
    let mut ex = pm_agents::codex_exec::Client::new(
        "oracle-run".into(),
        Authority::new(Access::Project, ApprovalMode::Auto, false).unwrap(),
        0,
    );
    let mut actual = Observations::default();
    let mut expected = Observations::default();
    let mut inspection = Observations::default();
    let mut queue = pm_agents::delivery::Queue::new(1024);
    let mut stimulus = 0;
    for f in frames {
        let v = f.value;
        if matches!(
            f.kind.as_str(),
            "run" | "in" | "answer" | "answer-return" | "steer" | "cancel"
        ) {
            stimulus += 1;
        }
        let observed = |kind: &str, value: Value| Message {
            kind: kind.into(),
            value,
            after: stimulus,
        };
        let e = match f.kind.as_str() {
            "run" => c.run(serde_json::from_value(v).unwrap(), 0).unwrap(),
            "in" => match case.surface.as_str() {
                "codex" => c.receive(v, 0),
                "claude" => cl.receive(v),
                "exec" => ex.receive(v),
                _ => panic!("invalid provider"),
            },
            "answer" | "answer-return" => {
                let answers: BTreeMap<String, Vec<String>> =
                    serde_json::from_value(v["answers"].clone()).unwrap();
                let result = if case.surface == "codex" {
                    c.answer(
                        v["id"].as_str().unwrap(),
                        v["itemId"].as_str().unwrap(),
                        &answers,
                    )
                } else {
                    cl.answer(v["itemId"].as_str().unwrap(), &answers)
                };
                if f.kind == "answer-return" {
                    expected
                        .events
                        .push(observed("answer-return", v["accepted"].clone()));
                    actual
                        .events
                        .push(observed("answer-return", json!(result.is_some())));
                } else {
                    assert!(result.is_some(), "{}: answer refused", case.name);
                }
                result.unwrap_or_default()
            }
            "steer" => c
                .steer(
                    v["id"].as_str().unwrap(),
                    v["prompt"].as_str().unwrap(),
                    &serde_json::from_value::<Vec<String>>(v["images"].clone()).unwrap(),
                    0,
                )
                .unwrap(),
            "cancel" => c.cancel(v.as_str().unwrap(), 0).unwrap(),
            "out" => {
                expected.wire.push(observed("out", v));
                continue;
            }
            "session" => {
                expected.lifecycle.push(observed("session", v));
                continue;
            }
            "close-stdin" => {
                expected.lifecycle.push(observed("close-stdin", v));
                continue;
            }
            "trace" | "progress" | "result" | "warning" => {
                expected.events.push(observed(&f.kind, v));
                continue;
            }
            other => panic!("unsupported frame {other}"),
        };
        let mut effects = Observations::default();
        if callbacks {
            use pm_agents::delivery::{Delivery, Surface};
            let surface = match case.surface.as_str() {
                "codex" => Surface::Codex,
                "claude" => Surface::Claude,
                "exec" => Surface::CodexExec,
                _ => panic!("invalid provider"),
            };
            queue.push(surface, e).unwrap();
            while let Some(entry) = queue.pop() {
                match entry.delivery {
                    Delivery::Wire(v) => effects.wire.push(msg("out", v)),
                    Delivery::Callback(event) => absorb(
                        Effects {
                            events: vec![event],
                            ..Effects::default()
                        },
                        &mut effects,
                    ),
                    Delivery::Inspection(event) => {
                        let mut observed = Observations::default();
                        absorb(
                            Effects {
                                events: vec![event],
                                ..Effects::default()
                            },
                            &mut observed,
                        );
                        for message in &mut observed.lifecycle {
                            message.after = stimulus;
                        }
                        inspection.lifecycle.extend(observed.lifecycle);
                    }
                    Delivery::CloseStdin => effects.lifecycle.push(msg("close-stdin", json!(true))),
                }
            }
        } else {
            absorb(e, &mut effects);
        }
        for messages in [
            &mut effects.wire,
            &mut effects.events,
            &mut effects.lifecycle,
        ] {
            for m in messages {
                m.after = stimulus;
            }
        }
        actual.wire.extend(effects.wire);
        actual.events.extend(effects.events);
        actual.lifecycle.extend(effects.lifecycle);
    }
    // Parser-only captures retain the actual tagged getter results. Translate
    // their terminal result at the runner boundary, retaining the source error.
    if let Some(t) = fixture.get("terminal") {
        let result = if let Some(error) = t["error"].as_str() {
            pm_agents::RunResult::failure(error, false)
        } else {
            let text = if !t["output"].is_null() {
                t["output"].to_string()
            } else {
                t["text"].as_str().unwrap().into()
            };
            pm_agents::RunResult::success(text, Access::Editor)
        };
        expected.events.push(Message {
            after: stimulus,
            ..msg("result", serde_json::to_value(result).unwrap())
        });
    }
    (expected, actual, inspection)
}
pub fn report(case: &Case, expected: &Observations, actual: &Observations) -> Report {
    let count = |o: &Observations| o.wire.len() + o.events.len() + o.lifecycle.len();
    let differences = diff(
        &serde_json::to_value(expected).unwrap(),
        &serde_json::to_value(actual).unwrap(),
    );
    Report {
        name: case.name.clone(),
        surface: case.surface.clone(),
        reference_messages: count(expected),
        native_messages: count(actual),
        identical: differences.is_empty(),
        wire_identical: expected.wire == actual.wire,
        events_identical: expected.events == actual.events,
        differences,
    }
}
