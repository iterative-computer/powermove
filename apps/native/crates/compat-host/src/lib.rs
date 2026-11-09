//! Mac rendering hosts for V8-owned DOM panels. No extension logic runs here.
//! See docs/native/compat-host.md for the shell and remote-DOM integration contract.
mod canvas;
mod capture;
pub use capture::CaptureTicket;
mod contract;
pub use canvas::CanvasFrame;
mod remote_dom;
mod transport;
pub use contract::*;
pub use remote_dom::*;
pub use transport::*;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{MacDomHost, StoreDataStores};
#[cfg(target_os = "macos")]
pub mod visual;
