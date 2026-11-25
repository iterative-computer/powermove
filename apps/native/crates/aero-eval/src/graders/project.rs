//! Project-state assertions over the saved project document: layers, their
//! timing, effects, keyframes and transitions, and the composition root.
use super::{Context, Expect, show};
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSpec {
    pub assertions: Vec<Assertion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assertion {
    /// What a person would say is being checked.
    pub name: String,
    #[serde(flatten)]
    pub check: Check,
}

/// Which layers of the open composition an assertion is about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_prefix: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub layer_type: Option<String>,
    /// Text layers whose text contains this (case-insensitive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_contains: Option<String>,
}

impl Selector {
    pub fn matches(&self, l: &Value) -> bool {
        let s = |k: &str| l.get(k).and_then(Value::as_str).unwrap_or("");
        self.id.as_deref().is_none_or(|v| s("id") == v)
            && self.name.as_deref().is_none_or(|v| s("name") == v)
            && self
                .name_prefix
                .as_deref()
                .is_none_or(|v| s("name").starts_with(v))
            && self.layer_type.as_deref().is_none_or(|v| s("type") == v)
            && self.text_contains.as_deref().is_none_or(|v| {
                l.pointer("/d/text")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.to_lowercase().contains(&v.to_lowercase()))
            })
    }
    fn describe(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// How many layers match.
    Count {
        select: Selector,
        expect: Box<Expect>,
    },
    /// A JSON pointer relative to each matching layer (all of them, or any).
    Field {
        select: Selector,
        path: String,
        expect: Box<Expect>,
        #[serde(default)]
        any: bool,
    },
    /// The matching layers, by start time, have exactly these names.
    Order {
        select: Selector,
        names: Vec<String>,
    },
    /// The matching layers, by start time, butt against each other (no gap,
    /// no overlap) and the first starts at `start` when given.
    Contiguous {
        select: Selector,
        #[serde(default = "frame_tol")]
        tol: f64,
        #[serde(default)]
        start: Option<f64>,
    },
    /// Every matching layer carries an effect of this type (or `$new:effect`)
    /// that is switched on and whose parameters meet the expectations;
    /// `present: false` forbids it. `instances` bounds how many instances of
    /// the type a layer carries (on or off).
    Effect {
        select: Selector,
        #[serde(rename = "type")]
        effect: String,
        #[serde(default)]
        params: BTreeMap<String, Expect>,
        #[serde(default = "yes")]
        present: bool,
        #[serde(default)]
        instances: Option<Box<Expect>>,
    },
    /// A keyframed channel: an effect parameter (with `effect`, on a switched
    /// on instance), a transform channel (`position.x`), or a JSON pointer
    /// from the layer (`/d/...`). `changes`: the first and last keys differ.
    Animated {
        select: Selector,
        #[serde(default)]
        effect: Option<String>,
        param: String,
        #[serde(default = "two")]
        keys_ge: usize,
        #[serde(default)]
        first: Option<Box<KeyExpect>>,
        #[serde(default)]
        last: Option<Box<KeyExpect>>,
        #[serde(default)]
        changes: bool,
    },
    /// A transition on a matching layer's in, out, or either side.
    Transition {
        select: Selector,
        #[serde(default = "either")]
        side: String,
        #[serde(rename = "type")]
        transition: String,
        #[serde(default)]
        dur: Option<Box<Expect>>,
    },
    /// Matching layers are identical before and after, except `ignore` pointers.
    Unchanged {
        select: Selector,
        #[serde(default)]
        ignore: Vec<String>,
    },
    /// The matching layers are exactly the layers with these ids (none new,
    /// none renamed into place) and each still has its value from before the
    /// run at every `same` pointer (enabled state, source, trim, speed).
    Kept {
        select: Selector,
        ids: Vec<String>,
        #[serde(default)]
        same: Vec<String>,
    },
    /// Each matching layer's value at `path` differs from before (the change
    /// happened in this step, not earlier).
    Changed { select: Selector, path: String },
    /// Markers added since before: exactly `count`, each at a time meeting
    /// `t`; markers that were there stay.
    MarkersAdded {
        count: usize,
        #[serde(default)]
        t: Option<Box<Expect>>,
    },
    /// A JSON pointer from the composition root.
    Root { path: String, expect: Box<Expect> },
    /// At least one of these checks holds.
    AnyOf(Vec<Check>),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyExpect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<Box<Expect>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<Box<Expect>>,
}

