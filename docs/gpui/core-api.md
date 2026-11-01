# `pm-mac-core`: the API panels code against

Crate: `apps/mac/crates/core` (package `pm-mac-core`). Pure Rust, no GPUI.
It is the single owner of the open project: every panel, the MCP server and
extensions read and change the document only through it
([engine-boundary.md](engine-boundary.md), sections 1, 2.2, 5.2, 6).
The crate-root docs (`src/lib.rs`) are the authoritative reference; this
page is the short version.

## Lifecycle

```rust
use pm_mac_core::{DocumentStore, Command, Origin, SaveTarget, SaveOptions, open_project, OpenOptions};

// Open: blocking I/O, run on a background executor.
let opened = open_project(&path, OpenOptions::default())?;   // worker thread
let mut store = DocumentStore::open(opened);                  // main thread, cheap

// Edit: one command = one revision = one undo step.
store.execute(Command::SetStaticOpacity { layer_id: id.clone(), value: 40.0 }, Origin::Interface)?;
store.undo()?;   // a NEW, higher revision restoring the old value
store.redo()?;

// Save: freeze on main, write on a worker, adopt on main.
let plan = store.prepare_save(SaveTarget::InPlace, SaveOptions::default())?;
let receipt = plan.run()?;                                    // worker thread
store.finish_save(receipt)?;                                  // clears dirty for THAT revision only
```

`DocumentStore` is `Send`, owned by the window's thread (wrap it in a GPUI
entity). `Snapshot` is `Send + Sync`: give it to render, audio and export
threads. `SavePlan::run` and `open_project` are the only blocking calls.

## Reading

| Need | Call |
|---|---|
| Immutable document at a revision | `store.snapshot()` → `Snapshot { doc_id, revision, document: Arc<Document> }` |
| Layers panel rows (open comp) | `store.layers()` / `snapshot.layers(Some(comp_id))` → `Vec<LayerInfo>` (id, name, kind, parent, group, in/out, visible, locked, effectively_locked, solo, shy, index) |
| Evaluated value at a time | `store.queries().evaluate(None, layer_id, "opacity", t)` |
| Several channels | `store.queries().values(None, layer_id, &["position.x", "position.y"], t)` |
| World/local matrix, world opacity | `world_matrix`, `local_matrix`, `world_opacity` |
| Bounds | `bounds` / `group_bounds` → `Bounds` in the layer's **own plane** (map with `world_matrix`) |
| Hit test | `pick(None, x, y, t, &PickOptions)` → `PickHit { layer_id, revision }` (already bound to a stable id) |
| Anything else in pm-wasm | `queries().query(&json_request)` (raw `{op, comp, layer, t, ...}`) |
| Catch up after lagging | `store.changes_since(rev)` → `Some(Vec<Arc<Change>>)` or `None` (take a snapshot) |

Queries can answer `NeedsFonts`, `NeedsInk`, `NeedsModels(ids)`, `NeedsHost`
or `Unsupported` instead of a value. These never mean "no layer": load the
inputs (`queries().engine_mut()`), then ask again.

## Changing

Only typed `Command`s, addressed by **id** (layers, keyframes `i`,
effects, masks, markers). One `Command` type wraps a domain enum per area;
each module in `src/commands/` owns its enum, docs and planning code, and
`From` impls let you write `store.execute(LayerCommand::Delete { .. }.into(), origin)`.
The rustdoc on each variant is the contract: parameters, validation, undo
label, and the old function it ports.

Conventions: times are composition seconds unless a field says otherwise
(key times are layer-local); commands that depend on the playhead take it
as `time` (the core has no playhead); ids of things a command creates are
passed in, made with `pm_mac_core::new_id(prefix)` (`L` layers, `k` keys,
`f` effects, `K` masks, `C` comps, `marker`), so the caller can select what
it made; channels are the old keys (`opacity`, `position.x`, `c.size`,
`<fxId>.<param>`, `<fxId>.$enabled`, `m.<maskId>.<param>`, `l.blend`, ...).

### Catalog and status

Status: **done** (ported, tested), **todo** (typed and documented, returns
`CommandError::NotImplemented`; nothing changes).

