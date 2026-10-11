//! Contract identities, versions, and version negotiation (contract 9).
//!
//! Every connection starts with `Hello` from the side that opened it and
//! `Welcome` from the other side. The wire layout version must match exactly;
//! each contract then negotiates to the highest version both sides support.
//! Version 0 marks a draft: drafts interoperate only between peers built from
//! the same source (equal `build` ids), because a draft may change without a
//! version bump. Freezing a contract sets it to 1; every later change needs an
//! RFC and a version bump (section 26, Phase 0).

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// Version of the envelope layout itself. Peers with different wire versions
/// cannot exchange anything but a `Hello` and the error that rejects it.
pub const WIRE_VERSION: u16 = 1;

/// The fifteen contracts of section 27.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ContractId {
    /// 1. `EvaluatedScene`.
    EvaluatedScene,
    /// 2. `FrameRequest` and `FrameDescriptor`.
    Frame,
    /// 3. The `pm-gpu` interop trait.
    GpuInterop,
    /// 4. The color contract.
    Color,
    /// 5. Patches (message envelope; semantics in pm-model).
    Patch,
    /// 6. The playback clock.
    Clock,
    /// 7. Errors and isolation.
    Errors,
    /// 8. Budget leases.
    Lease,
    /// 9. Protocol codec and code generation.
    Protocol,
    /// 10. Definition snapshots and lifecycle.
    Definitions,
    /// 11. The Blender job protocol, snapshot, and surface format.
    Blender,
    /// 12. Priority classes and agent frame requests.
    Priority,
    /// 13. GPU time slicing and deadline signals.
    Slicing,
    /// 14. Host services.
    HostServices,
    /// 15. The headless document host interface.
    DocumentHost,
}

impl ContractId {
    pub const ALL: [ContractId; 15] = [
        ContractId::EvaluatedScene,
        ContractId::Frame,
        ContractId::GpuInterop,
        ContractId::Color,
        ContractId::Patch,
        ContractId::Clock,
        ContractId::Errors,
        ContractId::Lease,
        ContractId::Protocol,
        ContractId::Definitions,
        ContractId::Blender,
        ContractId::Priority,
        ContractId::Slicing,
        ContractId::HostServices,
        ContractId::DocumentHost,
    ];

    /// The contract's number in section 27.
    pub const fn number(self) -> u8 {
        match self {
            ContractId::EvaluatedScene => 1,
            ContractId::Frame => 2,
            ContractId::GpuInterop => 3,
            ContractId::Color => 4,
            ContractId::Patch => 5,
            ContractId::Clock => 6,
            ContractId::Errors => 7,
            ContractId::Lease => 8,
            ContractId::Protocol => 9,
            ContractId::Definitions => 10,
            ContractId::Blender => 11,
            ContractId::Priority => 12,
            ContractId::Slicing => 13,
            ContractId::HostServices => 14,
            ContractId::DocumentHost => 15,
        }
    }

    /// The version this build implements. All contracts are drafts (0) until
    /// their consumer phase starts and they are frozen.
    pub const fn current_version(self) -> u16 {
        0
    }

    /// The RFC that specifies the contract, relative to the repository root.
    pub const fn rfc(self) -> &'static str {
        match self {
            ContractId::EvaluatedScene => "docs/engine/rfcs/01-evaluated-scene.md",
            ContractId::Frame => "docs/engine/rfcs/02-frame-request-and-descriptor.md",
            ContractId::GpuInterop => "docs/engine/rfcs/03-gpu-interop.md",
            ContractId::Color => "docs/engine/rfcs/04-color.md",
            ContractId::Patch => "docs/engine/rfcs/05-patches.md",
            ContractId::Clock => "docs/engine/rfcs/06-playback-clock.md",
            ContractId::Errors => "docs/engine/rfcs/07-errors-and-isolation.md",
            ContractId::Lease => "docs/engine/rfcs/08-budget-leases.md",
            ContractId::Protocol => "docs/engine/rfcs/09-protocol-codec.md",
            ContractId::Definitions => "docs/engine/rfcs/10-definition-snapshots.md",
            ContractId::Blender => "docs/engine/rfcs/11-blender-jobs.md",
            ContractId::Priority => "docs/engine/rfcs/12-priority-and-agent-frames.md",
            ContractId::Slicing => "docs/engine/rfcs/13-gpu-time-slicing.md",
            ContractId::HostServices => "docs/engine/rfcs/14-host-services.md",
            ContractId::DocumentHost => "docs/engine/rfcs/15-document-host.md",
        }
    }
}

