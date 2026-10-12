//! Crash breadcrumbs and attribution inside an engine (contract 7,
//! section 19 "Crash handling").
//!
//! [`InFlightTracker`] counts the attributable items (assets, effects,
//! definitions, layers) the engine is working on and produces an
//! [`InFlight`] event whenever the set changes, so the host knows the
//! suspects if the process dies mid-frame. [`FailedItems`] holds the items
//! the host marked failed after two attributed crashes; the engine refuses
//! work on them with `CrashAttributed` instead of crashing again.

use std::collections::{BTreeMap, BTreeSet};

use pm_protocol::error::{
    CrashReport, EngineError, ErrorCode, ErrorScope, InFlight, MAX_IN_FLIGHT_SUSPECTS,
};

/// A stable sort key for scopes (`ErrorScope` has no `Ord`).
fn key(scope: &ErrorScope) -> String {
    match scope {
        ErrorScope::Asset { asset } => format!("asset\u{0}{}", asset.as_str()),
        ErrorScope::Effect {
            layer,
            effect,
            definition,
        } => format!(
            "effect\u{0}{}\u{0}{}\u{0}{}",
            layer.as_str(),
            effect.as_str(),
            definition.as_str()
        ),
        ErrorScope::Definition { definition } => format!("definition\u{0}{}", definition.as_str()),
        ErrorScope::Layer { layer } => format!("layer\u{0}{}", layer.as_str()),
        other => format!("other\u{0}{other:?}"),
    }
}

/// Work that names attributable items. Returned by [`InFlightTracker::enter`]
/// and given back to [`InFlightTracker::leave`].
#[derive(Debug)]
#[must_use = "leave the tracker when the work ends"]
pub struct WorkToken {
    keys: Vec<String>,
}

/// The engine's breadcrumb: which attributable items are in flight.
#[derive(Debug, Default)]
pub struct InFlightTracker {
    items: BTreeMap<String, (ErrorScope, u32)>,
    /// The set last reported to the host.
    reported: BTreeSet<String>,
}

impl InFlightTracker {
    /// Marks `scopes` as in flight. Scopes that cannot be attributed
    /// (requests, jobs, sessions, connections) are ignored.
    pub fn enter(&mut self, scopes: &[ErrorScope]) -> WorkToken {
        let mut keys = Vec::new();
        for scope in scopes.iter().filter(|s| s.is_attributable()) {
            let k = key(scope);
            self.items
                .entry(k.clone())
                .and_modify(|(_, count)| *count += 1)
                .or_insert_with(|| (scope.clone(), 1));
            keys.push(k);
        }
        WorkToken { keys }
    }

    /// The work behind `token` ended (finished or failed).
    pub fn leave(&mut self, token: WorkToken) {
        for k in token.keys {
            if let Some((_, count)) = self.items.get_mut(&k) {
                *count -= 1;
                if *count == 0 {
                    self.items.remove(&k);
                }
            }
        }
    }

    /// Items in flight now.
    pub fn suspects(&self) -> Vec<ErrorScope> {
        self.items
            .values()
            .map(|(scope, _)| scope.clone())
            .collect()
    }

    /// The breadcrumb to send when the set changed since the last report.
    /// More than [`MAX_IN_FLIGHT_SUSPECTS`] items keep the first ones in key
    /// order (the host attributes what it is told; a frame touching that many
    /// items is not attributable item by item anyway).
    pub fn take_change(&mut self) -> Option<InFlight> {
        let now: BTreeSet<String> = self.items.keys().cloned().collect();
        if now == self.reported {
            return None;
        }
        self.reported = now;
        Some(InFlight {
            suspects: self
                .items
                .values()
                .take(MAX_IN_FLIGHT_SUSPECTS)
                .map(|(scope, _)| scope.clone())
                .collect(),
        })
    }

    /// Forget what was reported (a new host connection gets the full set).
    pub fn reset_reported(&mut self) {
        self.reported.clear();
    }
}

/// Items marked failed for the session by crash attribution.
#[derive(Debug, Default, Clone)]
pub struct FailedItems {
    items: BTreeMap<String, ErrorScope>,
}

impl FailedItems {
    /// Takes the host's attribution (`Event::Crash` sent to a restarted
    /// engine lists every item failed so far). Returns how many are new.
    pub fn apply(&mut self, report: &CrashReport) -> usize {
        let mut added = 0;
        for scope in report.marked_failed.iter().filter(|s| s.is_attributable()) {
            if self.items.insert(key(scope), scope.clone()).is_none() {
                added += 1;
            }
        }
        added
    }

    pub fn is_failed(&self, scope: &ErrorScope) -> bool {
        self.items.contains_key(&key(scope))
    }

    /// Refuses work that touches a failed item. An effect also fails when
    /// its definition is failed.
    pub fn check(&self, scopes: &[ErrorScope]) -> Result<(), Box<EngineError>> {
        for scope in scopes {
            let failed = self.is_failed(scope)
                || matches!(scope, ErrorScope::Effect { definition, .. }
                    if self.is_failed(&ErrorScope::Definition { definition: definition.clone() }));
            if failed {
                return Err(Box::new(EngineError::new(
                    ErrorCode::CrashAttributed,
                    scope.clone(),
                    "This item crashed the engine twice and stays off until the project is reopened.",
                )));
            }
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::Validate;
    use pm_protocol::error::EngineProcess;

    fn asset(id: &str) -> ErrorScope {
        ErrorScope::Asset { asset: id.into() }
    }

    #[test]
    fn the_breadcrumb_changes_only_when_the_set_does() {
        let mut tracker = InFlightTracker::default();
        assert_eq!(tracker.take_change(), None);
        let a = tracker.enter(&[asset("a"), ErrorScope::Connection]);
        let b = tracker.enter(&[asset("a"), asset("b")]);
        let crumb = tracker.take_change().unwrap();
        assert_eq!(crumb.suspects, vec![asset("a"), asset("b")]);
        assert!(crumb.validate().is_ok());
        assert_eq!(tracker.take_change(), None);
        tracker.leave(a);
        assert_eq!(tracker.take_change(), None, "a is still held by b");
        tracker.leave(b);
        assert_eq!(tracker.take_change().unwrap().suspects, vec![]);
        assert_eq!(tracker.take_change(), None);
    }

    #[test]
    fn failed_items_are_refused_and_failed_definitions_take_their_effects() {
        let mut failed = FailedItems::default();
        let report = CrashReport {
            process: EngineProcess::Interactive,
            attempt: 2,
            exit: "signal SIGSEGV".into(),
            suspects: vec![],
            marked_failed: vec![
                asset("a"),
                ErrorScope::Definition {
                    definition: "glow".into(),
                },
                ErrorScope::Connection,
            ],
            restart_in_ms: 500,
        };
        assert_eq!(failed.apply(&report), 2);
        assert_eq!(failed.apply(&report), 0);
        assert!(failed.check(&[asset("b")]).is_ok());
        let error = failed.check(&[asset("b"), asset("a")]).unwrap_err();
        assert_eq!(error.code, ErrorCode::CrashAttributed);
        assert_eq!(error.scope, asset("a"));
        let effect = ErrorScope::Effect {
            layer: "l".into(),
            effect: "e".into(),
            definition: "glow".into(),
        };
        assert!(failed.check(&[effect]).is_err());
        assert!(error.validate().is_ok());
    }
}