fn frame_tol() -> f64 {
    // Half a frame at 30 fps.
    1.0 / 60.0
}
fn yes() -> bool {
    true
}
fn two() -> usize {
    2
}
fn either() -> String {
    "either".into()
}

pub fn grade(spec: &ProjectSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("project");
    let Some(after) = cx.artifacts.project_after.as_ref() else {
        v.missing("the project after the run was not collected");
        return v;
    };
    check_all(
        &spec.assertions,
        crate::pmv::project_of(after),
        cx.artifacts
            .project_before
            .as_ref()
            .map(crate::pmv::project_of),
        cx,
        &mut v,
    );
    v
}

/// Run assertions against one project state (also used after panel probes).
pub fn check_all(
    assertions: &[Assertion],
    after: &Value,
    before: Option<&Value>,
    cx: &Context,
    v: &mut Verdict,
) {
    for a in assertions {
        match check(&a.check, after, before, cx) {
            Ok(Ok(line)) => v.check(true, format!("{}: {line}", a.name)),
            Ok(Err(line)) => v.check(false, format!("{}: {line}", a.name)),
            Err(missing) => v.missing(format!("{}: {missing}", a.name)),
        }
    }
}

pub fn layers(project: &Value) -> Vec<&Value> {
    project
        .get("layers")
        .and_then(Value::as_array)
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}

fn by_start<'a>(project: &'a Value, select: &Selector) -> Vec<&'a Value> {
    let mut v: Vec<&Value> = layers(project)
        .into_iter()
        .filter(|l| select.matches(l))
        .collect();
    v.sort_by(|a, b| f(a, "from").total_cmp(&f(b, "from")));
    v
}

fn f(l: &Value, k: &str) -> f64 {
    l.get(k).and_then(super::number).unwrap_or(0.0)
}
fn name(l: &Value) -> &str {
    l.get("name").and_then(Value::as_str).unwrap_or("?")
}

type Outcome = Result<Result<String, String>, String>;

