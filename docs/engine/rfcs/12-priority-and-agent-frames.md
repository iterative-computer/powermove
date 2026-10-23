# RFC 12: Priority classes and agent frame requests

Status: draft, version 0. Module: `pm_protocol::priority`. Gates Phase 2;
freezes when Phase 2 starts. Owner: Engine and platform (priority and
slicing, section 27), with Export and agents for the agent-facing rules.

## Purpose

Interactive frames come first, then agent snapshots, then jobs (plan
section 4, principle 7). Agents now capture several frames on every turn,
and today a capture pauses the user's playback and fails while another
capture or an export runs (appendix A item 35). In the engine, agent frames
are batched requests at their own priority that carry the agent's revision
(read-your-writes), **queue instead of failing, and never pause playback**
(section 20). Agent exports run in their own lane ahead of user batch jobs,
and `export_video` gains an optional `mode` and a queued result
(section 17). This contract defines the classes, the agent frame request
and result, and the job submission messages they share with user exports
and generated media.

## Types

| Type | Meaning |
|---|---|
| `PriorityClass` | Lowest to highest: background (4b renders), generation (proxies), user job, agent export, agent snapshot, interactive. The derived order is the preemption order (`a > b` means `a` preempts `b`) |
| `AgentFrameRequest`, `AgentFrame` | `Request::AgentFrames` (host → job engine, session-bound): agent run, 1 to 64 frames (composition and time), longest edge, image format, motion blur, `BlenderPolicy`. The envelope revision is the agent's last write |
| `AgentImageFormat` | `jpeg {quality}` or `png` |
| `BlenderPolicy` | `cachedOrPending` (default: cached Blender surfaces, else native shading reported as pending) or `render` (wait for Blender) |
| `AgentFrameResult`, `AgentFrameOutcome`, `BlenderUse` | One outcome per requested frame, in request order: rendered (index, revision rendered, size, image, how Blender was used) or failed (index, error); plus the time spent queued behind higher classes |
| `ImagePayload` | Inline bytes for small images, or a cache file behind a capability handle |
| `SubmitJob`, `JobSpec` | `Request::SubmitJob` (host → job engine): job id, class, owning agent run, spec, output choice (contract 14). Specs: `export {session, composition, settings}` from a frozen job snapshot session, or `generateProxy {asset, format, size}` |
| `ExportSettings` and its enums | Format (ProRes profiles, H.264, HEVC 8 or 10-bit in MP4 or MOV, WebM VP9 or VP8, PNG sequence or still at 8 or 16 bits), alpha (none, straight, premultiplied), quality tier (Draft, High, Max at 4, 16, 40 Mbps), size, frame rate, range, audio, motion blur, output color (contract 4), precision, dither, draft from proxies |
| `ProxyFormat`, `ProxySize` | Appendix D: ProRes 422 Proxy, ProRes 422 LT, HEVC 10-bit; Automatic (never below 540p), Half, Quarter, 1080p, 720p |
| `JobAccepted`, `JobStatus`, `JobState` | Acceptance with queue position; status events that map one-to-one onto `get_render_status` `{jobId, state, progress, path?, error?}`; states queued, running, waiting for media, done, failed, cancelled |
| `CancelJobs`, `JobsCancelled` | Cancel one job, or every job of an agent run (the run was cancelled or rolled back) |
| `AgentExportMode`, `AgentExportReply`, `agent_export_reply` | `export_video` `mode: "wait" \| "async"` (default `wait`) and the reply rule below |

## Source jobs

`sourceFilmstrip { session, asset, times, outputSize, fit, color }` draws one
asset from a frozen job snapshot. Times are non-negative source ticks, in
requested order, with one to 64 frames. Each frame is top-down straight
RGBA8, concatenated without a header. The protocol's batch limit is 512 MiB;
the app additionally caps each frame at 16 MiB and each edge at 8192 pixels.
Fit and color follow RFC 16's source target. SVG
sources request Chromium rasters through `CompatFrame`, using the frozen
document id to find the owning editor window. Their `sourceFit` key names
the final output canvas and its fit mode; the host returns straight pixels
drawn once at the fitted rectangle, which the job packs without refiltering.

