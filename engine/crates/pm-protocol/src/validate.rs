//! Contract invariants that types alone cannot express.
//!
//! Decoding checks structure and safe-integer ranges; [`Validate`] checks the
//! rest (limits, cross-field rules, references). [`crate::codec::decode`]
//! runs both, so a message that decodes is a message that obeys its contract.

use crate::version::ContractId;

/// A broken invariant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("contract {} ({contract:?}): {message}", contract.number())]
pub struct Violation {
    pub contract: ContractId,
    pub message: String,
}

impl Violation {
    pub fn new(contract: ContractId, message: impl Into<String>) -> Self {
        Self {
            contract,
            message: message.into(),
        }
    }
}

/// Checks a value's contract invariants.
pub trait Validate {
    fn validate(&self) -> Result<(), Violation>;
}

impl<T: Validate> Validate for Vec<T> {
    fn validate(&self) -> Result<(), Violation> {
        self.iter().try_for_each(Validate::validate)
    }
}

impl<T: Validate> Validate for Option<T> {
    fn validate(&self) -> Result<(), Violation> {
        self.as_ref().map_or(Ok(()), Validate::validate)
    }
}

/// Returns a violation unless `condition` holds.
pub(crate) fn ensure(
    condition: bool,
    contract: ContractId,
    message: impl FnOnce() -> String,
) -> Result<(), Violation> {
    if condition {
        Ok(())
    } else {
        Err(Violation::new(contract, message()))
    }
}

/// Fails when ids repeat.
pub(crate) fn ensure_unique<T: Ord + core::fmt::Debug>(
    items: impl IntoIterator<Item = T>,
    contract: ContractId,
    what: &str,
) -> Result<(), Violation> {
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let description = format!("{item:?}");
        if !seen.insert(item) {
            return Err(Violation::new(
                contract,
                format!("duplicate {what} {description}"),
            ));
        }
    }
    Ok(())
}
