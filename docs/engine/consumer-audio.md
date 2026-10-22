# Engine audio health in the status bar and the performance panel (A5)

Issue A5: pm-node's `AudioStatus` (problems, starved renders, underruns,
device overloads, the device error) was kept in the presenter's diagnostics
stats and never reached the app. It now reaches the existing status bar and
performance panel. There is no new surface and no modal. Today's app plays
fully decoded WebAudio buffers and has none of these signals, so with "Use
New Engine" off nothing changes.

## Wire

`RenderStats` (contract 2, render transport RFC 16) has an optional `audio:
AudioDiagnostics` field:

```
AudioDiagnostics { error?, problems, starvedRenders, underruns, deviceOverloads }
```

- `problems` is `AudioStatus.problems` as the audio thread keeps it: at most
  16 distinct messages, oldest first, each listed once (pm-node
  `report_playback`). These are media that did not resolve, mix and transport
  errors, and the bounded-wait drop from A3 ("Audio decode exceeded 120 ms;
  missing voice plays as silence: <voice>").
- The counters run from when the session's audio started.
- `audio` is absent until the session opens audio (a silent composition never
  does). Once a session has an audio link, every report carries it.

The field is additive and every contract is still a draft (version 0), so
there is no version bump. The generated TypeScript (`AudioDiagnostics.ts`,
`RenderStats.ts`) and the `event-renderStats` wire fixture were regenerated
with `pm-protocol-gen`. The fixture now carries a dropped-voice problem.

RFC 16 ("Stats") still lists the fields from before this change. Its owner
should add `audio?` there. This lane was not allowed to edit the RFC.

## Engine (pm-node)

- `present/stats.rs`: `ViewStats::audio` takes the session's `AudioStatus`
  each render-thread turn. A report is due when the frames presented, the late
  skips, *or the audio diagnostics* changed since the last report, still at
  most every 500 ms. Copying an unchanged status every turn sends nothing.
- `present/render.rs`: `document_step` already read the link's status into
  the node's stats every turn. It now passes the same status to the view's
  `ViewStats`. The render loop wakes at least every 50 ms, so a paused
  session reports a dropped voice or a failed device without presenting a
  frame. A view whose surface pool has not arrived yet used to return before
  `send_stats`. It now falls through to it, so its audio still reports.
- Not covered: a composition the engine routes to the legacy viewer returns
  before `send_stats`. That routing is dev scaffolding being retired
  (amendment 21).

## Renderer

- `engine-connection/audio-diagnostics.ts` turns `RenderStats.audio` into
  `AudioHealth`, then into the status bar field and the performance panel
  rows. `EngineStatsView.audio()` returns the latest session's health.
- `runtime/install-legacy.ts` mirrors it into `audioStatus` in
  `state/transport.svelte.ts` on `draw:status`, the same tick that mirrors
  engine `perf`. It replaces the value only when it changed, and clears it
  when the engine stops reporting.
- `shell/StatusBar.svelte` adds one plain `status-field` span after the
  playback field, only while something is wrong. The text shows the first
  that applies:
  1. `No audio output` when the device did not open.
  2. `Audio · N problem(s)` when there are problems.
  3. `Audio · N dropout(s)` for underruns plus overloads.

  The tooltip lists every problem and a counts sentence such as
  "2 buffer underruns and 1 device overload since audio started." It is
  plain text, so it has no pointer cursor and isn't a button.
- `panels/PerfPanel.svelte` appends rows after the existing counts, using
  the same `.row.split` anatomy:
  - `Audio output: Unavailable`, only when the device failed
  - `Underruns`
  - `Overloads`
  - `Audio waits` (starved renders)
  - one `Audio issue` row per problem

  Each row has a tooltip that explains it. Problem text is one line with an
  ellipsis, and the tooltip holds the full message.
- Starved renders on their own stay out of the status bar. A segment held
  within the 120 ms bound plays bit-exact and is not heard. A wait that
  exceeded the bound is a drop, and the drop shows as a problem.

## Tests

- `pm-node present::stats::tests::audio_changes_report_while_nothing_presents`:
  audio changes make a report due without frames. An unchanged status does
  not. Error, problems and counters reach the report. A closed link reports
  `None`.
- `pm-protocol` generated-output and wire fixture checks (regenerated).
- `engine-connection/audio-diagnostics.test.ts` (vitest, happy-dom) covers:
  - the mapping, singular and plural wording, and status-field priority
  - `EngineStatsView.audio()` across reports, including a paused report with
    no new frame
  - the `draw:status` mirror, with `StatusBar` and `PerfPanel` mounted:
    today's engine shows nothing, a dropped voice shows in both surfaces,
    and turning the engine off clears both

## Open

- No end-to-end run through the app: this lane did no Electron run or native
  addon rebuild. `PROTOCOL_BUILD` changed, so the pm-node addon must be
  rebuilt before the app talks to it.
- Counters and problems never reset while a session's audio link lives (the
  audio thread keeps them). Once a session drops audio, its status field
  stays until the session closes.
- The status field isn't a live region, so a screen reader is not told when
  it appears. Other status bar fields aren't live regions either.