| Command | Old app | Status |
|---|---|---|
| **Spine** (kept for existing callers) | | |
| `SetStaticOpacity`, `SetLayerName`, `SetLayerVisible`, `SetLayerLocked` | `set_property` static, `set_layer` | done |
| **Layers** `LayerCommand` | | |
| `Set { layer_id, patch: LayerPatch }` | `set_layer` (name, from, duration, visible, locked, shy, solo, blend, motionBlur, parent, matteSource, color, collapsed, threeD, scaleLinked, separateDimensions, continuousRaster, effectSpace) | todo |
| `Add(AddLayer)` | `add_layer`, `PM.mkLayer` | todo |
| `Delete { layer_ids }` | `delete_layers`, `PM.removeLayers` | todo |
| `Duplicate { layer_ids, new_ids }` | Cmd+D, `PM.cloneLayer` | todo |
| `Paste { comp, layers, new_ids, index, time }` | Cmd+V layers | todo |
| `Reorder { layer_id, index }` | `reorder_layer` | todo |
| `Restack { layer_ids, to }` | Cmd+[ ] (Shift: front/back) | todo |
| `TransformLayers` | `transform_layers` (generated tools) | todo |
| **Properties** `PropertyCommand` | | |
| `Set(SetProperty)` | `set_property` (keys at `time` when animated) | done |
| `SetExpression` | `set_expression` | done |
| `ToggleStopwatch` | stopwatch, `PM.toggleStopwatch` | done |
| `Reset` | channel-row Reset | todo |
| **Keyframes** `KeyframeCommand` | | |
| `Replace(ReplaceKeyframes)` | `replace_keyframes` | done |
| `SetEasing { key_ids, curve }` | `set_easing`, easing panel apply | done |
| `Simplify { scope }` | `simplify_keyframes` | todo |
| `Add`, `Toggle` | Add Keyframe Here, diamond | done |
| `Delete { key_ids }` | Delete on keys, `PM.removeKey` | done |
| `ClearAll` | Clear All Keyframes / Remove Animation | done |
| `Move { key_ids, delta }` | key drag, Option+←/→ | done |
| `Edit { key_id, time, value }` | keyframe value dialog | done |
| `Paste` | Paste Keyframes Here (one destination track) | done |
| `EasyEase`, `SetInterpolation`, `SetHold` | F9 family, interpolation menu, hold | done |
| `SetHandle` | graph editor handles (ease or point) | done |
| `SetSpring` | spring easing | done |
| `Transform` | graph transform box | todo |
| **Timing** `TimingCommand` | | |
| `Move { layer_ids, delta }` | strip drag (group strips move members) | todo |
| `TrimIn`, `TrimOut` | edge drags, Option+[ ] | todo |
| `MoveInTo`, `MoveOutTo` | [ ], Option+Home/End | todo |
| `Slip` | source slip | todo |
| `Split { layer_ids, time, new_ids }` | Cmd+Shift+D / Cmd+B | todo |
| `Stagger { layer_ids, step }` | Cmd+Option drag | todo |
| **Groups** `GroupCommand` | | |
| `Group`, `Ungroup`, `MoveToGroup` | `group_layers`, `ungroup_layers`, `move_to_group` | todo |
| **Compositions** `CompCommand` | | |
| `Set { comp, patch: CompPatch }` | `set_composition` | todo |
| `SetWorkArea`, `ResetWorkArea` | B, N, ruler menu | todo |
| `SetSceneParameter` | `set_scene_parameter` | todo |
| `Create`, `Duplicate`, `Delete` | New/Duplicate/Delete Composition | todo |
| `Precompose` | Cmd+Shift+C | todo |
| **Effects** `EffectCommand` | | |
| `Add(AddEffect)` | `add_effect`, effect paste | todo |
| `Remove`, `Set { patch: EffectPatch }` | `remove_effect`, `set_effect` (toggle, open, reorder, inputs) | todo |
| `Duplicate`, `Reset` | effect stack menu | todo |
| `SetTransition` | `set_transition` | todo |
| **Masks and paths** `MaskCommand` | | |
| `Add`, `Remove`, `Set`, `Reorder` | mask rows, `PM.mkMask` | todo |
| `SetPath` | pen tool results | todo |
| **Text** `TextCommand` | | |
| `SetText`, `SetBox` | in-place editor commit, paragraph resize | todo |
| `Split`, `AddAnimator`, `RemoveAnimator`, `SetStyleRange` | Split Text, text animators, style ranges | todo |
| **Content** `ContentCommand::Set` | `set_content` | todo |
| **Markers** `MarkerCommand` | | |
| `Add`, `Remove` (index), `RemoveById`, `Move`, `Rename` | `add_marker`, `remove_marker`, ruler menu | todo |
| **Captions** `CaptionCommand` | | |
| `Add(AddCaptions)`, `Edit { edit: CaptionEdit }` | `add_captions`, `edit_captions` | todo |
| **3D** `Scene3dCommand` | | |
| `SetData`, `Edit` | 3D `set_content`, MCP `edit_3d` | todo |
| **Media** `MediaCommand` | | |
| `Replace`, `AddAsset`, `RemoveAsset`, `RenameAsset`, `SetAssetFolder` | replace media, library | todo |
| **Library** `LibraryCommand` | | |
| `CreateSection`, `UpdateSection` | `create_section`, `update_section` | todo |
| **Transform** `TransformCommand` | | |
| `Nudge` | arrow keys | todo |
| `CenterAnchor`, `Align`, `Distribute` | Cmd+Option+Home, alignment strip | todo |

