# 4K video preview and portable saves — September 15, 2026

Auto and reduced preview resolutions use a disposable, alpha-preserving VP9 editing copy for video sources larger than 1920 pixels. Its longest edge is 1280 pixels and every frame is independently decodable. Full preview resolution, frame capture, cached-preview preparation and export retain the original source. The editing copy is prepared in the background through bounded native uploads and is never embedded in project files. It is rebuilt on reopening; a failed conversion falls back to the original.

The transport now chooses an active clip before controlling a decoder shared with inactive copies. Changing to another instance resynchronizes its source time. Paused seeks coalesce to the latest requested time instead of repeatedly cancelling an in-flight decode; completed seeks invalidate the texture immediately. Offline frame preparation cancels pending preview seeks before taking decoder ownership.

An isolated Electron profile using the reported 190,624,657-byte, 2160×3840 WebM measured adjacent frame seeks of up to 325 ms before editing copies were enabled. The final run presented 60 frames in two seconds, with 15 forward/backward steps between 6.6 and 17.4 ms. The prepared export frame remained 2160×3840 and the stored original retained its exact byte length. These are local observations, not universal latency guarantees. The private media is not committed.

Portable PMV3 files have no application-imposed total size cap. Blob-backed packing retains original media without first reading every asset into renderer memory. Native Save spools acknowledged 1 MiB chunks to disk, then streams an atomic replacement. Native Open parses the metadata header and imports bounded media ranges through temporary disk files. Backups and overwrite-conflict checks remain enforced. A 257 MiB source successfully saved, backed up and reopened with its full payload. A subsequent isolated test opened and saved a source larger than 1 GiB, preserving undo-only media and its byte length/tail sentinel; peak main-process external-memory growth was about 68 MiB. Disk capacity and metadata parsing remain practical constraints (see project-files.md).

Validation: 111 focused unit tests, clean renderer/main typechecking and all 10 extension boundary checks. The final five Electron checks passed for 4K playback/stepping, inactive duplicate clips, large portable files, transparent previews and original video-frame export. The wider regression run also passed audio playback, video/audio persistence, native save cancellation, failed writes, external-file conflicts and quit behavior. The existing 120 fps image-sequence test intermittently returned the wrong frame; it failed once in three runs both with the changes and with the original transport/compositor/frame-preparation code. That existing edge case is not claimed fixed here.

```sh
PM_VIDEO_FIXTURE=/absolute/path/to/private-4k.webm \
  bun run --cwd apps/desktop test:e2e -- video-playback-regression.spec.ts
bun run --cwd apps/desktop test:e2e -- project-save-large-media.spec.ts import-prores-alpha.spec.ts export-video-frames.spec.ts
```
