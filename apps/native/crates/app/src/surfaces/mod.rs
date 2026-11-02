//! Surface adapters are opt-in until their owning application controllers attach them.
//! N4/prune: existing controller consumers remain until the new host is attached.
#[allow(dead_code)]
pub mod recovery;
#[allow(dead_code)]
pub mod recovery_kernel;
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub mod recovery_macos;
#[allow(dead_code)]
pub mod recovery_native;
#[allow(dead_code)]
pub mod recovery_wire;