Copy is not an edit (the clipboard lane serializes `snapshot()` layers);
Cut is Copy + `LayerCommand::Delete`; Lock All / Unlock All is a
`transact` of `LayerCommand::Set` per layer.

* `execute(cmd, origin)`: one step. `Ok(None)` means nothing changed.
* `transact(label, origin, &[cmds])`: atomic; later commands see earlier ones.
* `transact_with(&cmds, TransactOptions { expected_revision, .. })`: refuses
  with `Stale` if the document moved (agents/MCP).
* Drags: `begin_gesture(label, origin)` → `update_gesture(id, &[cmds])` per
  pointer move (each publishes a live revision) → `commit_gesture(id)` (one
  undo step from start to end values) or `cancel_gesture(id)` (restores the
  start). While a gesture is open, other edits, undo and redo are refused.
* `restore_project(value, label, origin)`: replace the whole project as one
  step, written as minimal id-keyed operations (checkpoints, agent rollback).
* `history_mark()` + `squash_since(&mark, label)`: an agent run as one step.

Every change is staged on a scratch tree, validated as a whole, and
published only if everything succeeded. On any `Err`, revision, document,
history, selection and dirty state are exactly as before.

New commands are added by the core owner only (see `src/commands/mod.rs`).

## Observing

`store.subscribe(|event| ...)` is called synchronously, after the store and
its query mirror are updated, on the owning thread:

* `Changed(Arc<Change>)`: `{ base_revision, revision, ops, cause }`.
  `cause` is `Edit`, `Undo`, `Redo`, `GestureStep`, `GestureCancel` or
  `Restore`. Feed `ops` to incremental readers (engine `DocumentUpdate::Ops`).
* `SelectionChanged`, `HistoryChanged`, `DirtyChanged(bool)`, `Saved { revision }`.

A listener cannot call back into the store; GPUI wrappers just `cx.notify()`.

## Selection

`select_layers(ids, add)`, `toggle_layer(id)`, `select_keys(ids)` (keyframe
ids, the stored `i`), `set_selected_channel(chan)`, `clear_selection()`,
`selection()`. Ids not in the open composition are ignored; after every
change, deleted layers and keys are pruned. `primary()` is the first
selected layer (`PM.firstSel()`). Selection is never saved and never undone.

## Files

* Open: JSON, PMV3, PMV4. The whole file is hashed and every PMV4 media
  checksum verified in one pass; the envelope (`history`, `ws`, unknown
  members), all media ranges and the saved undo history are kept.
* Save always writes PMV4 in the old writer's layout: an in-place save of a
  PMV4 file clones it and appends the new document (unchanged media costs no
  I/O); a first save, Save As, or too much dead data writes a compact file.
  Golden files from the old TypeScript writer are reproduced byte for byte
  (`tests/ts_writer_parity.rs`).
* Durability: sibling temp file, `F_FULLFSYNC`, read-back verification,
  atomic exchange with verification of the displaced file, directory sync.
  A failed save leaves the previous file byte-identical and the store dirty.
  A file changed by someone else is never overwritten (`ChangedOnDisk`);
  `SaveAs { replace: false }` never overwrites anything.
* Only media the project or its undo history reaches is written (as before).
* Optional backups: `SaveOptions { backups: Some(BackupPolicy::new(dir)) }`.

## Differences from the old app (deliberate or not yet ported)

* **Dirty** follows the undo position: undoing back to the saved state is
  clean; re-typing an old value by hand is not (the old app compared content
  fingerprints). Workspace changes never dirty the project (as before).
* **History**: selection is not an undo step; `external` (closure) entries,
  nested `begin`/`commit`, takes, and `preserveHandEdits`/`markIntent`
  per-command flags are not ported. Imported and saved history uses the old
  format (`{ version: 1, paths: "composition", index, entries }`), so files
  move between apps with their undo stack.
* **Commands**: opacity outside 0–100 is refused (the old command accepted
  it); animated opacity is refused instead of keyed at the playhead.
* **Writer**: document keys are in canonical order (not insertion order);
  the registry journal, macOS Versions snapshots and the renderer upload
  protocol are the shell lane's or gone.
* **Queries**: no per-revision/time answer cache yet; typed wrappers for
  `textLayout`, 3D picks and `edit.*` kernels, and the interned
  `ev_id`/`ev_batch` fast path for drags are TODO (use `query()` meanwhile).
* **Not in this crate yet**: media catalog/asset resolution for render and
  audio, composition switching, autosave/recovery journal, keymap/settings.
