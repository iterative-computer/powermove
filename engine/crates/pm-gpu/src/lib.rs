//! wgpu device, pools, shader compilation, interop, time slicing (section 8)
//!
//! See docs/rust-engine.md.
//!
//! This crate currently holds the decode-in half of the interop trait
//! (contract 3): [`decode_in`] imports decoded frames from `pm-media` into
//! wgpu without copies and runs the section 13 normalization.

pub mod decode_in;
