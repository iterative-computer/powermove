//! Geometry-dependent edit kernels (docs/rust-engine.md, section 6, "Making
//! every write visible", item 4; section 26, Phase 1).
//!
//! The renderer stays the authority for the document and Undo. A kernel
//! reads the mirror (`pm-wasm`'s canonical document), computes what the
//! legacy TypeScript edit would write, and returns it as canonical patch
//! operations (contract 5): the renderer applies them to the live document
//! inside the same history transaction, so Undo is unchanged.
//!
//! A kernel never touches the mirror. It works on an [`Edit`]: a scratch copy
//! of the document with its own evaluator. Writes land in the copy as they
//! are made, so later reads see them (the legacy code reads what it wrote a
//! line earlier: a reparented parent before its children, a baked parent key
//! before the child that depends on it), and [`Edit::finish`] diffs the
//! touched compositions into the smallest operation list (`diff_ops`): leaf
//! sets where a value changed, id-keyed inserts and deletes where layers or
//! keys came or went. Leaf sets keep the identities of the live objects the
//! renderer holds (a channel's keys stay the same objects).
//!
//! # Kernels
//!
//! | Module | Ports | Kernel |
//! |---|---|---|
//! | [`pose`] | `parenting.ts` | pose and aim preservation: [`pose::reparent`], [`pose::preserve_world_pose`], [`pose::preserve_world_aim`] |
//! | [`groups`] | `layer-groups.ts` | [`groups::move_to_group`], [`groups::ungroup`] (rebase or bake), [`groups::group_placement`] |
//! | [`align`] | `extensions/inspector/align.ts`, `AlignmentStrip.svelte` | [`align::align`], [`align::distribute`] |
//! | [`anchor`] | `font-anchor.ts`, `size-anchor.ts`, `editing.ts` `runOne` | anchor capture and anchor moves with font and size anchors |
//! | [`split_text`] | `split-text.ts` | the layers a text split creates |
//! | [`comp_bounds`] | `compositions.ts` `leaveAttributes` | composition bounds for Pre-compose › Leave all attributes |
//!
//! # Time (section 5, rule 4 with amendment 1)
//!
//! Kernels echo every time they do not change (the original `f64` is copied
//! through untouched). Times they create or move are computed in ticks on the
//! composition's exact grid and written with [`pm_model::IngestRate::seconds_of`]:
//! `n / fps` for a frame start, `ticks / TICKS` off the grid ([`time`]).
//!
//! # Exactness
//!
//! Every value keeps the TypeScript operation order and JavaScript's value
//! semantics (`+` concatenates onto a stored string, `Number(v)` coerces), so
//! results equal the legacy edit bit for bit wherever the inputs are the same
//! floats. Where the legacy code throws, the kernel throws the same message.
//! The differences, all deliberate, are listed in docs/engine/pm-edit.md.
#![forbid(unsafe_code)]

pub mod align;
pub mod anchor;
pub mod channel;
pub mod comp_bounds;
pub mod groups;
pub mod pose;
pub mod split_text;
pub mod time;
pub mod work;

pub use work::{Edit, IdSource};
