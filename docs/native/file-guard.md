# Native project file guard

## Source audit and decision — 2026-11-04

The owner's going-back decision replaces the earlier reverse-migration/Restore
proposals: there is no reverse migration. Preserve original files and archives.
This lane changes only the native external-PMV writer and this proposal. It does
not patch or merge `main` or `feat/tr-prep`, and authorizes no release.

**Release blocker:** neither pinned Electron reader has a capability guard.
A metadata marker cannot make those already-shipped binaries refuse a file.
The writer below provides a transition-release contract, not retroactive
protection. Do not advertise native-only files as safe to open in those builds.
A new PMV magic/index version would force a generic parse/index failure, but
also requires a production native-reader change outside this lane's ownership;
writing a file our own reader cannot reopen is not an acceptable workaround.

The references are `v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`)
and the owner-pinned `main` (`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`).
The pinned main commit is also the `v1.3.2` release tag. The intervening
`v1.3.0` (`3a4edb1f912eb2ab05af821406ac1181d35dee1e`) and `v1.3.1`
(`5458b544927c6633490d916132d1fbec6114695a`) tags have byte-identical
container/index readers and shape validator, and the same decisive hydration
statements. The pinned-main column below applies separately to **v1.3.0,
v1.3.1 and v1.3.2**. These are source-level release-tag findings; actual signed
binary behavior still needs macOS verification. Earlier releases than the
requested v1.2.0 baseline were not audited. The baseline integration head also contains
`shared/reader-version.ts` / `engine-stamp.ts`, but **neither file exists in
these four release references**. Their presence on native-engine is not shipped proof.

| Input or operation | v1.2.0 | Pinned main |
| --- | --- | --- |
| PMV4 index `version != 4`, with valid checksums | Throws `The project index is invalid.` before document admission/association. | Same. |
| Unknown magic, e.g. `PMV5\n` | Falls through to JSON parsing; binary PMV5 fails with a `SyntaxError` (engine wording varies). No explicit unsupported-version diagnosis. | Same. |
| PMV3 header contains an unknown `version` member | No version-member check; PMV3 is selected by exact magic and validates document/media ranges. Extra header members are ignored. | Same. |
| Envelope `v` is a future app-version string | Not a container version or a reader gate; admitted if project shape passes. | Same. |
| Unknown envelope field (`requiresEngine`, `minReaderVersion` included) | Ignored by shape validation and open; save rebuilds `{v,proj,history,ws}`, dropping the envelope field. | Same. |
| Unknown project-root field | Accepted; hydration's `Object.assign(base,p)` preserves it and ordinary save serializes the project. Preservation does not mean its semantics are understood. | Same. |
| Unknown non-null layer `type` absent from `PM.TYPE_META` | Hydration changes `L.type` to `null`, builds the null-layer defaults, and deletes `L.p` keys outside that schema. Open succeeds; later save can persist the changed type/channels. | Same. |
| Known `extension` layer with an unavailable definition | Not the unknown-layer-type case. Generic extension data is retained/sanitized; definition/render availability is a separate runtime dependency. | Same. |
| Studio data newer than its strict 3D schema | No file guard; generic extension hydration can accept the file while later 3D parse/render/edit cannot represent it. No whole-file safe refusal guarantee. | Same, for the post-main Studio schema changes. |

Evidence paths (all relative to `apps/desktop/src` at the reference commit):

- `shared/project-incremental.ts`: `readProjectFooter`, `validateProjectIndex`,
  `readIncrementalIndex`, `decodeIncrementalProject` (identical at all four release refs).
- `shared/project-container.ts`: exact magic dispatch, JSON fallback,
  `projectContainerIndex` (identical at all four release refs).
- `main/project-document-validator.ts:7`: shape-only `assertProjectDocument`
  (same at all four release refs, including its worker-captured source).
- `main/project-files.ts`: `open`, validation before association publication;
  `main/save.ts`: `openProjectForWindow` returns `{ok:false,error}` on failure.
- `renderer/src/legacy/app.ts`: `hydrateSteps` at tag line 215 / main line 232;
  unknown-type coercion at tag 332 / main 350; channel deletion at tag 346 /
  main 364; `openProjectFile`, `saveProject` and library/boot open paths.
- `renderer/src/legacy/core/model.ts`: the identical 13-type `TYPE_META` roster.
- `renderer/src/core/scene3d/{schema,layers}.ts`: strict 3D schemas and
  definition IDs; Studio additions compared against read-only source branch
  `feat/na-3d-source` at `b81b94c663d2d4b380ba14a322fa2f6b14b769df`.