`browserRendition { session, asset, format }` converts the frozen original
to `png` or `webm` with today's browser-rendition converter flags. It does
not render a composition. Its output follows EXPORT-1: decoded image,
video, and audio content must match the current converter. Both jobs use
the existing job scheduler, output capabilities, terminal status events,
and cancellation. Converter cancellation terminates the child process.

Source frame jobs retain a bounded, sender-owned frozen-session cache: at
most four cached sources per window, sixteen overall, with a 90-second idle
expiry. Retired in-flight bindings live until their requests release them;
they are no longer cache entries.
Its document contains only the requested asset, preserving all metadata
except the generated poster flag. Main pins the selected media before a cold
engine wait. Unrelated project edits and poster persistence can advance the
same source session's revision without changing its body or media binding;
a changed source reading creates another session. Already submitted jobs
retain their frozen document and binding until completion. Restart, project
close and window teardown retire sessions before numeric identifiers can be
reused. Browser-rendition jobs retain their dedicated per-request sessions.

Poster/source-monitor use prepares one real source frame ahead of a strip.
An idle one-frame job also starts the exact Export-purpose decoder's bounded
same-GOP prefetch, sharing its decoded cache with later batch jobs. Larger
strips and playback slices keep direct decode under the priority gate.
The user-job lane initializes its renderer when the job runtime starts;
other lanes remain lazy. A filmstrip admits bounded chunks through the
priority gate (up to sixteen frames and 16 MiB of padded staging), records one
GPU submission and maps one readback per chunk. Playback can reduce an
admission to one frame. SVG exchange remains one frame at a time. Output
remains ordered straight RGBA8, with no image encoding. Main returns one
packed typed array over Electron IPC; the renderer creates frame views after
the preload boundary. This is not a zero-copy transfer. Completed strips
have a 16 MiB renderer cache within the existing 64 MiB source cache budget.

Each request still owns a private scratch destination and removes output
capabilities, listeners and scratch files on success, failure or cancel.
Window reload and close cancel the sender's outstanding requests. Source
requests never change composition playback or silently call the legacy
capture/conversion surface.

## Invariants

1. **Class order and placement** (tested):

   | Class | Process | During playback | Persists across relaunch |
   |---|---|---|---|
   | Interactive | Interactive engine | Never preempted | No (not a job) |
   | Agent snapshot | Job engine | Sliced (contract 13) | No (not a job) |
   | Agent export | Job engine | Sliced | No: agent jobs never resume |
   | User job | Job engine | Sliced | Yes |
   | Generation | Job engine | Paused | Yes |
   | Background | Job engine | Paused | Yes |

   Agent snapshots run in the job engine, so they can never pause or drop
   the user's playback; contract 13 keeps the job engine off the GPU's
   critical path.
2. **Read-your-writes.** An agent frame request's envelope revision is the
   agent's last write. The job engine's session follows the same patch
   stream (contract 5) and answers at that revision or later; each outcome
   carries the revision it rendered. If the revision does not arrive within
   the wait bound, the frame fails with `StaleRevision` (contract 7); it is
   never answered from an older revision.
3. **Queue, never fail for being busy.** A request behind other work waits
   in its class; `queued_ms` reports the wait. Busy is not an error code.
4. **Agent defaults stay** (section 20): `render_frames` 720 px,
   `observeVisual` 480 px, JPEG quality 0.74, motion blur off, Blender
   `cachedOrPending`. A batch has 1 to 64 frames; the longest edge is 1 to
   8192 px; JPEG quality is in (0, 1]. Checked by
   `AgentFrameRequest::validate`.
5. **One outcome per frame, in order.** A failed frame does not fail the
   batch.
6. **Agents never choose output paths** (section 3, `main/agent-export.ts`).
   An agent export, and only an agent export, belongs to an agent run and
   uses the host's automatic destination (`OutputChoice::AgentAutomatic`);
   user exports use a destination the user chose. Agent exports never
   render from proxies. Proxy generation runs only in the generation class
   and has no output (generated media goes to the generated-media store).
   Agent snapshot and interactive classes are not jobs. Checked by
   `SubmitJob::validate`.
