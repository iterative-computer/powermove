# Extension compatibility checks

**Gate P2 is OPEN.** This harness inventories sources, runs real V8 activation
probes, and defines a paired trace/PNG/interaction comparison protocol. The
bundled native adapter currently stops after activation/disposal. It does not
yet drive WebKit, replay the interaction script, or capture native traces/PNGs.
It reports `cant_run` until those adapters exist. Unit tests and successful
activation are never extension matches.

The continuation adds a disposable native kernel adapter, V8 contribution
mounting, a production WebKit remote-DOM adapter, and profile-scoped Store
admission/data-store components. The latter two are not yet integrated into the
capture CLI. See `docs/native/extcheck-p2.md` for their limits and current counts.

## Prepare the frozen reference

Use an owned, disposable checkout; never change the installed app or its profile.
Keep at least 6.5 GiB free. This lane's revised resource policy pauses below
6.5 GiB and resumes at or above 6.5 GiB. The recorder must be enabled
**during the build**.

```sh
mkdir -p /tmp/pm-extcheck
git worktree add --detach /tmp/pm-extcheck/reference v1.2.0
bun tools/trace-recorder/apply.ts /tmp/pm-extcheck/reference
(cd /tmp/pm-extcheck/reference && bun install --frozen-lockfile)
(cd /tmp/pm-extcheck/reference && POWERMOVE_TRACE_RECORDER=1 bun run --cwd apps/desktop build)
bun install --frozen-lockfile --cwd apps/native/js/compat
# Remote DOM's lockfile v2 needs a supporting Bun; on Bun 1.3.14 use:
(cd apps/native/js/remote-dom && npm install --ignore-scripts --no-package-lock)
bash tools/extcheck/build.sh test
bash tools/extcheck/build.sh test pm-compat-host
bash tools/extcheck/build.sh test pm-store-host
bash tools/extcheck/build.sh build
bash tools/extcheck/run.sh \
  --reference-root /tmp/pm-extcheck/reference \
  --native-runner "$HOME/Developer/pm-target/debug/pm-compat-runner" \
  --store --summary /tmp/pm-extcheck/summary.json
```

`build.sh` sources `engine/env.sh`, uses Rust 1.96.1, the shared target, the
build lock, the verified S1 archive **and bindings**, and monitors disk during
builds. Cargo runs at `nice -n 20` with four build jobs and four test threads.
It uses debug=0 to avoid a second dependency build with line tables.
Runner tests first build the actual `pm-store-helper` with the same sandbox
feature and verified archive; no substitute helper is used.
Set `PM_S1_ARTIFACT` to another verified copy if needed. No V8 source build.

The native runner is a workspace package, `pm-compat-runner`. Its tests
execute the production compat bundle inside `pm-runtime-v8`, including actual
remote-DOM event closures and native recorder pixels. Bun is a build/test tool;
the executable does not load Node, Electron or a JS test double.

The standalone WebKit component probe uses the production page applier and a
bounded routing worker. It opens and closes its own synthetic window, and must
run only after the owner's pause ends, with the shared GUI lease:

```sh
bun build apps/native/js/remote-dom/src/browser-applier.ts --target browser \
  --format iife --outfile /tmp/pm-extcheck/page.js
bash tools/extcheck/build.sh build pm-compat-host remote-dom-smoke
python3 tools/extcheck/gui_capture.py \
  "$HOME/Developer/pm-target/debug/examples/remote-dom-smoke" \
  /tmp/pm-extcheck/page.js
```

Its DOM acknowledgement is component evidence only. It does not capture an
extension oracle or declare a P2 match.

## Inputs and privacy

Default inputs are immediate manifest-bearing children of the owner's installed
extension root and `apps/desktop/e2e/fixtures/compat-corpus`. Override with
`--owner-root` and `--corpus-root`. These are separate source populations; they
are not deduplicated across populations. Nested manifests are not separate
installations. Enabled state, projects, variables, credentials, storage, and
Store provenance are not copied from the owner's profile.

Inputs are copied into mode-0700 run directories under `/tmp/pm-extcheck`.
Source symlinks are refused; `.git` and `node_modules` are excluded. Manifest
identities and source bytes remain unchanged inside these disposable copies.
Only ordinal case labels and aggregate fixed reason codes reach stdout or the
summary. Paths, manifests, source, errors, screenshots and per-case artifacts
are deleted after each case, including failed cases. Do not commit the work
directory or logs. Interrupted processes may require removing their owned run
directory after checking that their children stopped.

`--store` reads the public HTTPS catalog and every latest published release,
tree and archive, without credentials/cookies, at least one second apart.
Archive SHA-256, tree identity, complete file inventories, byte sizes and Git
blob hashes are verified before use. Traversal, links and oversized files are
refused. A second catalog read detects collection drift. Public-read failures
do not imply an empty Store. The Store connector can be used instead when an
isolated editor session is already available; it was session-gated in this run.

