<script lang="ts">
  let {
    kind,
    bare = false,
  }: {
    kind: 'extensions' | 'performance' | 'export';
    bare?: boolean;
  } = $props();

  const ariaLabel = $derived(
    kind === 'extensions'
      ? 'Powermove Settings, Extensions. Two sample effects are installed. Audio-reactive visuals is enabled and Motion tracking is off.'
      : kind === 'performance'
        ? 'Powermove performance comparison against After Effects on Apple silicon: 660 MB vs 4.10 GB RAM while editing, 65 fps vs 18 fps live scrubbing without RAM preview, and 2.8 s vs 10.2 s 1080p MP4 export.'
        : 'Powermove Export sheet with Video selected and MP4 output at full 1920 by 1080 resolution, 30 frames per second.'
  );
</script>

<div class="app-preview" class:bare role="img" aria-label={ariaLabel}>
  {#if kind === 'extensions'}
    <div class="preview-stage" aria-hidden="true">
      <div class="extension-settings">
        <div class="extensions-heading">Extensions</div>
        <p class="extensions-description">Manage extensions and turn them on or off.</p>
        <div class="extensions-filters">
          <div class="extensions-search">
            <svg aria-hidden="true" width="13" height="13" viewBox="0 0 16 16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
              <circle cx="6.75" cy="6.75" r="4.5" />
              <path d="m10.1 10.1 3.4 3.4" />
            </svg>
            <span>Search extensions</span>
          </div>
          <span class="preview-popup">
            All types
            <svg aria-hidden="true" viewBox="0 0 8 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round">
              <path d="m1 5 3-3 3 3M1 8l3 3 3-3" />
            </svg>
          </span>
        </div>
        <div class="preview-segments">
          <span class="selected">All 2</span>
          <span>Enabled 1</span>
          <span>Off 1</span>
          <span>Needs attention 0</span>
        </div>
        <div class="extensions-category">
          <svg aria-hidden="true" width="13" height="13" viewBox="0 0 16 16" stroke="currentColor" stroke-width="1.3" stroke-linecap="round">
            <path d="m2 14 8.5-8.5M6 10l2 2m4-9v3m-1.5-1.5h3M6 1v4M4 3h4m5 6v4m-2-2h4" />
          </svg>
          Effects · 2
        </div>
        <div class="extensions-list">
          <div class="extension-row">
            <span class="extension-name">Audio-reactive visuals</span>
            <span class="extension-source">AI-created</span>
            <span class="extension-toggle on"><i></i></span>
            <span class="preview-button">Details</span>
          </div>
          <div class="extension-row">
            <span class="extension-name">Motion tracking</span>
            <span class="extension-source">AI-created</span>
            <span class="extension-toggle"><i></i></span>
            <span class="preview-button">Details</span>
          </div>
        </div>
        <p class="extensions-summary">2 installed · 1 enabled · 1 off</p>
      </div>
    </div>
  {:else if kind === 'performance'}
    <div class="preview-stage" aria-hidden="true">
      <div class="perf-card">
        <div class="perf-head">
          <span class="perf-kicker">Apple silicon benchmark</span>
          <span class="perf-meta">Identical 1080p timelines</span>
        </div>
        <div class="perf-metrics">
          <div class="perf-metric">
            <div class="perf-metric-head">
              <span class="perf-metric-title">Memory while editing (50-layer timeline)</span>
              <span class="perf-badge">6.2× less RAM</span>
            </div>
            <div class="perf-bars">
              <div class="perf-bar-row pm">
                <span class="perf-app"><img class="perf-app-icon" src="/icons/powermove-app.png" alt="" width="18" height="18" />Powermove</span>
                <div class="perf-track"><div class="perf-fill" style="width:16.1%"></div></div>
                <span class="perf-val">660 MB</span>
              </div>
              <div class="perf-bar-row">
                <span class="perf-app"><img class="perf-app-icon" src="/icons/after-effects-app.png" alt="" width="18" height="18" />After Effects</span>
                <div class="perf-track"><div class="perf-fill" style="width:100%"></div></div>
                <span class="perf-val">4.10 GB</span>
              </div>
            </div>
          </div>

          <div class="perf-metric">
            <div class="perf-metric-head">
              <span class="perf-metric-title">Live scrubbing without RAM preview</span>
              <span class="perf-badge">3.6× faster</span>
            </div>
            <div class="perf-bars">
              <div class="perf-bar-row pm">
                <span class="perf-app"><img class="perf-app-icon" src="/icons/powermove-app.png" alt="" width="18" height="18" />Powermove</span>
                <div class="perf-track"><div class="perf-fill" style="width:100%"></div></div>
                <span class="perf-val">65 fps</span>
              </div>
              <div class="perf-bar-row">
                <span class="perf-app"><img class="perf-app-icon" src="/icons/after-effects-app.png" alt="" width="18" height="18" />After Effects</span>
                <div class="perf-track"><div class="perf-fill" style="width:27.5%"></div></div>
                <span class="perf-val">18 fps</span>
              </div>
            </div>
          </div>

          <div class="perf-metric">
            <div class="perf-metric-head">
              <span class="perf-metric-title">Exporting 1080p MP4 (50 layers)</span>
              <span class="perf-badge">3.6× faster</span>
            </div>
            <div class="perf-bars">
              <div class="perf-bar-row pm">
                <span class="perf-app"><img class="perf-app-icon" src="/icons/powermove-app.png" alt="" width="18" height="18" />Powermove</span>
                <div class="perf-track"><div class="perf-fill" style="width:27.5%"></div></div>
                <span class="perf-val">2.8 s</span>
              </div>
              <div class="perf-bar-row">
                <span class="perf-app"><img class="perf-app-icon" src="/icons/after-effects-app.png" alt="" width="18" height="18" />After Effects</span>
                <div class="perf-track"><div class="perf-fill" style="width:100%"></div></div>
                <span class="perf-val">10.2 s</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  {:else}
    <div class="preview-stage" aria-hidden="true">
      <div class="export-dialog">
        <div class="export-heading">
          Export
          <span class="export-close">
            <svg aria-hidden="true" width="11" height="11" viewBox="0 0 12 12" stroke="currentColor" stroke-width="1.25" stroke-linecap="round">
              <path d="m3 3 6 6m0-6L3 9" />
            </svg>
          </span>
        </div>
        <div class="export-tabs preview-segments">
          <span class="selected">Video</span>
          <span>Images</span>
          <span>Project</span>
        </div>
        <p class="export-hint">MP4, WebM or ProRes for delivery and editing.</p>
        <div class="export-section">Output</div>
        <div class="export-rows">
          <div class="export-row">
            <div>Format<p>Container and codec of the exported file.</p></div>
            <span class="preview-popup">MP4 · H.264 or HEVC<svg aria-hidden="true" viewBox="0 0 8 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="m1 5 3-3 3 3M1 8l3 3 3-3" /></svg></span>
          </div>
          <div class="export-row">
            <div>Quality<p>Video bitrate used for video exports.</p></div>
            <span class="preview-popup">High · 16 Mbps<svg aria-hidden="true" viewBox="0 0 8 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="m1 5 3-3 3 3M1 8l3 3 3-3" /></svg></span>
          </div>
          <div class="export-row">
            <div>Range<p>Export the work area or the whole composition.</p></div>
            <span class="preview-popup">Full composition<svg aria-hidden="true" viewBox="0 0 8 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="m1 5 3-3 3 3M1 8l3 3 3-3" /></svg></span>
          </div>
          <div class="export-row">
            <div>Resolution<p>Rendered size relative to the composition, or any pixel size.</p></div>
            <span class="preview-popup">Full · 1920×1080<svg aria-hidden="true" viewBox="0 0 8 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="m1 5 3-3 3 3M1 8l3 3 3-3" /></svg></span>
          </div>
          <div class="export-row">
            <div>Frame Rate<p>Frames written per second of exported video.</p></div>
            <span class="preview-popup">30 fps<svg aria-hidden="true" viewBox="0 0 8 13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"><path d="m1 5 3-3 3 3M1 8l3 3 3-3" /></svg></span>
          </div>
          <div class="export-row">
            <div>Destination</div>
            <div class="export-destination">
              <span>Summer in Lisbon.mp4</span>
              <span class="preview-button">Choose…</span>
            </div>
          </div>
        </div>
        <div class="export-footer">
          <span class="preview-button">Cancel</span>
          <span class="preview-button export-action">Export…</span>
        </div>
      </div>
    </div>
  {/if}
</div>
