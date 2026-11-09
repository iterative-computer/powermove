//! Immutable observations from the native media worker. Persisted dimensions,
//! paths and proxy presence do not prove that an original can be decoded.
use crate::{MediaRuntime, RuntimeState, Stamp};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObservedRuntime {
    stamp: Option<Stamp>,
    states: BTreeMap<String, RuntimeState>,
}
impl ObservedRuntime {
    /// How many of `assets` are missing (the Project panel's offline filter).
    pub fn missing<'a>(&self, assets: impl IntoIterator<Item = &'a str>) -> usize {
        assets
            .into_iter()
            .filter(|id| {
                self.states
                    .get(*id)
                    .is_some_and(|s| s.availability.is_missing())
            })
            .count()
    }
    pub fn new(stamp: Stamp, states: BTreeMap<String, RuntimeState>) -> Self {
        Self {
            stamp: Some(stamp),
            states,
        }
    }
    /// Carry these observations to a revision that changed no media input.
    /// The caller proves that; `state` still refuses every other stamp.
    pub fn rebound(mut self, stamp: Stamp) -> Self {
        self.stamp = Some(stamp);
        self
    }
    /// These observations with `f` applied to each asset's state (what the
    /// host tracks itself, such as renditions).
    pub fn with_states(mut self, f: impl Fn(&str, &mut RuntimeState)) -> Self {
        for (id, state) in &mut self.states {
            f(id, state);
        }
        self
    }
}
impl MediaRuntime for ObservedRuntime {
    fn state(&self, stamp: &Stamp, asset: &str) -> RuntimeState {
        if self.stamp.as_ref() != Some(stamp) {
            return RuntimeState::default();
        }
        self.states.get(asset).cloned().unwrap_or_default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Availability;
    #[test]
    fn missing_counts_files_the_offline_filter_lists() {
        let stamp = Stamp {
            project: "p".into(),
            document: 1,
            revision: 2,
        };
        let with = |availability| RuntimeState {
            availability,
            ..Default::default()
        };
        let observed = ObservedRuntime::new(
            stamp,
            BTreeMap::from([
                ("ready".into(), with(Availability::Ready)),
                ("gone".into(), with(Availability::Offline)),
                ("broken".into(), with(Availability::Failed("x".into()))),
                ("loading".into(), with(Availability::Loading)),
            ]),
        );
        assert_eq!(observed.missing(["ready", "gone", "broken", "loading"]), 2);
        // Only the project's own assets count.
        assert_eq!(observed.missing(["gone"]), 1);
    }
    #[test]
    fn readiness_does_not_cross_relink_document_or_project_boundaries() {
        let stamp = Stamp {
            project: "p".into(),
            document: 1,
            revision: 2,
        };
        let ready = RuntimeState {
            availability: Availability::Ready,
            ..Default::default()
        };
        let observed = ObservedRuntime::new(
            stamp.clone(),
            BTreeMap::from([("asset".into(), ready.clone())]),
        );
        assert_eq!(observed.state(&stamp, "asset"), ready);
        assert_eq!(observed.state(&stamp, "missing"), RuntimeState::default());
        let mut changed = stamp.clone();
        changed.revision += 1;
        assert_eq!(observed.state(&changed, "asset"), RuntimeState::default());
        changed = stamp.clone();
        changed.document += 1;
        assert_eq!(observed.state(&changed, "asset"), RuntimeState::default());
        changed = stamp.clone();
        changed.project = "other".into();
        assert_eq!(observed.state(&changed, "asset"), RuntimeState::default());
        changed = stamp.clone();
        changed.revision += 1;
        let rebound = observed.rebound(changed.clone());
        assert_eq!(rebound.state(&changed, "asset"), ready);
        assert_eq!(rebound.state(&stamp, "asset"), RuntimeState::default());
    }
}