Store sources are compiled but never run through the trusted owner facade.
The frozen asynchronous facade now has a confined native helper/core connection
and durable kernel Store state adapter, covered by synthetic tests. Public
releases still need native installer admission and registration/view/resource
dispatch. The checked release metadata has no signed-release envelope for the native
installer. The disposable Electron profile uses the
shipping Store marker and verifies Store isolation, without copying account
state or granting full access. No extensions are excluded because an adapter
is absent.

## Scripts and capture protocol

`--scripts /private/path` reads `<source>-0001.json`, etc. Ordinals follow the
sorted snapshot; keep the snapshot fixed when authoring scripts. A script is:

```json
{
  "version": 1,
  "complete": false,
  "steps": [
    {"op": "reload"},
    {"op": "seek", "time": 0},
    {"op": "screenshot", "label": "active", "family": "DOM panel"},
    {"op": "dispose"}
  ]
}
```

The reference driver additionally supports `panel`/`command` with `id`, `click`
with `selector`, `fill` with `selector`/`value`, `key` with a Playwright key,
and `assert` with `selector`, `property` (`value` or text) and `equals`.
Scripts that contain private selectors/strings belong outside Git. `complete`
means a human has reviewed that extension's coverage; the default smoke script
is incomplete and cannot pass P2. Full corpus proof also needs extension-specific
projects, contribution mounting, media/rendering and callback scenarios.

An adapter reads one JSON request on stdin: `version`, anonymous `case`, copied
`extension` path, `input_sha256`, `script_sha256`, `script`, `role`, `output`.
`--native-command '["/path/to/adapter", "arg"]'` replaces the bundled native
probe when a full adapter is available. No shell interpolation is used.

A completed capture writes `trace.jsonl` (existing trace-recorder v1 format),
`interactions.json` (one `{op,result}` per step), `<label>.png` for every screenshot,
and `capture.json`. The latter includes `status: "captured"`, matching case/input/
script hashes; the reference also includes its exact frozen revision. Native
captures must declare:

```json
{"path":{"logic":"runtime-v8","panels":"webkit-remote-dom","canvas":"native-recorder"}}
```

This is adapter provenance, not independent security attestation. The bundled
probe does not declare it. Blocked captures use reviewed fixed reason codes.
Every role is attempted independently, even when the other role cannot run.
Timeouts terminate the owned process group. Each capture role acquires the
shared POSIX GUI lock and rechecks the screen. A busy GUI or locked screen waits
and retries every 60 seconds, outside the capture execution deadline.
Build/capture tools honor `/tmp/pm-shots/PAUSE_LOCAL`: they check every five
minutes and stop their owned process group if a local pause begins. Interrupted
capture attempts are retried, rather than classified as extension failures.
`/tmp/pm-shots/USER_ACTIVE` permits low-priority headless work but blocks either
capture role and the standalone GUI probe. GUI tools wait until it disappears,
and stop their owned capture if it appears during a run.
Electron uses a disposable profile, clears `ELECTRON_RUN_AS_NODE`, checks recorder
instrumentation, and closes on every exit path. The installed app is untouched.

Before accepting a match the harness requires two equal captures per role,
valid balanced/completed traces, exact ordered/type-preserving behavioral rows,
complete interaction results and successful reference assertions. Headers retain
host provenance but are not behavioral rows. PNGs are decoded and compared in
premultiplied RGBA, including dimensions; the default requires exact visible
pixels. No arbitrary tolerance or browser-specific exception is approved here.
Changes get only a family/row/step diagnostic; private differing values are not
exported. The six families are PM facade, sync services, raster override,
recorder canvas, DOM panel and lifecycle. Missing/invalid/nondeterministic
evidence is `cant_run`, not a mismatch or match. Exit 2 means P2 remains open.

The actual Rust bridge currently supports a recorder subset (basic state,
paths/rectangles, transforms, immutable drawImage sources and pixel readback).
Unsupported canvas operations fail explicitly. JSON transport, typed ImageData,
complete text/paint/encoding and kernel/engine backends need further integration.
Remote-DOM batches can be drained and feedback delivered on the V8 owner;
queue admission is not a WebKit paint acknowledgement. The native CLI runs on
a worker and is not callable as a blocking AppKit/main-thread API.

## Verify and clean up

```sh
python3 -m unittest discover -s tools/extcheck -p 'test_*.py'
node tools/extcheck/test_bundle.mjs /tmp/pm-extcheck/reference \
  "$HOME/Developer/pm-target/debug/pm-compat-runner"
bash tools/extcheck/build.sh test
bash tools/extcheck/build.sh clippy
bash tools/extcheck/build.sh fmt
git worktree remove --force /tmp/pm-extcheck/reference
```

Pillow is needed for the visual comparator (`python3 -m pip install Pillow` in
an appropriate environment). Only copy reviewed aggregate counts into
`docs/native/extcheck-p2.md`; delete owned temporary inputs and artifacts when
finished. Never remove another lane's target files, profiles or GUI session.
