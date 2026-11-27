//! "Every extension the run changed loads, in every window", from the runner's
//! extension report, plus the contributions the task needs registered.
use super::Context;
use crate::verdict::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadSpec {
    /// The run must create or update at least this many extensions.
    #[serde(default = "one")]
    pub min_changed: usize,
    /// Contributions that must be registered after the run.
    #[serde(default)]
    pub require: Vec<Required>,
    /// These extensions must be among the changed ones (a fix task's target).
    #[serde(default)]
    pub must_change: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Required {
    pub kind: String,
    /// An exact id, or `$new` for any id registered by a changed extension.
    pub id: String,
    #[serde(default)]
    pub params_ge: usize,
}

fn one() -> usize {
    1
}

pub fn grade(spec: &LoadSpec, cx: &Context) -> Verdict {
    let mut v = Verdict::new("extensions-load");
    let r = &cx.artifacts.extensions;
    for c in r.changed.iter().filter(|c| c.action == "refused") {
        v.check(
            false,
            format!("{} contains a symlink, so nothing in it was read", c.id),
        );
    }
    let live: Vec<_> = r
        .changed
        .iter()
        .filter(|c| c.action == "created" || c.action == "updated")
        .collect();
    v.check(
        live.len() >= spec.min_changed,
        format!(
            "{} extensions created or updated ({}), expected at least {}",
            live.len(),
            live.iter()
                .map(|c| format!("{} {}", c.id, c.action))
                .collect::<Vec<_>>()
                .join(", "),
            spec.min_changed
        ),
    );
    for id in &spec.must_change {
        v.check(
            live.iter().any(|c| &c.id == id),
            format!("{id} was changed by the run"),
        );
    }
    if !live.is_empty() && r.windows.is_empty() {
        v.missing("the host reported no windows, so load status is unknown");
    }
    for c in &live {
        for w in &r.windows {
            match r.health.iter().find(|h| h.id == c.id && &h.window == w) {
                None => v.missing(format!("no load status for {} in window {w}", c.id)),
                Some(h) => v.check(
                    h.state == "ok",
                    format!(
                        "{} in window {w}: {}{}",
                        c.id,
                        h.state,
                        h.error
                            .as_deref()
                            .map(|e| format!(" ({e})"))
                            .unwrap_or_default()
                    ),
                ),
            }
        }
    }
    for q in &spec.require {
        let ids = if q.id == "$new" {
            cx.resolve(&format!("$new:{}", q.kind))
        } else {
            vec![q.id.clone()]
        };
        let found = r
            .registered
            .iter()
            .find(|g| g.kind == q.kind && ids.contains(&g.id) && g.params >= q.params_ge);
        v.check(
            found.is_some(),
            match found {
                Some(g) => format!("{} {} registered by {}", g.kind, g.id, g.extension),
                None => format!(
                    "no {} {} registered{}",
                    q.kind,
                    q.id,
                    if q.params_ge > 0 {
                        format!(" with at least {} parameters", q.params_ge)
                    } else {
                        String::new()
                    }
                ),
            },
        );
    }
    v
}
