# Native Studio document core

## 2026-11-04 — source and decisions

This lane ports `origin/feat/na-3d-source` at `b81b94c66` (`core/scene3d`,
`core/studio/operations.ts`, and `extensions/layers-3d`). Shipped behaviour
reference: `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`; v1.2.0 remains a required
input. The integration design update was merged at `78294c4c9`.
[Opus's Studio spec](design/surfaces/3d-studio.md) controls all presentation.
No panels or visual output are built by this crate. Its section 12 proposals
are excluded. Exploded views, place-on and soften belong to agent operations.

| Decision | Selecting rule |
| --- | --- |
| Models, lights and cameras remain ordinary timeline layers; normal property paths own transforms and animation, `d.data` owns typed content | N3 exact authored state; N6 existing extension paths; N7 id-based commands |
| Plan edits as pure Rust data and execute against the actor's scratch tree, with normal inverse capture, journal and revision checks | Single-writer kernel architecture; N7; N8 no JS waits |
| Full channel assignment retains authored handles, key identities and unknown key metadata instead of reconstructing only ease/hold | N3 correctness over imitation |
| Use native OpenCascade/XDE in a separate import helper for STEP/IGES/BREP, preserving names, hierarchy and face colors in GLB | N1 native CAD quality; N8 excludes the Electron JS/WASM host; no Blender |
| Retain selected original import bytes and archive replaced legacy layers inside the document | N4 archive, never delete; N3 exact source provenance |
| Keep navigation transient and feed light previews to kernel gestures, with host-supplied surface picks | N3 shot intent; N7 headless driving; host independence |
| Use exact stable splat depth order; reject malformed/nonfinite input instead of reproducing WebGL errors or a quantization artifact | N3 spec over Chromium quirks |
| Use Softbox, Spotlight, Bulb and Sun in model labels, retaining area/spot/point/sun on disk; use the design's failure/progress copy | Opus design ownership and N6 stored types |

`pm-scene3d` joins the existing `crates/*` workspace member glob. It reuses `pm-geom3d` for self-contained glTF decoding and primitive geometry.
An owned native OBJ/MTL/texture packer derives from that engine's importer,
with Studio's 512 MiB ceiling; it no longer exports the Chromium-only source
image/canvas metadata. Import limits are 10M triangles and 65,536 nodes.
It does not fetch model dependencies.

Kernel wiring supplies `Scene3dCommand::Apply`, the existing `Edit`/`SetData`
entrypoints and 3D channel discovery for normal property/keyframe commands.
Final scratch validation checks coupled camera planes and authored ranges.
Structural changes are ordered before channel changes in a compound edit.

Upgrade runs in memory before query publication; source files change only on
Save. A legacy container becomes an enclosing group retaining its original ID,
outer transforms/effects/masks/locks and complete archived source; children
become ordinary 3D layers. Failed conversions retain all source data and a
machine-readable issue. This is not a claim of complete old-history or renderer
integration. Those gates remain visible until verified below.

## 2026-11-04 — initial checkpoint

Targeted Linux tests: 16 schema/document tests and 9 real-kernel tests pass,
covering atomic failure, camera cross-field edits, locks/hand intent, full
channel metadata, shot tracks, duplicate key identity, exact Undo/Redo and
PMV save/reopen including Undo.

Work remains in progress: MCP dispatch/schema/readback, imported geometry
bounds adapters, animated legacy upgrade, native CAD build/real fixtures,
navigation/light/import negative tests and final full test replay.
No macOS rendering, physical input, GPUI/WebKit, signed helper or Opus visual
acceptance is claimed by this checkpoint.


## 2026-11-05 — completed document lane

This section supersedes the initial checkpoint. The Rust port covers scene
schema/defaults/validation, per-layer data and animation paths, pure operation
plans, all eleven source Studio presets, five shot tracks, environment and
material edits, camera navigation, and the surface-pick Light tool model.
`Softbox`, `Spotlight`, `Bulb`, `Sun`, Fast/Quality and failure/progress strings
come from the merged Opus spec. Section 12 proposals are not implemented.
Explode, place-on and soften have agent operations and no panel controls.

`Scene3dCommand::Apply` runs in the normal actor transaction, retaining full
channel/key metadata, hand-edit intent and locks. Ordinary AddLayer,
SetContent, property, keyframe, expression, duplicate and Undo/Redo paths
support the three registered native definitions. Unknown extension definitions
retain the host-required result. A compound edit validates the final scratch
state, including coupled camera planes, before publication. Generated keys
have collision-free identities. No main-thread JS participates.

`scene3d::prepare_geometry` takes an immutable kernel snapshot plus an
`AssetGeometry` capability on a worker. `ApplyPrepared` rejects stale snapshots,
changed composition/time and geometry-less imports. Parent inverse matrices
convert world deltas into authored local tracks. `LoadedAssets` checks the
asset metadata against the loaded model; missing geometry fails explicitly.
`scene3d::resolve_scene` uses native query evaluation for world matrices,
visibility, opacity, camera position/up and light targets. Stable depth blocks
respect ordinary 2D barriers. This is document/service output, not a renderer
qualification. Even tiny nonzero authored opacity is retained (N3).

### Imports and durable ownership

On an import worker:

1. `import::prepare` admits explicitly selected model/material/texture bytes;
   no network, sibling discovery or original-file mutation occurs.
2. `Archive::admit` verifies decoded geometry and durably retains converted
   bytes and **all** originals under immutable SHA-256 content names.
3. A sealed receipt goes to `Scene3dCommand::Import`: it stages media resources,
   metadata, model and an optional initial Studio rig as one Undo entry.
4. PMV save/reopen carries every original source reference through current and
   undone history states. Equal original/converted bytes share one media record.
   Undo never deletes the archive. Recovery uses worker-side `Archive::recover`
   and a verified `BindImport`, which adds no document revision or Undo entry.

Models: OBJ/MTL and selected textures, self-contained GLB/glTF. Malformed glTF
attributes/undrawable primitives fail instead of claiming a blank import passed.
Splats: binary little-endian PLY (Gaussian or point-cloud) and 32-byte .splat,
with covariance/color decode, 16M limit, dense-core framing, COLMAP axis
conversion and exact stable depth ordering. Original points are unchanged.

CAD: native OpenCascade/XDE STEP, IGES and BREP, through an explicit installed
helper path behind `Tessellator`. It preserves part names, hierarchy and
per-face material colors, converts millimeters/Z-up to meters/Y-up and emits
GLB. Tessellation uses source values 0.0006 of model extent and 0.25 radians.
The helper's private scratch root is canonicalized (including macOS system
symlinks). No JS/WASM library, Node or Blender is needed (N1, N8). Missing native
tools fail the explicit feature tests; they never become an optimistic pass.
Native fixtures are generated by the owned C++ target, not external spikes.

