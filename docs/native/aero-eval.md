# Aero eval

How "Aero is better" is proved (Aero design, section 8). The crate is
`apps/native/crates/aero-eval` (`pm-aero-eval`, binary `aero-eval`); its data is
in `data/`: `suite.json` (the versioned task file), `fixtures/` (synthetic
projects, media and broken extensions, see its README), `references/`,
`rubrics/` and `replay/` (recorded model streams).

## The suite

19 tasks, at least two per kind: cut, grade, add-effect, new-effect,
new-transition, build-panel, fix-extension (one each for a compile error, a
runtime error and wrong output), restyle, multi-step. Each has the prompt a
person types, its fixture, its graders and its time and step limits.
`aero-eval check` validates it: fixtures match their pins, every kind has two
tasks, every task has a programmatic grader, and no prompt names a tool or
uses implementation words (`aero-eval list` prints the tasks). Changing a
task, fixture or grader bumps `version`; summaries never pool versions.

Every task's graders are tested against a correct result and the cheapest
wrong results we could think of (`tests/wrong_results.rs`, one test per task):
clips switched off or renamed into place, a disabled effect with valid keys, a
tint passed off as a chromatic split, a wipe passed off as a push, a dead
panel button, a no-op Tap, transparent theme text, and so on. Each wrong
result fails a programmatic grader. The owner's spot-check decides only the
rubric; it never rescues a programmatic failure.

## Graders

Each reads only the run's collected evidence (`artifacts.json`) and returns a
verdict (pass, fail or unavailable) with one evidence line per check.
Unavailable means the evidence was not collected; it never counts as success.

