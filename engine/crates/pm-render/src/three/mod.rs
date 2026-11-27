//! three.js r180's program generation, ported (amendment 11): the vendored
//! GLSL chunks ([`chunks`]), `WebGLProgram`'s prefix and define assembly
//! ([`program`]), `WebGLPrograms.getParameters` for the materials the 3D
//! pass draws ([`materials`]), three.js's math ([`math`]), and the disk
//! cache of translated programs ([`cache`]). Program text is byte for byte
//! the source WebGL receives (`tests/three_programs.rs`); it compiles
//! through `pm_gpu::glsl`.

#[rustfmt::skip]
pub mod chunks;
pub mod cache;
pub mod materials;
pub mod math;
pub mod program;
pub mod specialize;