## Writer contract — 2026-11-04

The smallest requirement is one existing field, at the **save envelope** root:

```json
{"v":"1.0.0","proj":{},"history":null,"ws":null,"requiresEngine":"native"}
```

The empty project above illustrates field placement only, not a valid project.
Container magic, PMV4 index version, media ranges/checksums and authored project
fields remain intact. `v` stays the existing writer value; it is not repurposed.
The native production reader preserves the envelope field and reopens the file.
Ordinary files retain the existing byte-for-byte TypeScript writer golden layout.

`project_io/save.rs` computes the requirement on the save worker from its frozen
project **and the exact history being written**. Its sibling `file_guard.rs`
scans only project/composition, asset and layer structure. Opaque extension JSON,
workspace, notes, edit logs and arbitrary future metadata do not trigger it.
Known v1–3 `extension` definitions and legacy 3D are not blanket-blocked.
The current predicate roster is:

- Non-null layer types outside both readers' 13 built-in types (including
  invalid non-string types that the old hydrator would coerce).
- Linked-media/interpretation asset declarations (`storage:"linked"`, non-null
  `link` or `interpret`); nondefault linear blending or `displayEncoded` blend
  domain; audio/video `d.pitch:"follow"`; precomp `d.keepFrameRate:true`.
- Actual post-main Studio schema additions on the four built-in 3D definition
  IDs: rounded boxes; new material channels or `screen`; area/width/height
  lights; lens `fStop`/`focusDistance`; environment `shadowCatcher`/`samples`.
  An explicit default still counts because the old schema is strict. Ordinary
  2D/legacy 3D content with no such fields does not count.
- Existing root engine/minimum-reader declarations, including declarations
  found only in history.
- Any reachable undo **or redo** state needing those features. Replay joins
  partial leaf patches to their owning layer/composition using kernel history
  semantics, including imported dictionary-packed and root-relative histories.
  Unimportable, trimmed or unreplayable raw history also requires a newer
  reader: preserving the bytes does not certify an old build can retain Undo.

Only the known writer-owned envelope value `"native"` is recomputed/removed
when no requirement remains, including history. Other envelope requirement
values and **all** project-root requirements stay untouched. This means an
explicit foreign/root requirement can intentionally keep an otherwise plain
file guarded. Source snapshots, document revisions, history cursor, and unknown
fields are never rewritten by the stamp calculation. Save As, append and
compaction share this path; existing atomic save/verification rules still apply.

Every future native-only persisted feature must add a predicate and an old-reader
witness before shipping. This roster is not a claim of complete future feature
coverage. A writer-only predicate also cannot certify a restored Electron
profile/library: direct library persistence, migration/recovery writers and
archives are separate integration boundaries outside this lane's ownership.

| Decision | Selecting rule |
| --- | --- |
| Use one existing requirement field, conditional on content/history, keeping PMV4 and ordinary bytes compatible | N4 every old project remains readable; N3 exact authored intent; smallest contract. |
| Do not reproduce unknown-type coercion or strip new data to appease the old reader | N3 correctness/quirk policy; N4 no data loss. |
| Inspect frozen worker state, never mutate the live document or call JS in production | Kernel single writer; N7 headless/id-driven; N8 no Node/Chromium or main-thread JS waits. |
| Preserve foreign requirements and opaque extension-owned JSON; do not block standard extension definitions | N4 preservation and N6 v1–3 contract. |
| Guard reachable Undo/Redo and retain raw unimportable history | N4 user work includes history; N3 no optimistic capability claims. |
| Report shipped-reader ignorance as a release blocker, not a successful refusal | N1 pro-grade; N2 strictly better; N3 correctness and honesty. |
| Keep all old readers/writers and archives; no reverse migration added | Owner going-back decision; N4 archive-never-delete; prune only after no consumer remains. |

## Transition-release patch proposal — 2026-11-04

**Proposal only.** Apply to a reviewed transition release on its own branch.
The following capture-free function is the executable part of the proposal;
the native tests run it directly from this document against native PMV4 output.
The strings are machine error codes, neutral placeholders rather than approved
user-facing copy. `DESIGN-TODO(opus)`: approve the refusal message and its
presentation in existing open/error surfaces; explain that the file is intact
and which compatible app can open it. No new UI is implemented here.

