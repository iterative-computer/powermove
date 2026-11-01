# Headless native CLI (Wave 2, package 18)

`engine/crates/pm-cli/native` builds package **pm-native-cli**, binary **pm-cli**.
It uses the kernel actor and the frozen MCP catalog, planners and read digests.
No Node, Chromium, GPUI, WebKit, installed profile discovery or display server is
needed for project reads, edits, validation or migration dry-runs.

The existing engine `pm-cli` render/parity/scope binary remains unchanged because
it still has consumers. This new entrypoint is an isolated nested workspace with
its own committed lockfile. Putting an independent workspace at
`apps/native/crates/cli` would conflict with the native workspace's `crates/*`
membership; joining either parent instead would require changing its lockfile,
which is outside this lane. Both parent workspaces remain valid. No new
`apps/native/crates/cli` is created. Integrating the two CLI entrypoints and any
parent-workspace lock update is later integration work; do not install both
binaries under the same name.

## Build and commands

From the repository root (the session activation script supplies Rust and the
build target directory):

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/engine
cargo +1.96.1 build --locked --manifest-path crates/pm-cli/native/Cargo.toml
cargo +1.96.1 test --locked --manifest-path crates/pm-cli/native/Cargo.toml -p pm-native-cli
cargo +1.96.1 clippy --locked --manifest-path crates/pm-cli/native/Cargo.toml -p pm-native-cli --all-targets --no-deps -- -D warnings
cargo +1.96.1 fmt --manifest-path crates/pm-cli/native/Cargo.toml -p pm-native-cli --check
```

On macOS use the existing `engine/env.sh` toolchain/FFmpeg setup. The CLI forwards
the native FFmpeg library directory as its binary's runtime search path.

| Command | Arguments | Behavior |
|---|---|---|
| `open PROJECT`, `info PROJECT` | `--args JSON` or `--args-file FILE` | Verified JSON/PMV3/PMV4 open; exact `get_project_state` digest, including its paging options. |
| `timeline PROJECT` | MCP `get_timeline` arguments | Exact shared MCP timeline digest; IDs remain authored IDs. |
| `apply PROJECT --output NEW.pmv` | MCP `apply_commands` arguments | Plan the complete batch against a scratch reader, then one guarded actor transaction and durable session SaveAs. |
| `render-frames PROJECT` | MCP `render_frames` arguments | Capture native PNGs at the frozen revision; one to five deduplicated sample times. |
| `export PROJECT --output NEW.mp4` | MCP `export_video` arguments | Wait for the service's same-executable export child; return the actual nonempty output path. ProRes destinations should end in `.mov`. |
| `migrate PROFILE --dry-run --scratch NEW_DIRECTORY` | No live-profile discovery or secret consent | Verify a private copy, then run the existing first-launch importer on the copy. |
| `validate PROJECT` | No creative-lint arguments | Container/document/media checksum verification plus layer-ID/reference checks; reports its limited scope. |

Examples (synthetic paths only):

```sh
pm-cli info /tmp/synthetic.pmv --args '{"indexOnly":true,"layerLimit":100}'
pm-cli timeline /tmp/synthetic.pmv --args '{"offset":0,"limit":100}'
pm-cli apply /tmp/synthetic.pmv --output /tmp/synthetic-edited.pmv \
  --args '{"label":"Move title","commands":[{"type":"set_property","target":"title-id","path":"position.x","value":320}]}'
pm-cli render-frames /tmp/synthetic-edited.pmv --args '{"times":[0,1],"width":720}'
pm-cli export /tmp/synthetic-edited.pmv --output /tmp/synthetic-export.mp4 \
  --args '{"format":"mp4","range":"work","audio":true,"motionBlur":true}'
