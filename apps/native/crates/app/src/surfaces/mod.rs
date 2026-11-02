//! App-owned secondary surfaces. Platform adapters never execute JavaScript.
#[allow(dead_code)] // Entry-point dispatch belongs to the shell integration lane.
pub mod authority;

//! Secondary media surfaces. Publication and effect execution are host-owned.
// These public entry points are awaiting the app integration lane (see surfaces.md).
#[allow(dead_code)]
pub mod media;
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub mod media_appkit;
#[allow(dead_code)]
pub mod media_kernel;
#[allow(dead_code)]
pub mod media_wire;

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
