# Aero eval

How "Aero is better" is proved (Aero design, section 8). The crate is
`apps/native/crates/aero-eval` (`pm-aero-eval`, binary `aero-eval`); its data is
in `data/`: `suite.json` (the versioned task file), `fixtures/` (synthetic
projects and broken extensions, see its README), `references/`, `rubrics/` and
`replay/` (recorded model streams).

## The suite

19 tasks, at least two per kind: cut, grade, add-effect, new-effect,
new-transition, build-panel, fix-extension (one each for a compile error, a
runtime error and wrong output), restyle, multi-step. Each has the prompt a
person types, its fixture, its graders and its time and step limits.
`aero-eval check` validates it: fixtures match their pins, every kind has two
tasks, every task has a programmatic grader, and no prompt names a tool or
uses implementation words (`aero-eval list` prints the tasks). Changing a
task, fixture or grader bumps `version`; summaries never pool versions.

## Graders

Each reads only the run's collected evidence (`artifacts.json`) and returns a
verdict (pass, fail or unavailable) with one evidence line per check.
Unavailable means the evidence was not collected; it never counts as success.

| Grader | Checks |
|---|---|
| `project` | Assertions on the saved project: counts, fields (JSON pointers), order by start, butt edits, effects and their parameters, keyframes, transitions, untouched layers, the composition root, any-of. `$new:effect` means an effect registered by an extension the run created or changed. |
| `extensions-load` | Every extension the run created or changed (by folder hash) loads in every window the host reports; required registrations exist; a fix task's extension was actually changed. |
| `frames` | A rendered frame against a reference (a PNG, a solid colour or the fixture's own frame) within a CIEDE2000 tolerance: mean and 95th percentile. |
| `frame-stats` | Statistics of a region (L*, percentiles, warmth, chroma, share of pixels near a colour, ΔE and tone correlation against the fixture's frame), absolute or relative to the fixture; at every time, or at any one (a transition's mid-point). |
| `frame-time` | Milliseconds the new effect or transition adds per frame, at a percentile, under its budget (4 ms for effects, 3 ms for transitions at 1280x720). |
| `panel` | The panel opens, shows the text it should, and a scripted probe (select layers, click a labelled control, type into a field) moves the playhead or edits the project as it should. |
| `theme` | The active theme comes from the run's extension; tokens meet hue, chroma and lightness ranges and WCAG contrast floors. |
| `rubric` | Subjective work: a fresh-context evaluator's scores against `rubrics/*.json` (every criterion needs evidence; hard criteria have floors). The owner's spot-check (`spot-check.json` in the run folder) decides whenever present; a disagreement with the evaluator is recorded for tuning the evaluator. |

## Running

Real-model runs need the Chief's OK: without `--approved "who, when"` (or
`--replay`) the runner refuses any harness but `none`. It never builds the
app; pass binaries built elsewhere.

```sh
cd apps/native
cargo run -p pm-aero-eval --bin aero-eval -- check
# One task on one harness (Codex through its CLI, the host as its MCP server):
aero-eval run --task cut-drop-flubbed-take --harness codex --model gpt-6.2-sol --effort high \
  --mcp-bin <powermove-mcp> --harness-bin <codex> --out <runs> --approved "<who, when>"
# A batch: tasks × harnesses × repeats, interleaved; writes summary.json:
aero-eval run --task all --harness aero,codex --model gpt-6.2-sol --effort high --repeats 5 \
  --mcp-bin <powermove-mcp> --harness-bin <codex> --out <runs> --approved "<who, when>"
aero-eval summarize <runs>
aero-eval regrade <runs>/<task>/<run>      # after adding spot-check.json
```

Each run gets a clean folder: a lane profile seeded with the fixture (a v1.2
library file entry) and its extensions, a headless `powermove-mcp --listen`
host the runner talks to over stdio, and an isolated home for the harness CLI
(only `PATH` and the locale are inherited; never the owner's sign-ins or keys).
The runner opens the project, renders the baseline frames, runs the harness
(Codex and Claude Code attach with `--powermove-mcp --profile`), then saves
and reads the project back, renders, reports extensions, times frames, runs
panel probes, grades, and writes `run.json` and `artifacts.json`. `--ablate`
passes the subsystems to switch off to Aero (topics, completion-gate,
self-check, evaluator). The `none` harness does nothing: on the real host it
proves seeding, collection and grading without a model (every task fails).

Aero runs inside the host, so its adapter is a JSON-RPC method on the
runner's host connection, for lanes A and B to implement: `aero/run`
(prompt, model, effort, limits, gate continuations, replay base URL,
ablations) answered with `{finalText, completed}`, with `aero/event`
notifications for usage, tool calls, retries and gate continuations. Until
then the adapter reports that the host has no Aero.

## Scoring

A run succeeds when every grader passes within the time and step limits.
"Renders correctly" is the frame graders alone. Tokens are normalised:
input counts every prompt token, cached ones included; cached is that subset.
Per harness, model and effort (pooled and per task) the summary gives the
success rate with a 90 % Wilson interval, medians of wall time and tokens
with distribution-free 90 % intervals (order statistics; five runs give the
sample range), mean tool calls, retries and gate continuations, and how
often the owner said a result needed fixing.

**Aero is better** on a model when, against each other harness on the same
model and effort, its success interval lies wholly above the other's and its
median wall time and tokens are no worse. Five runs per task per harness are
the minimum; pooled over the suite that is about 95 runs a side.

## Replay

A recording (`aero-eval-recording` v1) is every model request a harness made
and the streamed response it got, normalised as `crates/agents/tests/replay`
normalises: run folders become `/workspace/agent`, key order is insignificant,
everything else is exact. Per-run fields (a cache key) are listed in the
recording as volatile and stored masked. Headers and credentials are never
recorded; a recording with a home path or an API-key-shaped string is refused.
The replayer serves a recording on loopback (or in process, for Aero's Rust
fetch bridge): requests must arrive in order and match, and the first
divergence answers 409 with the differing leaves, so a harness regression is
one precise failure; an extra or a missing request fails too. `aero-eval run
--replay <recording>` points the harness at it; `aero-eval replay serve` and
`replay check` work on a recording by hand. The recording proxy forwards to a
plain-HTTP upstream only; recording a real provider (HTTPS) is part of an
approved batch's setup. `data/replay/synthetic-toy-cut.json` is synthetic,
proved by the crate's tests with a toy harness; real recordings come from
approved batches and then run in pm-gate.

## Not yet proved

Everything above runs against a fake host and fake CLIs in the crate's tests.
On the real `powermove-mcp` (first heavy slot, `--harness none` over the
suite): the library seeding, the PMV4 fixtures opening, `save_project`'s
answer, PNG frames from `render_frames`, and panel probes. The headless host
reports extension health as null and no transitions, themes or registration
owners yet, so those graders answer unavailable until it does; registrations
are attributed by searching the changed extensions' sources meanwhile.
