# Phase 4a implementation and gate status

Phase 4a is **not accepted for cutover**. This change implements the proxy and
cache foundations and desktop controls, but the outstanding integration and
measurement gates below must close before claiming the phase complete.

## Observed checks

| Check | Result |
|---|---|
| Whole-clip proxy generation and store | 9 tests pass: ProRes Proxy/LT, HEVC10, alpha 4444, exact VFR PTS, odd raster/display aspect, s24 and f64 PCM, source/store identity, obsolete recipes and disk threshold |
| Generation job service | Real host-capability resolution, playback pause/resume and persisted output test passes |
| Original decode measurement | 3 media tests and 1 job-fact test pass; insufficient samples remain explicitly unknown |
| Presenter | 75 unit tests pass, including machine-profile lifecycle; a later real-GPU run observed a positive 78.042 microsecond timestamp span |
| Reduced preview/export isolation | Renderer export buffers are identical before and after reduced ProRes previews |
| GPU preview/export isolation | Automatic, Half and Quarter cache keys remain separate; full-quality export buffers are byte-identical before and after warming |
| Memory cache | 3 tests pass for lease shrink, zero budget, oversized entries and clock wrap |
| Desktop integration | 117 tests in 8 focused files pass; earlier UI regression run passed 464 tests in 41 files |
| Presentation surface lifecycle | 5 cached-preview tests pass, including 30 warm frames through a simulated three-slot pool; the session regression run passes 23 tests |
| Protocol | 11 wire tests pass, including new events, golden fixtures and deterministic TypeScript generation; the existing opt-in codec timing measurement was not run |
| App and native bridge builds | Pass; the native bridge matches the generated protocol build |
| Svelte check | Zero errors; one existing IconCropper accessibility warning |
| Hidden-app gates | Engine off passes. Engine on times out waiting 90 seconds for a proxy to become Ready; export identity with a ready proxy and cached preview are therefore not proven in the app |
| Half-scale video parity | Fails: maximum channel difference 204 CV; 228,586 of 230,400 pixels differ by more than 1 CV. Current/native captures are retained outside Git |

The node TypeScript check retains two existing TS6307 cross-project file-list
errors in canonical JSON/composition imports. The built-in version check
retains the integration base's inspector, keymap-default and timeline version
errors. Viewer is bumped to 1.3.0 and its lock entry is updated.

App gates live in `engine-proxies.spec.ts` (both engine settings, original-only
export identity, real generation/deletion and cached preview) and
`engine-proxy-viewer-parity.spec.ts` (paired half-scale video). These require
the native add-ons, the WASM document module and app assets to be built first.
All generated media, profiles and captured evidence stay outside the repository;
tests remove their temporary profiles and source/output files on failure too.

## Remaining integration gates

- The standalone automatic policy accepts measured original, reduced and proxy
  costs, GPU headroom, device-pixel size and power state. The live viewer still
  selects from measured-slow originals, readiness and user intent; it does not
  yet feed all measured costs and power changes through that complete policy.
- The runtime disk probe measures the default generated-media volume. Changing
  generated-media location to a different volume does not yet remeasure it.
- The tiled mip cache is implemented and bounded, but the full-layer renderer
  still uploads a reduced whole image instead of requesting visible tiles.
- Existing converted image-sequence assets are eligible for whole-clip proxies;
  a native producer that reads original sequence files is not implemented here.
- Generation uses a serial utility lane and pauses for playback/export/low disk.
  Submission order is not yet reordered by distance from the playhead.
- Drop diagnosis with the Create Proxies action is not implemented. Generation
  progress and named failures are wired; failed cold measurements are logged.
- Service-level low-disk pause/resume simulation remains untested; the real
  generation guard and its threshold unit test are present.
- Full cache-contamination coverage with Blender preview settings and sustained
  load, the chart DeltaE2000 bar, paired 4K codec performance, paused-upgrade
  latency and the one-hour memory loop are not measured by this change.
- The viewer performs a complete proxy integrity check on its first store
  lookup after a file identity change. That cold check is synchronous and
  needs to move off the rendering path before large-clip playback can meet
  the no-wait policy.

The two additional reference-machine runs remain pending beta. Existing-project
viewer parity must pass without routing, changing references or widening a
tolerance. A visible difference blocks this phase even when export identity
and the unit tests pass.
