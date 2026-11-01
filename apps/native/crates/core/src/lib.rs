//! `pm-mac-core` is now `pm-kernel`. This crate re-exports it unchanged so
//! existing callers keep compiling; new code depends on `pm-kernel`
//! directly (docs/native/kernel.md). Delete it once nothing uses it (N5).

pub use pm_kernel::*;
