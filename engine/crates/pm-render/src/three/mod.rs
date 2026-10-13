//! three.js r180's program generation, ported (amendment 11): the vendored
//! GLSL chunks ([`chunks`]) and `WebGLProgram`'s prefix and define assembly
//! ([`program`]). The output is byte for byte the source WebGL receives
//! (`tests/three_programs.rs`); it compiles through `pm_gpu::glsl`.

#[rustfmt::skip]
pub mod chunks;
pub mod math;
pub mod program;
