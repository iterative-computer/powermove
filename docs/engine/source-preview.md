# Source preview monitor (C5)

The viewer's source monitor is `apps/desktop/src/extensions/viewer/source-preview.ts`.
It retains the shipped browser image, video and audio monitor, including the
live asset fields required by older extensions. C5 is not complete: engine
frames and source audio cannot replace these consumers until the bridge
exposes them. No unavailable placeholder was shipped in place of the monitor.

`pm_media::preview::PreviewReader` can decode an exact source-time frame and
return duration and video facts (see `media.md`), but the interactive engine
does not expose it to the renderer. `pm-protocol::Request` has `probeMedia`
for source facts and `renderFrame` / `agentFrames` / snapshots for *composition*
frames; it has no source-frame request, source playback stream, or independent
source-audio output. The preload and renderer connection likewise expose
media facts and composition snapshots only. Sampling those snapshots would
display the current composition rather than the selected asset. The agent
media tools use main-process FFmpeg sampling and are not an interactive
native monitor contract.

To finish C5, the engine/bridge owner needs a session-scoped source monitor
API: asset handle, source time and interpretation for exact frame requests;
duration and native frame/color facts; cancellation and generation tagging
for rapid seeks and asset switches; and a separate source-audio voice tied to
the same play/seek clock without restarting composition audio. The renderer
then needs a frame delivery and presentation surface that can display those
frames without creating a browser media element. Source in/out, J/K/L,
reverse playback and unsupported-browser codecs require end-to-end tests once
that contract exists. The current source monitor has only play/pause and
scrub controls, so those transport features are also outstanding.

The existing `source-preview.test.ts` remains unchanged and covers the shipped
monitor. Engine-source acceptance is blocked on that contract; a green legacy
unit test is not evidence of native source parity.