7. **Export settings are consistent** (section 17): sizes 1 to 16,384 px per
   edge, a positive frame rate (it may differ from the composition rate), a
   non-empty range, alpha only where the format carries it (ProRes 4444 and
   4444 XQ, 8-bit HEVC, WebM, PNG), no audio on image outputs, image tags
   for images and video tags for video, never the viewer's color space.
   Checked by `ExportSettings::validate`.
8. **`export_video` replies** (`agent_export_reply`, tested): done returns
   the real path after encoding, verification, and the move into place;
   failed and cancelled return at once; in `async` mode anything unfinished
   returns `{status: "queued", jobId}`; in `wait` mode a job that has waited
   in the queue longer than the configured bound returns queued, and a
   running job keeps the call waiting, because tool timeouts count from job
   start. A done job without a path is reported as an internal failure.
9. **Cancelling or rolling back an agent run cancels its jobs**
   (`CancelJobs::Run`); the output scope is per job, not per window.
10. **Status is the render status.** `JobStatus` has a path only when done
    and an error only when failed; progress is 0 to 1. Host services fill
    in the path; the engine never sees paths (contract 14).

## Implementation (Phase 2 infrastructure)

`pm_engine::priority::Scheduler`: one FIFO per class, the oldest item of
the highest runnable class first, generation and background paused during
playback, every job-engine class sliced during playback, work for the other
process refused, and the queue time reported with each item.

Phase 3b (delivery, `docs/engine/export.md`): `ExportSettings.delivery`
(`DeliveryOptions`: audio coding, start timecode with drop-frame and reel,
hide captions, live preview, PNG sequence name) and `JobStatus.preview` (a
JPEG of the latest encoded frame, at most `JOB_PREVIEW_MAX_EDGE` = 640 px).
Both are optional on the wire (serde defaults, optional in TypeScript), so
Phase 2 peers decode them. Queued user jobs whose media is offline report
`waitingForMedia` and recheck every 2 s; agent exports fail `mediaOffline`.

## Versioning

Version 0 until Phase 2 starts. New export formats after cutover (DNxHR,
AV1, EXR, DPX, TIFF, GIF, animated WebP, audio-only) are new `ExportFormat`
variants and a version bump each. New classes are a bump, and their place
in the order is part of the contract.

## What Phase 0 must confirm

- **S8 (GPU contention):** that running agent snapshots in the job engine at
  their class, sliced per contract 13, keeps 4K playback at zero drops over
  60 s. If not, the fix is stronger slicing or pausing jobs during playback
  (S8's on-failure column), not moving agent snapshots into the interactive
  engine.
- **Phase 2 performance row:** "Agent snapshots during playback: 4 frames
  per agent turn, no dropped playback frames". The protocol cost of a
  4-frame batch is one request and one response; the Rust codec numbers for
  the samples are in the fixtures (`request-agentFrames`,
  `response-agentFrames`).
- **Phase 3c bound:** `render_frames` with 5 frames at 720 px, p95 ≤ 10 s
  during 4K playback, with 8 subagents, a second thread, and 2 external
  connections. The contract allows it (batches up to 64, no single-flight);
  the gate measures it.
- **Export and agents lane:** the configured queue bound for `wait` mode
  and the updated tool timeouts (`LONG_CALLS`, `toolTimeoutSec`), agent
  instructions, and `MCP.md` (section 17).

## Open questions

1. Whether 64 is the right batch ceiling. Composition contact sheets are
   the largest batch today; the Export and agents lane sets the number from
   the contact-sheet tool before the freeze.
2. Whether agent snapshots should be preemptible mid-frame by interactive
   work beyond what slicing gives (contract 13). Slicing is the mechanism;
   no separate preemption message exists.
3. Whether user exports started while an agent export runs should be
   allowed to overtake it when the user explicitly asks ("Render now").
   Section 17 puts agent exports ahead of user batch jobs; an explicit user
   override is not in the plan and would need Jude's decision.
4. Whether `ExportSettings` should carry start timecode, reel name, file
   name tokens, and the overwrite policy (section 17, "Comfort"). Today the
   overwrite policy is in contract 14's destination request and the rest
   belongs to `pm-export`'s preset model; decided with Phase 3b presets.
   Decided (Phase 3b): start timecode, drop-frame, and reel travel in
   `ExportSettings.delivery`; file name tokens are expanded by the sheet before
   the save panel opens, and the overwrite policy stays in contract
   14's destination request.
