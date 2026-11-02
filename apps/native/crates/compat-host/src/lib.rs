//! Mac rendering hosts for V8-owned DOM panels. No extension logic runs here.
//! See docs/native/compat-host.md for the shell and remote-DOM integration contract.
mod contract;
mod remote_dom;
mod transport;
pub use contract::*;
pub use remote_dom::*;
pub use transport::*;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{MacDomHost, StoreDataStores};

pub const PAGE_APPLIER: &str = include_str!("applier.js");