/// The versions of one contract a peer can speak, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ContractRange {
    pub contract: ContractId,
    pub min: u16,
    pub max: u16,
    /// When true, the connection fails if this contract cannot be negotiated.
    pub required: bool,
}

/// A negotiated contract version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ContractVersion {
    pub contract: ContractId,
    pub version: u16,
}

/// Which process a peer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum PeerRole {
    /// Electron main or the CLI host (host services, contract 14).
    Host,
    /// A renderer window (UI, kernel, pm-wasm).
    Renderer,
    /// The interactive engine utility process.
    InteractiveEngine,
    /// The job engine utility process.
    JobEngine,
    /// A headless document host worker (contract 15).
    DocumentHost,
    /// pm-cli.
    Cli,
}

/// First message on every connection, from the side that opened it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub role: PeerRole,
    /// Must equal the peer's [`WIRE_VERSION`].
    pub wire: u16,
    /// Source build identity (git commit plus dirty flag). Draft contracts
    /// require equal build ids on both sides.
    pub build: String,
    pub contracts: Vec<ContractRange>,
}

/// Reply to `Hello` with the negotiated versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub role: PeerRole,
    pub wire: u16,
    pub build: String,
    /// One entry per contract both sides speak, sorted by contract.
    pub contracts: Vec<ContractVersion>,
    /// Largest message the sender accepts, in bytes.
    pub max_message_bytes: u32,
}

/// Why negotiation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NegotiationError {
    #[error("wire version mismatch: local {local}, remote {remote}")]
    WireMismatch { local: u16, remote: u16 },
    #[error("contract {contract:?} has no common version (local {local_min}..={local_max}, remote {remote_min}..={remote_max})")]
    NoCommonVersion {
        contract: ContractId,
        local_min: u16,
        local_max: u16,
        remote_min: u16,
        remote_max: u16,
    },
    #[error("contract {contract:?} is required but the peer does not speak it")]
    MissingRequired { contract: ContractId },
    #[error("draft contract {contract:?} needs equal builds (local {local}, remote {remote})")]
    DraftBuildMismatch { contract: ContractId, local: String, remote: String },
    #[error("contract {contract:?} is listed twice or has min > max")]
    MalformedRange { contract: ContractId },
}

/// The ranges this build supports: every contract at its current version,
/// all required.
pub fn local_ranges() -> Vec<ContractRange> {
    ContractId::ALL
        .iter()
        .map(|&contract| ContractRange {
            contract,
            min: contract.current_version(),
            max: contract.current_version(),
            required: true,
        })
        .collect()
}

fn check_ranges(ranges: &[ContractRange]) -> Result<(), NegotiationError> {
    let mut seen = std::collections::BTreeSet::new();
    for range in ranges {
        if range.min > range.max || !seen.insert(range.contract) {
            return Err(NegotiationError::MalformedRange { contract: range.contract });
        }
    }
    Ok(())
}