| Grader | Checks |
|---|---|
| `project` | Assertions on the saved project: counts, fields (JSON pointers), order by start, butt edits, kept layers (the same ids, with their enabled state, media, trim and speed from before the run), effects (switched on, parameters, instance count), keyframes (on a switched-on effect; first and last keys differ), transitions, untouched layers, changed values, markers added, the composition root, any-of. `$new:effect` means an effect registered by an extension the run created or changed. |
| `extensions-load` | Every extension the run created or changed (by folder hash) loads in every window the host reports; required registrations exist; a fix task's extension was actually changed; an extension folder with a symlink fails, unread. |
| `frames` | A rendered frame against a reference (a PNG, a solid colour or the fixture's own frame) within a CIEDE2000 tolerance: mean and 95th percentile. |
| `frame-stats` | Statistics of a region (L*, percentiles, warmth, chroma, share of pixels near a colour, ΔE and tone correlation against the fixture's frame, fine detail as mean Sobel magnitude, sideways red/blue channel displacement), absolute, relative or as a ratio to the fixture; at every time, or at any one (a transition's middle). |
| `frame-time` | Milliseconds the new effect or transition adds per frame, at a percentile, under its budget (4 ms for effects, 3 ms for transitions at 1280x720), from paired enabled/disabled timings the host measures. |
| `panel` | The panel opens, and every control the prompt names is exercised by its own probe (select layers, seek, click a labelled control, type into a field), run in order on the same panel. Each probe is judged against the state just before its steps: project, markers, playhead and frames rendered before and after them. Rows pair what belongs together: a marker with its time, a layer with its note. A row is an element of the panel naming its item and none of the probe's other items, and it must show its own time (read from `2.5`, `2.5s`, `0:02`, `0:02.5`, `1:02:03` or `00:00:02:15` at 24/25/30/60 fps; bare integers are not times) and no other item's. |
| `theme` | The active theme comes from the run's extension; tokens meet hue, chroma and lightness ranges and WCAG contrast floors. A foreground is composited over its background before contrast is measured; a translucent background or a translucent token checked on its own fails. |
| `rubric` | Subjective work: a fresh-context evaluator's scores against `rubrics/*.json` (every criterion needs evidence; hard criteria have floors). The owner's spot-check (`spot-check.json` in the run folder) decides the rubric whenever present; a disagreement with the evaluator is recorded for tuning the evaluator. |

Before the baseline the runner checks which evidence the host can supply
(`runner/capabilities.rs`); a grader that needs something the host lacks
answers "unavailable: host lacks …" (see the host contract below).

## Running

Every run is a **replay** of a recorded model stream, or the `none` harness
(no model). Live runs are refused: they need batch credentials, and how an
approved batch supplies them is not decided yet (`harness::Credentials`; the
only implementation, `NotApproved`, refuses). `--approved "who, when"` is
recorded but does not unlock a live run. The runner never builds the app;
pass binaries built elsewhere.

```sh
cd apps/native
cargo run -p pm-aero-eval --bin aero-eval -- check
# One task on one harness, replayed:
aero-eval run --task cut-drop-flubbed-take --harness codex --model gpt-6.2-sol --effort high \
  --mcp-bin <powermove-mcp> --harness-bin <codex> --out <runs> --replay <recording>
# Seeding, collection and grading on the real host with no model:
aero-eval run --task all --harness none --model none --effort none --mcp-bin <powermove-mcp> --out <runs>
aero-eval summarize <runs>
aero-eval regrade <runs>/<task>/<run>      # after adding spot-check.json
```

A replayed `run` exits 1 when any run's replay diverged or was incomplete,
its harness did not complete its turn (an exit status other than 0, a failed
turn, Aero `completed: false`) or the run errored, whatever the graders said:
that is the regression gate. A recording replays only the slot it was made
for: its task, harness, model and effort are checked before anything starts.

Each run reserves a fresh folder atomically (an existing folder is never
touched, and a failed write fails the run): a lane profile seeded with the
fixture (a v1.2 library file entry) and its extensions, a headless
`powermove-mcp --listen` host the runner talks to over stdio, and an isolated
home for the harness CLI. The runner opens the project, renders the baseline,
runs the harness (Codex and Claude Code attach with `--powermove-mcp
--profile`), then saves and reads the project back, renders, reports
extensions, runs panel probes, grades, and writes `run.json` and
`artifacts.json`. Evidence is collected and kept even when the harness or the
replay failed. `--ablate` passes the subsystems to switch off to Aero (topics,
completion-gate, self-check, evaluator); `--cohort` names the batch.

### Isolation

- **Environment**: the host and the CLIs get exactly `PATH`, the locale, a
  home and a temp folder inside the run, and (CLIs) their config folders and,
  for replays, a loopback endpoint with a dummy key. Nothing else is inherited.
- **Files**: the runner opens the reserved run folder once and reaches every
  file below it from that open folder, one component at a time, with
  `openat(…, O_NOFOLLOW)` (`runner::paths::Anchor`): reads, walks (the
  extension roots and every extension folder), frame and capture writes, the
  saved project and the graders' frame reads. A component that is a link is
  refused, never followed, and there is no check-then-open by name a run
  could race. Files the runner creates directly in the run folder are
  created exclusively (a link left there is never written through), and a
  rewrite replaces the name. The path `save_project` names is read only if it
  lies in the run's `exports` or `work` folder and every component from the
  run folder down is a real folder or file.
- **OS sandbox**: replays and `none` runs put every component (host, CLI and
  all their children) under a Seatbelt profile (`sandbox-exec`, the mechanism
  `pm-runtime-v8`'s Store sandbox uses): reads under `$HOME` and `/Users` are
  refused except the run folder and the binaries given (`--sandbox-read PATH`
  adds a CLI's install folder), writes go only to the run folder, the
  Keychain daemons cannot be looked up, and the only outbound network is TCP
  to the replay's port (a `none` run has none at all; no DNS, no UDP). On a
  platform without Seatbelt, or when the profile does not apply (the runner
  runs a trivial program under it first), the run is refused. Canary tests
  prove each rule: an inherited canary variable, a canary file outside the
  run, a connection to another host, to an unrelated loopback listener and to
  a listener on this Mac's own address are all refused.
- **Network boundary for replays** (decision of the harness engineer,
  2026-11-24; no VM). Seatbelt's one host filter, `localhost`, matches
  loopback and this Mac's own addresses alike, so a one-port rule alone still
  reaches a host service listening on that port on any interface. The replay
  server therefore picks a free port and holds it on every interface for the
  whole run (`replay::hold::PortHold`): loopback, both wildcards and every
  address of every interface, each bound without `SO_REUSEPORT`. A wildcard
  alone is not enough on macOS: with `SO_REUSEADDR` or `SO_REUSEPORT` another
  socket still binds the port on `127.0.0.1` or on a host address (measured
  on this Mac). Before a replay starts the hold proves itself: binding the
  port on loopback, both wildcards (a dual-stack socket included) and every
  interface address, with no option, `SO_REUSEADDR`, `SO_REUSEPORT` and both,
  must all be refused, or the replay is refused. While the run goes a
  watchdog holds any address that appears (a VPN coming up) and marks the
  replay failed when one cannot be held. Connections that reach a holder are
  dropped at once; the replay serves loopback peers only. The Seatbelt
  profile allows outbound TCP to that port only. Tests: a replay succeeds
  through the held port; a confined run is refused an unrelated loopback
  listener and a listener on this Mac's non-loopback address; no other
  listener can bind the replay port on loopback, the wildcards or any host
  address while the server holds it.

### Deadlines and steps

Every component runs in its own process group. At the wall-clock limit the
whole group is killed and reaped, so no child is left holding a pipe; the
group is also stopped when the harness exits by itself. Host start-up and
`initialize`, each host call, and the baseline and the collection each have
their own deadline (`runner::mcp::Deadlines`). The runner's host connection
reads on one thread and writes on another, and the caller only waits on a
channel with its own deadline, so a host that stops reading its input cannot
hold a request, a notification or a cancel past its deadline: a write that
misses it spends the connection (every later call fails at once) and the
host is stopped. One step is one tool call the model issued, counted once
per call id the moment the call starts (Codex `item.started`, Claude Code's
`content_block_start` with `--include-partial-messages`, Aero `tool-started`),
so a call that starts and stalls has used its step. Every adapter counts
steps as the stream arrives and stops the run the moment it passes the
task's limit (Codex and Claude Code by stopping the CLI; Aero by
`aero/cancel`). No turn cap is passed to a CLI.

## Scoring

A run succeeds when every grader passes, the harness completed its turn, a
replay was consumed exactly, and the run stayed within its time and step
limits. "Renders correctly" is the frame graders alone. Tokens are
normalised: input counts every prompt token, cached ones included; cached is
that subset. A run whose harness reported no usage has no tokens (never
zero), and so does a run with any usage report lacking real input and output
counts (an empty `usage {}` is unavailable, not zero); wall time is absent
when the harness never ran.

Per harness, model, effort, mode (live, replay, none) and ablation, pooled
and per task, the summary gives the success rate with a 90 % Wilson
interval, medians of wall time and tokens over the runs that reported them
(with how many did not) and distribution-free 90 % intervals from order
statistics with their attained coverage (none below five samples, where no
order-statistic interval reaches 90 %), mean tool calls, retries and gate
continuations, and how often the owner said a result needed fixing. Modes
and ablations are never pooled; duplicate run ids count once.

**Aero is better** on a model and effort when, against every other harness
that ran live on it (`none` never counts), its success interval lies wholly
above the other's and its median wall time and tokens are no worse. Runs
compare only inside a cohort both harnesses ran in as one interleaved batch
(distinct batch positions, start times and repeats in batch order, both
harnesses in every repeat), and only on a balanced sample: the same tasks on
both sides, and from every task the same number of repeats both harnesses
ran, paired by repeat, at least five. Runs outside a shared cohort or beyond
the balanced sample are left out and counted in the reasons. Every compared
run has observed wall time and usage, and only live runs of the full
harness count (no replays, no ablations). Each shared cohort must clear the
bar on its own. One verdict per model: better only when every comparison is;
unavailable when the evidence cannot decide.

## Replay

A recording (`aero-eval-recording` v1) is every model request a harness made
and the streamed response it got, normalised as `crates/agents/tests/replay`
normalises: run folders become `/workspace/agent`, key order is insignificant,
everything else is exact. Per-run fields (a cache key) are listed in the
recording as volatile and stored masked. Headers are never recorded; only
the provider's known paths (`/v1/responses`, `/v1/messages`…) and query keys
(`beta`) are, and other query pairs are dropped. Validation (on load, on
finish and on every save, so an unvalidated recording is never written)
refuses credential-named fields, key-shaped strings, `Bearer` values and home
or temp paths of any platform. SSE is decoded per complete line, so a
character split across reads is kept intact.

The replayer serves a recording on loopback (or in process, for Aero's Rust
fetch bridge): requests must arrive in order and match. The first error of
any kind (a divergence, a malformed or unfinished request, a body that is not
JSON) is latched; every later request gets the same 409 and the report fails.
Each connection has a read and write deadline, so an unfinished request
cannot hold the server. An extra or a missing request fails too. `aero-eval
replay serve` and `replay check` work on a recording by hand. The recording
proxy forwards to a plain-HTTP upstream only; recording a real provider
(HTTPS) is part of an approved batch's setup.

**Real recordings** (`synthetic: false`) come only from approved batches and
need an explicit review before they enter this public repo: someone reads
every exchange, then records `review: {by, at, note}` in the file. A test
refuses any committed recording that is neither synthetic nor reviewed.
`data/replay/synthetic-toy-cut.json` is synthetic, proved by the crate's tests
with a toy harness.

## Host contract (for lane B)

The stock headless host leaves the engine, surfaces and extension services
unset, so render, panel, transition and evaluator evidence is unavailable on
it. An eval-capable host advertises what it supplies in its `initialize`
answer, `capabilities.experimental.powermoveEval`, and offers the tools:

| Evidence | Advertised flag or tool | What it must be |
|---|---|---|
| Extension health | `extensionHealth` | `get_workspace_state` `extensions[].health` (`ok`, or the state and error) per window of the tested window set, after the run's extensions reload. |
| Registration owners | `registrationOwners` | Every entry of `registeredEffects`, `registeredTransitions`, `registeredPanels` and `theme` names the extension that registered it (`extension`). No attribution by searching sources. |
| Transitions, panels, themes | `transitions`, `panels`, `themes` | `registeredTransitions[]`, `registeredPanels[]` (`{id, extension, parameters}`), and `theme {active, extension, tokens}` with resolved CSS values. |
| Panel probes | `open_panel`, `get_panel_state`, `interact_panel`, `capture_panel` | As in the v1.2 catalog, on the eval connection. `get_panel_state` is a tree of the panel's elements, one JSON object (or list) per element, with its displayed strings as values: the runner reads each element's text as a row, so a list item's name and time must sit in the same element. `ref`, `id`, `role`, `type`, `kind`, `key`, `testId`, `className` and `style` are never read as displayed text. |
| Seek | `set_playhead {time}` | Moves the playhead for the probe. |
| Frame time | `measure_frame_time {target, times, samples, width}` | `{revision, enabledMs[], disabledMs[]}`: paired renders of the same revision with the target enabled and disabled, at active frames. (Subtracting a whole-render median credits unrelated edits that offset a slow effect.) |
| Evaluator | `evaluator` flag and `evaluate_rubric {rubric, prompt, sees}` | A fresh-context evaluator session (never the generator's context) answering an `Evaluation` (`{evaluator, rubric, scores: {id: {score, evidence}}}`). |

### `aero/run` wire contract

The eval adapter is a private method on the runner's own host connection,
around the same in-process `pm-agents` Aero harness the app uses; the
product UI keeps the normal Action/Event path. The runner's client side is in
`runner/harness.rs` and is tested against the fake host.

- **Request** `aero/run` `{runId, projectId, revision, prompt, model, effort,
  wallSecs, maxSteps, maxGateContinuations, provider, ablate}`. The run is
  bound to the project the runner opened and its revision; the host refuses
  any other. `provider` is explicit: `{mode: "replay", baseUrl}` sends every
  model request to the loopback replay and never falls back to real auth;
  `{mode: "none"}` and `{mode: "live"}` are the others.
- **Events** `aero/event` notifications `{runId, seq, event}`: run-scoped,
  `seq` from 0 with no gaps, `event` one of `usage {input, cached, output}`
  (all three counts required), `tool-started {callId, name}` (one step,
  counted once per `callId` when the call starts), `tool-finished {callId,
  ok}` (only for a started call), `retry {reason}`,
  `gate-continuation {failures}`. The runner refuses events of another run,
  out of order, or finishing a call that never started.
- **Result** `{runId, finalText, completed, reason}` with `reason` one of
  `completed`, `gate-exhausted`, `step-limit`, `deadline`, `cancelled`,
  `error`. Only `completed` with `completed: true` completes the turn.
- **Cancellation** `aero/cancel {runId, reason}` (a notification) is wired to
  the harness's existing cancel action; the host then answers the pending
  `aero/run` with `reason: "cancelled"`. The runner sends it at the deadline
  and when the steps pass the limit, then waits a grace period; a host that
  does not answer is stopped.
- The RPC dispatch needs a notification and cancellation path for an
  in-flight call (today it has none).

## Not yet proved

- **The host's `--listen` bridge under the replay profile.** The real host's
  bridge for the harnesses' MCP proxies is a TCP port on IPv4 loopback that
  the host picks itself. The replay profile allows outbound TCP to the
  replay's port only, so a real CLI's proxy cannot reach that bridge. Lane B:
  serve the bridge on a Unix socket inside the run folder (the profile
  already allows those), or take a port the runner holds and passes in the
  same way as the replay's.

Everything above runs against a fake host and fake CLIs in the crate's
tests, under the real OS sandbox. On the real `powermove-mcp` (first heavy
slot, `--harness none` over the suite): the library seeding, the PMV4
fixtures opening, `save_project`'s answer, PNG frames from `render_frames`,
whether the host and the real CLIs run under the sandbox profile (a CLI
installed under `$HOME` needs `--sandbox-read`), and the take colours the
cut graders expect from the fixture media.
