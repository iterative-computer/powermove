//! Shared update admission and durable staging policy. No application services,
//! migration coordinator, UI, network transport or installer implementation.
pub mod update_distribution;
pub mod update_handoff;
pub mod updater;