fn check(c: &Check, after: &Value, before: Option<&Value>, cx: &Context) -> Outcome {
    let ok = |b: bool, s: String| Ok(if b { Ok(s) } else { Err(s) });
    match c {
        Check::Count { select, expect } => {
            let n = by_start(after, select).len();
            ok(
                expect.holds(Some(&Value::from(n))),
                format!(
                    "{n} layers match {}, expected {}",
                    select.describe(),
                    expect.describe()
                ),
            )
        }
        Check::Field {
            select,
            path,
            expect,
            any,
        } => {
            let ls = by_start(after, select);
            if ls.is_empty() {
                return ok(false, format!("no layer matches {}", select.describe()));
            }
            let seen: Vec<(String, bool)> = ls
                .iter()
                .map(|l| {
                    let got = l.pointer(path);
                    (
                        format!("{} {path} = {}", name(l), show(got)),
                        expect.holds(got),
                    )
                })
                .collect();
            let pass = if *any {
                seen.iter().any(|s| s.1)
            } else {
                seen.iter().all(|s| s.1)
            };
            let lines: Vec<String> = seen.into_iter().map(|s| s.0).collect();
            ok(
                pass,
                format!("{}; expected {}", lines.join(", "), expect.describe()),
            )
        }
        Check::Order { select, names } => {
            let got: Vec<&str> = by_start(after, select).into_iter().map(name).collect();
            ok(
                got == names.iter().map(String::as_str).collect::<Vec<_>>(),
                format!("order {got:?}, expected {names:?}"),
            )
        }
        Check::Contiguous { select, tol, start } => {
            let ls = by_start(after, select);
            if ls.is_empty() {
                return ok(false, format!("no layer matches {}", select.describe()));
            }
            let mut problems = Vec::new();
            if let Some(s) = start
                && (f(ls[0], "from") - s).abs() > *tol
            {
                problems.push(format!("{} starts at {:.3}", name(ls[0]), f(ls[0], "from")));
            }
            for w in ls.windows(2) {
                let end = f(w[0], "from") + f(w[0], "dur");
                let gap = f(w[1], "from") - end;
                if gap.abs() > *tol {
                    problems.push(format!(
                        "{} {} {} by {:.3}s",
                        name(w[0]),
                        if gap > 0.0 { "gap before" } else { "overlaps" },
                        name(w[1]),
                        gap.abs()
                    ));
                }
            }
            ok(
                problems.is_empty(),
                if problems.is_empty() {
                    format!("{} layers butt end to end", ls.len())
                } else {
                    problems.join("; ")
                },
            )
        }
        Check::Effect {
            select,
            effect,
            params,
            present,
            instances,
        } => {
            let ls = by_start(after, select);
            if ls.is_empty() {
                return ok(false, format!("no layer matches {}", select.describe()));
            }
            let types = cx.resolve(effect);
            if types.is_empty() && *present {
                return ok(false, format!("no extension registered {effect}"));
            }
            let mut lines = Vec::new();
            let mut pass = true;
            for l in ls {
                let fx: Vec<&Value> = l
                    .get("fx")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter(|e| {
                                e.get("type")
                                    .and_then(Value::as_str)
                                    .is_some_and(|t| types.iter().any(|x| x == t))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !*present {
                    pass &= fx.is_empty();
                    lines.push(format!("{} has {} {effect}", name(l), fx.len()));
                    continue;
                }
                if let Some(x) = instances
                    && !x.holds(Some(&Value::from(fx.len())))
                {
                    pass = false;
                    lines.push(format!(
                        "{} has {} instances of {effect}, expected {}",
                        name(l),
                        fx.len(),
                        x.describe()
                    ));
                    continue;
                }
                // Any one instance meeting every parameter expectation counts.
                let hit = fx.iter().find(|e| {
                    e.get("on").and_then(Value::as_bool) != Some(false)
                        && params
                            .iter()
                            .all(|(k, x)| x.holds(e.pointer(&format!("/p/{k}"))))
                });
                match hit {
                    Some(e) => lines.push(format!(
                        "{} has {} {}",
                        name(l),
                        e["type"].as_str().unwrap_or("?"),
                        params
                            .keys()
                            .map(|k| format!("{k}={}", show(e.pointer(&format!("/p/{k}/v")))))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                    None => {
                        pass = false;
                        lines.push(if fx.is_empty() {
                            format!("{} has no {effect}", name(l))
                        } else {
                            format!(
                                "{} has {effect} but not with {} (or it is off)",
                                name(l),
                                serde_json::to_string(params).unwrap_or_default()
                            )
                        });
                    }
                }
            }
            ok(pass, lines.join("; "))
        }
        Check::Animated {
            select,
            effect,
            param,
            keys_ge,
            first,
            last,
            changes,
        } => {
            let ls = by_start(after, select);
            if ls.is_empty() {
                return ok(false, format!("no layer matches {}", select.describe()));
            }
            let types = effect.as_deref().map(|e| cx.resolve(e));
            let mut lines = Vec::new();
            let mut pass = true;
            for l in ls {
                let channel = match &types {
                    // A switched-off instance animates nothing.
                    Some(types) => l.get("fx").and_then(Value::as_array).and_then(|a| {
                        a.iter()
                            .filter(|e| {
                                e.get("on").and_then(Value::as_bool) != Some(false)
                                    && e.get("type")
                                        .and_then(Value::as_str)
                                        .is_some_and(|t| types.iter().any(|x| x == t))
                            })
                            .find_map(|e| e.pointer(&format!("/p/{param}")))
                    }),
                    None if param.starts_with('/') => l.pointer(param),
                    None => l.get("p").and_then(|p| p.get(param.as_str())),
                };
                let mut keys: Vec<&Value> = channel
                    .and_then(|c| c.get("kf"))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().collect())
                    .unwrap_or_default();
                keys.sort_by(|a, b| f(a, "t").total_cmp(&f(b, "t")));
                let key_ok = |k: Option<&&Value>, x: &Option<Box<KeyExpect>>| {
                    x.as_ref().is_none_or(|x| {
                        k.is_some_and(|k| {
                            x.t.as_ref().is_none_or(|e| e.holds(k.get("t")))
                                && x.v.as_ref().is_none_or(|e| e.holds(k.get("v")))
                        })
                    })
                };
                let moves = !*changes
                    || keys
                        .first()
                        .zip(keys.last())
                        .is_some_and(|(a, b)| a.get("v") != b.get("v"));
                let good = keys.len() >= *keys_ge
                    && key_ok(keys.first(), first)
                    && key_ok(keys.last(), last)
                    && moves;
                pass &= good;
                lines.push(format!(
                    "{} {param}: {} keys {}",
                    name(l),
                    keys.len(),
                    keys.iter()
                        .map(|k| format!("({}, {})", show(k.get("t")), show(k.get("v"))))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
            ok(pass, lines.join("; "))
        }
        Check::Transition {
            select,
            side,
            transition,
            dur,
        } => {
            let ls = by_start(after, select);
            if ls.is_empty() {
                return ok(false, format!("no layer matches {}", select.describe()));
            }
            let types = cx.resolve(transition);
            if types.is_empty() {
                return ok(false, format!("no extension registered {transition}"));
            }
            let sides: &[&str] = match side.as_str() {
                "in" => &["transitionIn"],
                "out" => &["transitionOut"],
                _ => &["transitionIn", "transitionOut"],
            };
            let mut lines = Vec::new();
            let mut pass = false;
            for l in &ls {
                for s in sides {
                    let Some(t) = l.get(*s).filter(|t| t.is_object()) else {
                        continue;
                    };
                    let ty = t.get("type").and_then(Value::as_str).unwrap_or("");
                    let good = types.iter().any(|x| x == ty)
                        && t.get("missing").and_then(Value::as_bool) != Some(true)
                        && dur.as_ref().is_none_or(|d| d.holds(t.get("dur")));
                    pass |= good;
                    lines.push(format!("{} {s} {ty} dur {}", name(l), show(t.get("dur"))));
                }
            }
            if lines.is_empty() {
                lines.push(format!("no transition on {}", select.describe()));
            }
            ok(pass, lines.join("; "))
        }
        Check::Unchanged { select, ignore } => {
            let Some(before) = before else {
                return Err("the project before the run was not collected".into());
            };
            let strip = |l: &Value| {
                let mut l = l.clone();
                for p in ignore {
                    remove_pointer(&mut l, p);
                }
                l
            };
            let was = by_start(before, select);
            if was.is_empty() {
                return ok(
                    false,
                    format!("no layer matches {} before", select.describe()),
                );
            }
            let now = layers(after);
            let mut lines = Vec::new();
            for l in was {
                let id = l.get("id");
                match now.iter().find(|n| n.get("id") == id) {
                    None => lines.push(format!("{} was removed", name(l))),
                    Some(n) if strip(n) != strip(l) => lines.push(format!("{} changed", name(l))),
                    Some(_) => {}
                }
            }
            ok(
                lines.is_empty(),
                if lines.is_empty() {
                    "untouched".into()
                } else {
                    lines.join("; ")
                },
            )
        }
        Check::AnyOf(checks) => {
            let mut lines = Vec::new();
            for c in checks {
                match check(c, after, before, cx)? {
                    Ok(line) => return Ok(Ok(format!("one holds: {line}"))),
                    Err(line) => lines.push(line),
                }
            }
            ok(false, format!("none holds: {}", lines.join(" | ")))
        }
        Check::Kept { select, ids, same } => {
            let Some(before) = before else {
                return Err("the project before the run was not collected".into());
            };
            let got: Vec<&Value> = by_start(after, select);
            let mut have: Vec<&str> = got
                .iter()
                .map(|l| l.get("id").and_then(Value::as_str).unwrap_or("?"))
                .collect();
            have.sort();
            let mut want: Vec<&str> = ids.iter().map(String::as_str).collect();
            want.sort();
            let mut problems = Vec::new();
            if have != want {
                problems.push(format!("layer ids {have:?}, expected {want:?}"));
            }
            let was = layers(before);
            for l in &got {
                let id = l.get("id");
                let Some(b) = was.iter().find(|b| b.get("id") == id) else {
                    continue;
                };
                for p in same {
                    if l.pointer(p) != b.pointer(p) {
                        problems.push(format!(
                            "{} {p} is {} (was {})",
                            name(l),
                            show(l.pointer(p)),
                            show(b.pointer(p))
                        ));
                    }
                }
            }
            ok(
                problems.is_empty(),
                if problems.is_empty() {
                    format!("{} kept layers with their {}", got.len(), same.join(", "))
                } else {
                    problems.join("; ")
                },
            )
        }
        Check::Changed { select, path } => {
            let Some(before) = before else {
                return Err("the state before this step was not collected".into());
            };
            let ls = by_start(after, select);
            if ls.is_empty() {
                return ok(false, format!("no layer matches {}", select.describe()));
            }
            let was = layers(before);
            let mut pass = true;
            let mut lines = Vec::new();
            for l in ls {
                let old = was
                    .iter()
                    .find(|b| b.get("id") == l.get("id"))
                    .and_then(|b| b.pointer(path));
                let new = l.pointer(path);
                pass &= old != new;
                lines.push(format!("{} {path} {} → {}", name(l), show(old), show(new)));
            }
            ok(pass, lines.join("; "))
        }
        Check::MarkersAdded { count, t } => {
            let Some(before) = before else {
                return Err("the state before this step was not collected".into());
            };
            let list = |p: &Value| -> Vec<Value> {
                p.get("markers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            };
            let (was, now) = (list(before), list(after));
            let lost = was.iter().filter(|m| !now.contains(m)).count();
            let added: Vec<&Value> = now.iter().filter(|m| !was.contains(m)).collect();
            let times_ok = t
                .as_ref()
                .is_none_or(|e| added.iter().all(|m| e.holds(m.get("t"))));
            ok(
                added.len() == *count && lost == 0 && times_ok,
                format!(
                    "{} markers added at {:?}, {lost} lost; expected {count}{}",
                    added.len(),
                    added.iter().map(|m| show(m.get("t"))).collect::<Vec<_>>(),
                    t.as_ref()
                        .map(|e| format!(" at {}", e.describe()))
                        .unwrap_or_default()
                ),
            )
        }
        Check::Root { path, expect } => {
            let got = after.pointer(path);
            ok(
                expect.holds(got),
                format!("{path} = {}, expected {}", show(got), expect.describe()),
            )
        }
    }
}

impl Check {
    /// Whether the check names a contribution the run registered (`$new:…`).
    pub fn uses_new(&self) -> bool {
        let new = |s: &str| s.starts_with("$new");
        match self {
            Check::Effect { effect, .. } => new(effect),
            Check::Animated { effect, .. } => effect.as_deref().is_some_and(new),
            Check::Transition { transition, .. } => new(transition),
            Check::AnyOf(c) => c.iter().any(Check::uses_new),
            _ => false,
        }
    }
}

fn remove_pointer(v: &mut Value, pointer: &str) {
    let Some((parent, key)) = pointer.rsplit_once('/') else {
        return;
    };
    match v.pointer_mut(parent) {
        Some(Value::Object(m)) => {
            m.remove(key);
        }
        Some(Value::Array(a)) => {
            if let Ok(i) = key.parse::<usize>()
                && i < a.len()
            {
                a.remove(i);
            }
        }
        _ => {}
    }
}