### Automatic upgrade and history

Opening upgrades known legacy 3D definitions before query publication. A scene
container becomes an enclosing group preserving its original ID, transforms,
locks, effects, masks, timing and complete archived source; models/lights/camera
become ordinary layers. Old OBJ camera/orbit/autorotation and expressions are
sampled in the **original kernel query context** at the project frame rate,
within the source's 2,398-frame budget. Animated backgrounds and missing models
keep source intact and produce the spec's failure text. No compatibility
renderer is created. A failed layer remains editable project data and draws
nothing; diagnostics are available to agents.

Every state of imported Undo/Redo history is upgraded deterministically, with
labels and cursor retained and the full original logical history archived.
Save/reopen does not rearchive or overwrite it. An unreplayable history stays
archived with a visible issue instead of publishing unsafe inverses. PM model
lone UTF-16 code units are never rounded into replacement characters: when
these prevent JSON schema conversion, known 3D layers retain exact original
records and become explicitly failed upgrades; the project still opens/saves.

The engine's synthetic corpus includes `slots`/mesh-group fields absent from
both the owner-selected source and shipped main. Empty slots are removed only
from converted data and retained in the source archive. The one nonempty
slot/group case remains an explicit failed upgrade with all input preserved.
The storage corpus test proves exact source reconstruction and exact upgraded
save/reopen; it does **not** certify that unsupported case's render output.
This boundary stays visible under N3/N4 and the design's failed-upgrade rule.

**DESIGN-TODO(opus):** approve reason copy for unsupported historical/future
fields, invalid Unicode and evaluator failures. Until then the neutral model
sentence is `This 3D layer couldn’t be upgraded.`; raw details stay in
`native3dUpgradeDiagnostic` for the Error Log. No layout or visual choice was
made in this lane.

### Agent contract and evidence

MCP `edit_3d` dispatches the source's operation schema through real kernel
transactions, revision guards, result/selection IDs and rollback. `get_3d_scene`
returns all source material/light/camera channels, evaluated tracks and
conditional upgrade issues. The other 65 frozen catalog entries and non-3D
stdio pairs are unchanged. Geometry-backed operations accept an optional host
asset capability; unavailable data is reported precisely.

Owned fixtures independently execute the selected TypeScript source for 34
schema cases, the two 3D catalog entries and scene readback. Rust tests consume
vendored outputs without Node. Generator scripts are development-only; zod
4.6.5 is pinned in the schema oracle. Source metadata and commands live beside
the fixtures. Existing frozen v1–3 fixtures remain intact.

Linux verification covers real-kernel transactions, grouping rollback,
parent-space geometry, navigation/light gestures, source-schema conformance,
legacy animation and history, model/splat archive recovery and PMV bytes.
Native OpenCascade 7.9.3 additionally verifies actual STEP/IGES/BREP geometry,
part names/colors/units, malformed input, and import/Undo/PMV reopen for each.
Final command counts and merged base are recorded in the branch handoff reply.

### Host and macOS boundaries

Other owners must connect their import worker to `Archive`/Import/BindImport,
supply loaded asset capabilities, consume resolved service data in the new
engine renderer, and bind the v1–3 V8 extension facade to the kernel commands.
General group/ungroup still requires the existing host geometry integration;
this lane does not change that command's host boundary. No built-in panels,
GPUI input routing, renderer, physical picking, export pixels or V8 facade were
implemented or claimed here; those paths are outside this lane's ownership.

Mac verification remains: build/package/sign the native CAD helper and OCCT
libraries (including license/source obligations), exercise actual system temp
symlinks and import cancellation, connect/publish verified media on the app
worker, compare Metal preview/export and camera/parent transforms against
source intent, physical trackpad/modifier/pick gestures and one-gesture Undo,
v1–3 API calls, saved real-world legacy assets, and Opus visual acceptance.
Linux passes are not evidence for AppKit/GPUI/WebKit/Metal behavior.