pm-cli migrate /tmp/synthetic-profile --dry-run --scratch /tmp/synthetic-import-proof
```

`--args-file -` reads stdin, capped at 16 MiB. Unknown, repeated, malformed and
command-inapplicable options fail. JSON arguments must be objects and use the
existing tool schemas: no new edit vocabulary or truncating command batch.
`--expected-revision N` guards this opened actor lifetime; a new CLI process
starts a new lifetime. For a request planned by an earlier process, supply
`--expected-source-hash SHA256` from the inspected file's SHA-256 (for example,
`sha256sum` on Linux or `shasum -a 256` on macOS). Both guards can be supplied;
a changed file or mismatched guard refuses before mutation. The digest JSON is
unchanged, so the hash is an explicit CLI precondition, not an added MCP field.

Headless playhead is zero with empty ephemeral selection. A command batch may
select/address new layers through MCP's planner; source workspace/envelope,
unknown project fields, reachable embedded media and replayable history use the
existing kernel save format. Named and anonymous root compositions keep their
existing identities (`compId` and the engine's `@root:<project-id>` view).

## Output and exit codes

Stdout contains exactly one newline-terminated MCP result:

```json
{"content":[{"type":"text","text":"{\"...\":\"tool result\"}"}],"isError":false}
```

The text is serialized through `pm_mcp::success`, so project, timeline and apply
result fields match those tools, including applied durability notices. Frame
results contain the text summary `{times,width,rendered}` followed by real PNG
MCP image blocks `{type:"image",mimeType:"image/png",data:"<base64>"}`.
Export results use `{path,projectId,width,height,fps,missingMedia}`. Native child
export refuses offline originals before writing, so successful native exports
have no missing media. Errors use `pm_mcp::failure` and its structured
`{code,message,backing?}` text. Diagnostics go to stderr. Export-child protocol
lines are private inherited pipes, not public CLI stdout.

| Exit | Meaning |
|---|---|
| 0 | Completed, or a clean scoped validation report. |
| 1 | Kernel/engine/migration failure, busy/changing profile, or validation issues. |
| 2 | Usage, schema, unsupported arguments or existing destination. |
| 3 | Backend/host capability unavailable in this build. |
| 4 | Source hash, source version, actor revision or returned-frame revision conflict. |
| 5 | Direct filesystem/stdin/stdout I/O failure. |

Validation success proves only the stated structural scope. It does not claim
the still-pending MCP `check_project` creative lint, font availability, effect
activation, original-media closure or visual correctness. A paused migration
(`needs_secret_consent`) is a successful dry-run report with that explicit status;
it is not a completed migration or permission to cut over.

## Ownership and safety

Reads use `open_project` (including PMV4 media verification) and immutable actor
publications without source-side file locks. Edits open a generation-bound kernel
session, preserve the input file and require a fresh output path. Source versions
are checked after opening and before edit admission. A kernel `Transact` applies
the complete planned batch with its expected revision. The session's SaveAs uses
`replace:false`, its own lock/version policy and atomic writer; then the CLI
awaits `DurabilityBarrier` before returning. A concurrent destination creator
cannot be overwritten. Dangling symlinks count as existing destinations.

If an edit was acknowledged but saving/durability fails, shutdown leaves its
journal in a retained private scratch archive and emits
`{"recoveryJournal":"/private/generated/path"}` on stderr. Successful temporary
caches have no remaining consumer after durable save and are cleaned up. The
input is never replaced or deleted. No broad `--force` is provided.

Migration first refuses an active/unknown Electron lock, overlap, preexisting
scratch and nonregular/symlink profile members. It checks space for four source
copies plus the importer's six-GiB reserve, streams a private mode-0700 copy,
checks BLAKE3/length inventories before and after copying, and verifies the copy.
Only that copy is passed to `pm_first_launch::run`; original source fingerprints
are checked again after it returns. The copied profile, importer checkpoints,
results and `cli-dry-run.json` remain in the requested scratch on completion or
failure. File contents and secret values never enter the CLI report. No Keychain
accessor or secret-consent entrypoint is constructed. A changing source requires
a new closed snapshot and fresh scratch; failed copies are retained for review.

## Platform adapters and remaining gates

`FrameEngine` and `ExportChild` are injected worker traits behind `Backend`.
The command driver owns no Metal/AppKit types and has no JS runtime.

| Backend | Linux | macOS |
|---|---|---|
| Project/kernel/MCP operations | Production pure-Rust paths | Same paths; actor waits occur off main. |
| Migration dry-run | Real first-launch importer on copies; secret step reported | Same; this command never accesses Keychain. |
| `native` frames | Exit 3, explicit unavailable error | `pm-engine-service::Headless::with_fonts` on a worker; native `FontService` broker pumped on main. |
| `native` export | Exit 3, explicit unavailable error | `ExportRequest` → `ExportHandle` → same binary `--engine-job`; Metal/VideoToolbox/FFmpeg, original-only media and the service's existing native font broker. |
| `test-double` frames | Real synthetic PNG signal, labelled `testDouble:true` | Same, with no Metal/font initialization. |
| `test-double` export | Labelled JSON manifest, `encodedVideo:false`; requires `.test-export.json` | Same. |

The double deliberately does not draw the project or encode video. It is a CI
contract path, not visual parity evidence or a software rendering fallback.
Native PNGs use the exporter's straight-alpha conversion instead of reproducing
premultiplied Chromium canvas readback. Video defaults retain the engine's
explicit QuickTime CICP 1-13-1 output setting, exact document rational rate,
requested work/all range and audio/motion-blur choices. Odd/invalid dimensions
are left to engine validation rather than silently changing authored dimensions.
Embedded media is streamed through the kernel read capability to a private
cache; linked originals resolve relative to the project, with no proxy fallback.

**Still required outside this lane:** P2 v1–3 V8 activation, raster overrides,
project-scoped font assets, extension generation/resource pins and definitions
are not hosted by this CLI. Known project font assets fail explicitly. Existing
MCP planner limitations remain: pending transform/caption adapters and commands
needing registered effect definitions fail without partial edits. The engine
must likewise surface unsupported layers instead of emitting a success-shaped
empty picture. Kernel migration/import refusals are reported; no real-profile,
private-corpus or v1–3 extension conformance claim is made. A shared CLI dispatch
and parent lockfile integration require changes outside the owned paths.

## Decisions and selecting rules

| Decision | Rule |
|---|---|
| Keep the old engine CLI; add an isolated native entrypoint within the owned CLI directory | Prune rule/N4: active parity/render consumers remain; N7: headless actor access can ship independently. |
| Reuse MCP schemas, planners, digest builders and success/error serialization | N6/N7: one command/inspection vocabulary for agents and tools. |
| One kernel actor; complete guarded command transaction and session SaveAs | N3/N7: authored changes are atomic and inspectable, with no competing document writer. |
| Fresh outputs, input version/hash guards, durable-save barrier and retained failed-edit journals | N4: concurrent work and acknowledged edits must remain recoverable. |
| Run the actual first-launch importer on a verified private copy | N4: dry-run exercises production behavior without changing the source profile or user data. |
| No Keychain access or automatic consent during dry-run | N4 and the existing first-launch consent contract. |
| Native service/export-child traits with explicit CI doubles and visible failures | N1/N3/N7: test the control contract without pretending synthetic pixels prove quality. |
| Native font broker on main; actor/render/export work on workers | N8: no Node/Chromium, no main-thread wait for JS or kernel publication. |
| Correct straight-alpha PNGs; explicit engine video color/rate settings | N3 and quirk policy: preserve authored intent and PNG semantics, not unrecorded canvas quirks. |
| Scoped structural validation with creative-lint capability reported absent | N3: a checksum/reference pass cannot certify unavailable creative/runtime checks. |

## Verification

Linux: **30 CLI tests pass** (26 headless/process tests and 4 migration tests),
with generated synthetic projects/profiles only. They cover MCP JSON shapes,
stdin arguments, ID resolution, atomic refusal, one-step persisted Undo,
unknown/envelope/media preservation, PMV3→PMV4, source hash/version guards,
locked layers, existing/symlink/racing-safe destinations, failed-save recovery
archives, corrupt checksums, validation scope, frame timing/dimensions and stale
revisions, decodable labelled PNGs, export-child missing-output failure,
explicit Linux unavailability, actual first-launch import, private scratch,
source nonmutation, profile locks, overlap and symlink refusal.

The unchanged native suites also pass: **312 tests** across pm-kernel, pm-mcp
and pm-first-launch using the parent workspace's committed lockfile. Package
formatting and strict CLI Clippy pass; pm-kernel has an existing Linux
`provider_xattr(path)` unused-parameter warning outside this lane.

**Not run here:** the three macOS-only subprocess smoke tests (Metal frame,
text/font-broker frame and MP4/ProRes same-binary child export), native build/link
and FFmpeg rpath, alpha/color parity, original media decoding, supported-macOS
font behavior, frozen-reference pixels, and extension/resource activation. Run
these on macOS with the existing FFmpeg setup before claiming native rendering
or export verified. No AppKit/Metal/VideoToolbox result was inferred from Linux.