<!-- transition-guard:start -->
```js
export function assertLegacyReadable(document) {
  const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
  const holders = record(document) ? [document, document.proj].filter(record) : [];
  for (const holder of holders) {
    if (Object.hasOwn(holder, 'requiresEngine') && holder.requiresEngine != null) {
      const error = new Error('PMV_REQUIRES_NATIVE');
      error.code = 'PMV_REQUIRES_NATIVE';
      throw error;
    }
    if (Object.hasOwn(holder, 'minReaderVersion') && holder.minReaderVersion != null
        && (typeof holder.minReaderVersion !== 'number' || !Number.isFinite(holder.minReaderVersion)
          || holder.minReaderVersion > 1)) {
      const error = new Error('PMV_READER_TOO_OLD');
      error.code = 'PMV_READER_TOO_OLD';
      throw error;
    }
  }
}
```
<!-- transition-guard:end -->

Patch attachment points and required ordering:

1. Add the function to a shared module. Invoke it at the **start** of
   `assertProjectDocument`; include it in the validation worker source alongside
   that capture-free validator. Marker errors are document failures, never
   `ProjectDocumentWorkerError` (no fallback that bypasses the guard).
2. In `ProjectFiles.open`, enforce it before publishing an association/read
   capability or persisting any registry changes. Both inline and >64 MiB
   worker validation must make the same decision, after container checksums.
3. For buffer/Blob/browser imports and native document-range opens, invoke it
   immediately after decoding, **before** `restoreProjectFileStream/Media`,
   hydration, ID/name changes, thread copying, outgoing-session capture,
   registry insertion, workspace restore or any autosave/dirty notification.
   Close every read token/handle on refusal. Include drag/drop, IPC/CLI opens,
   recovered-copy opens and headless document hosts.
4. For boot/autosave/library/tabs and `openProjectHere`, check the raw envelope
   before unwrap and check the raw project before hydrate; library serialization
   must preserve requirement metadata too. Do not show an editable/read-only
   inaccurate rendering. Refuse the incoming file/project and retain the
   previous active document and its recovery state.
5. Recheck requirements at every save/replace entry point in both processes,
   including direct `projectSave` IPC and streamed completion. Never strip the
   marker to permit a save. Preserve unknown requirement values; do not let the
   pre-existing engine-stamp remover clear the native file requirement.
6. For a newly unknown layer type without a writer marker, refuse before
   hydration using the active registered type roster; do not convert it to
   `null`. Keep arbitrary `extension` definitions distinct from layer types.
7. Convert unknown container versions into an explicit unsupported-format error
   before JSON fallback; distinguish corruption/checksum errors. Do not catch
   these to recover a partially readable project or overwrite the source.

Transition acceptance still required: main/worker/browser/boot/library/headless
and IPC save paths, old and new 3D, failure handle cleanup, unchanged file bytes,
unchanged previous active document, no media/store/registry/thread/workspace/
autosave side effects, and Opus-approved macOS error presentation. The tested
function alone is not that integrated patch and is not a release gate pass.

## Verification — 2026-11-04

The sibling module's tests vendor the extracted PMV4 reader and shape validator
from all four immutable release references (identical source), plus the decisive hydration
fragments. They execute in Node 22.18+ / Bun as **development-only** oracles;
no JS runtime is linked into the app. No Git objects, desktop checkout, spikes,
private corpus or outside-lane fixtures are needed at test time. The oracle
checks real native files, including recomputed container hashes when probing
unknown versions. It asserts the shipped acceptance/loss and transition refusal
separately; acceptance by a shipped reader is intentionally **not** a guard pass.

Linux tests cannot prove AppKit dialogs, WebKit/GPUI state, LaunchServices,
APFS cloning/F_FULLFSYNC, signed shipped binaries or physical crash durability.
Run the integrated transition patch on macOS with both actual shipped builds,
synthetic private copies and an isolated profile before claiming safe refusal.

Vendored source provenance (SHA-256 of the complete original file, including
its import; equal at all four release tags):

| Source under `apps/desktop/src` | SHA-256 |
| --- | --- |
| `shared/project-incremental.ts` | `5ebfa3ef44dbd761eb826d470cf020ce8ee44699ee2a6c8630f7cb7aa488e233` |
| `shared/project-container.ts` | `38de97465e2e76604701ad7bfe120926f1756de6b1381de8fb71715f21545a88` |
| `main/project-document-validator.ts` | `ace7716ba88002dd65b9501c387586b1e3bd5fc03f0f4e05c8d11e91a713ecae` |
