# What Powermove collects

Powermove sends anonymous usage, crash, hang and performance reports so we can
see what people use, what breaks and what is slow, and fix it. You can turn
this off at any time in **Settings › Privacy › Share Anonymous Usage and Crash
Reports**. Off, nothing is sent, and anything waiting to be sent is deleted.

The complete list of events and properties is one file:
[`apps/native/crates/telemetry/src/events.rs`](../apps/native/crates/telemetry/src/events.rs).
Nothing outside it is sent.

## Never sent

Your projects or anything in them; the names of layers, files, media,
compositions or projects; file paths; prompts or anything you or an agent wrote;
the source or names of your own extensions; your e-mail address. Your IP
address is discarded by the server.

## Who you are

A random install id made on your Mac. If you sign in to a Powermove Account,
that id is joined to the account's anonymous id (never the e-mail), so your
Macs count as one person.

## With every event

App version and build, release channel, macOS version, chip, memory (as a
range), number of displays and their largest scale, language and region codes.

## Events

| Event | What it says |
|---|---|
| `app_launched` | cold or warm launch, time to the first window and to the first usable frame, first run, whether the last session ended without quitting |
| `app_quit` | session length in minutes, clean quit |
| `onboarding_step`, `onboarding_finished` | onboarding step ids and what you did, time spent, whether the v1 library came over and how many projects and extensions |
| `project_opened`, `project_created`, `project_saved` | how it was opened, how long it took; layer, composition and effect counts, duration, resolution and frame rate as ranges; whether it has 3D; save time and file size as a range |
| `command_invoked` | the built-in command's id and how you ran it (menu, shortcut, palette, toolbar, context menu); an extension's commands count as "other" |
| `panel_opened`, `tool_selected`, `effect_added`, `layer_added` | built-in ids; a Store extension by its Store id; your own extensions only as "local" |
| `export_started`, `export_finished` | format, codec, resolution and duration ranges, frame rate, render time, result, a fixed error class |
| `agent_run_finished` | which harness, the model if it is a known public model (else "custom"), effort, result, duration, turns, tool calls and tokens when the harness reports them (never the prompt or any message) |
| `store_extension_installed`, `store_extension_removed` | the Store extension's id |
| `settings_changed` | theme, tabs or windows, and telemetry being turned back on |
| `perf_summary` | every 5 minutes: frame-time percentiles and slow-frame counts per activity (paused, playing, scrubbing, dragging, scrolling), playback frame rate achieved and aimed for, peak memory and GPU memory, thermal state, battery |
| `hang` | the app stopped responding for more than 2 seconds: how long, as a range (no stack) |
| exceptions | a crash or panic: its type from a fixed list (a panic: where in Powermove's code, never its message), and the stack as offsets and function names in Powermove's own executable; other frames are anonymous |

## How it works

Release builds only (`cargo build --features telemetry`); development, test and
lane builds compile it out or keep it off. Nothing is sent before Powermove has
read your Settings choice. The app hands events to a background thread without
waiting; it sends them in batches to PostHog (US) and keeps them in
`telemetry-outbox/` in the profile while offline (at most 200 files, 8 MB).
Turning sharing off deletes that folder and everything waiting. Frame times are
counted on the Mac and sent as one summary every 5 minutes. A crash is read
from the macOS crash report at the next launch, and only if it is from a
Powermove session that was sharing. `PM_NO_TELEMETRY=1` turns it off for a
process; `PM_TELEMETRY_DEV=1` turns it on in a development build, where only
events marked internal can leave.