/// Negotiates `local` against a received `remote` hello. Returns the agreed
/// versions sorted by contract. Contracts only one side lists are left out
/// unless that side marks them required. A contract negotiated to version 0
/// (draft) also requires equal build ids.
pub fn negotiate(
    local_build: &str,
    local: &[ContractRange],
    remote: &Hello,
) -> Result<Vec<ContractVersion>, NegotiationError> {
    if remote.wire != WIRE_VERSION {
        return Err(NegotiationError::WireMismatch { local: WIRE_VERSION, remote: remote.wire });
    }
    check_ranges(local)?;
    check_ranges(&remote.contracts)?;
    let mut agreed = Vec::new();
    for mine in local {
        let theirs = remote.contracts.iter().find(|r| r.contract == mine.contract);
        let Some(theirs) = theirs else {
            if mine.required {
                return Err(NegotiationError::MissingRequired { contract: mine.contract });
            }
            continue;
        };
        let low = mine.min.max(theirs.min);
        let high = mine.max.min(theirs.max);
        if low > high {
            return Err(NegotiationError::NoCommonVersion {
                contract: mine.contract,
                local_min: mine.min,
                local_max: mine.max,
                remote_min: theirs.min,
                remote_max: theirs.max,
            });
        }
        if high == 0 && local_build != remote.build {
            return Err(NegotiationError::DraftBuildMismatch {
                contract: mine.contract,
                local: local_build.to_owned(),
                remote: remote.build.clone(),
            });
        }
        agreed.push(ContractVersion { contract: mine.contract, version: high });
    }
    for theirs in &remote.contracts {
        if theirs.required && !local.iter().any(|r| r.contract == theirs.contract) {
            return Err(NegotiationError::MissingRequired { contract: theirs.contract });
        }
    }
    agreed.sort_by_key(|v| v.contract);
    Ok(agreed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(contract: ContractId, min: u16, max: u16, required: bool) -> ContractRange {
        ContractRange { contract, min, max, required }
    }

    fn hello(build: &str, contracts: Vec<ContractRange>) -> Hello {
        Hello { role: PeerRole::Renderer, wire: WIRE_VERSION, build: build.into(), contracts }
    }

    #[test]
    fn contract_numbers_are_one_to_fifteen_in_order() {
        for (index, contract) in ContractId::ALL.iter().enumerate() {
            assert_eq!(contract.number() as usize, index + 1);
            assert!(contract.rfc().starts_with(&format!("docs/engine/rfcs/{:02}-", index + 1)));
        }
    }

    #[test]
    fn picks_the_highest_common_version() {
        let local = [range(ContractId::Frame, 1, 3, true)];
        let remote = hello("b", vec![range(ContractId::Frame, 2, 5, true)]);
        let agreed = negotiate("a", &local, &remote).unwrap();
        assert_eq!(agreed, vec![ContractVersion { contract: ContractId::Frame, version: 3 }]);
    }

    #[test]
    fn disjoint_ranges_fail() {
        let local = [range(ContractId::Frame, 1, 1, true)];
        let remote = hello("b", vec![range(ContractId::Frame, 2, 2, true)]);
        assert!(matches!(
            negotiate("a", &local, &remote),
            Err(NegotiationError::NoCommonVersion { contract: ContractId::Frame, .. })
        ));
    }

    #[test]
    fn drafts_need_equal_builds() {
        let local = local_ranges();
        let same = hello("abc", local_ranges());
        assert_eq!(negotiate("abc", &local, &same).unwrap().len(), 15);
        let other = hello("def", local_ranges());
        assert!(matches!(
            negotiate("abc", &local, &other),
            Err(NegotiationError::DraftBuildMismatch { .. })
        ));
    }

    #[test]
    fn optional_contracts_may_be_missing_and_required_ones_may_not() {
        let local = [range(ContractId::Frame, 1, 1, true), range(ContractId::Blender, 1, 1, false)];
        let remote = hello("b", vec![range(ContractId::Frame, 1, 1, true)]);
        assert_eq!(negotiate("a", &local, &remote).unwrap().len(), 1);
        let remote = hello("b", vec![range(ContractId::Blender, 1, 1, false)]);
        assert_eq!(
            negotiate("a", &local, &remote),
            Err(NegotiationError::MissingRequired { contract: ContractId::Frame })
        );
        let local = [range(ContractId::Frame, 1, 1, true)];
        let remote = hello("b", vec![range(ContractId::Frame, 1, 1, true), range(ContractId::Clock, 1, 1, true)]);
        assert_eq!(
            negotiate("a", &local, &remote),
            Err(NegotiationError::MissingRequired { contract: ContractId::Clock })
        );
    }

    #[test]
    fn wire_and_malformed_ranges_are_rejected() {
        let mut remote = hello("b", vec![]);
        remote.wire = WIRE_VERSION + 1;
        assert!(matches!(negotiate("a", &[], &remote), Err(NegotiationError::WireMismatch { .. })));
        let remote = hello("b", vec![range(ContractId::Frame, 2, 1, false)]);
        assert!(matches!(negotiate("a", &[], &remote), Err(NegotiationError::MalformedRange { .. })));
        let remote = hello("b", vec![range(ContractId::Frame, 1, 1, false), range(ContractId::Frame, 1, 1, false)]);
        assert!(matches!(negotiate("a", &[], &remote), Err(NegotiationError::MalformedRange { .. })));
    }
}
