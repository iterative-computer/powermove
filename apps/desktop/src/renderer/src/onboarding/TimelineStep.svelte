<script lang="ts">
  import type { OnboardingTimelineMode } from '../../../shared/creative-workspace';

  let { onback, oncontinue }: { onback: () => void; oncontinue: (mode: OnboardingTimelineMode) => void } = $props();
  let mode = $state<OnboardingTimelineMode>('layers');

  type Clip = { x: number; w: number; type: string; name: string };
  type Row = { label: string; clips: Clip[]; gap?: boolean };
  /* The same five clips in both previews, so the difference is only the layout.
     Clip colours follow the timeline's per-type palette (CLIP_TYPES in extensions/timeline/timeline.ts). */
  const title: Clip = { x: 96, w: 72, type: 'text', name: 'Title' };
  const lower: Clip = { x: 196, w: 70, type: 'text', name: 'Lower third' };
  const broll: Clip = { x: 172, w: 100, type: 'video', name: 'B-roll' };
  const interview: Clip = { x: 78, w: 92, type: 'video', name: 'Interview' };
  const music: Clip = { x: 78, w: 194, type: 'audio', name: 'Music' };
  const options: Array<{ id: OnboardingTimelineMode; name: string; rows: Row[] }> = [
    { id: 'layers', name: 'Layers', rows: [
      { label: '1  Title', clips: [title] },
      { label: '2  Lower third', clips: [lower] },
      { label: '3  B-roll', clips: [broll] },
      { label: '4  Interview', clips: [interview] },
      { label: '5  Music', clips: [music] }
    ] },
    { id: 'tracks', name: 'Tracks', rows: [
      { label: 'V2', clips: [title, lower] },
      { label: 'V1', clips: [interview, broll] },
      { label: 'A1', clips: [music], gap: true }
    ] }
  ];
  const ROW = 20;
  const rowY = (rows: Row[], index: number) => 8 + index * ROW + (rows.slice(0, index + 1).some(row => row.gap) ? 8 : 0);
  const height = (rows: Row[]) => rowY(rows, rows.length - 1) + 24;
  /* Audio clips carry a waveform in the audio tint, as on the real timeline. */
  const waveform = (clip: Clip, y: number) => {
    let d = '';
    for (let x = clip.x + 4; x < clip.x + clip.w - 3; x += 3) {
      const amp = 1.5 + 4 * Math.abs(Math.sin(x * 0.21) * Math.cos(x * 0.057));
      d += `M${x} ${(y + 8 - amp).toFixed(1)}V${(y + 8 + amp).toFixed(1)}`;
    }
    return d;
  };
</script>

<main class="timeline-step" aria-labelledby="timeline-title">
  <button class="back" type="button" aria-label="Back to welcome" onclick={onback}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
  </button>
  <h1 id="timeline-title" tabindex="-1">Pick your timeline.</h1>
  <p class="intro">Work the way you already do.<br />You can switch anytime from the timeline panel.</p>
  <div class="choices" role="radiogroup" aria-labelledby="timeline-title">
    {#each options as option}
      <label class="choice" class:selected={mode === option.id}>
        <input type="radio" name="timeline-mode" value={option.id} bind:group={mode} />
        <span class="label">
          <span class="check" aria-hidden="true">
            <svg viewBox="0 0 16 16"><path d="m4.5 8.25 2.25 2.25 4.75-5" /></svg>
          </span>
          <span class="name">{option.name}</span>
        </span>
        <span class="preview" aria-hidden="true">
          <svg viewBox="0 0 280 {height(option.rows)}">
            {#each option.rows as row, index}
              {@const y = rowY(option.rows, index)}
              <text class="track-label" x="8" y={y + 11.5}>{row.label}</text>
              <rect class="lane" x="74" y={y} width="202" height="16" rx="3" />
              {#each row.clips as clip}
                <rect class="clip {clip.type}" x={clip.x} y={y + 1} width={clip.w} height="14" rx="3" />
                {#if clip.type === 'audio'}<path class="wave audio" d={waveform(clip, y)} />
                {:else}<text class="clip-label {clip.type}" x={clip.x + 5} y={y + 11}>{clip.name}</text>{/if}
              {/each}
            {/each}
            <rect class="playhead" x="150" y="2" width="1.5" height={height(option.rows) - 4} rx=".75" />
          </svg>
        </span>
      </label>
    {/each}
  </div>
  <div class="actions">
    <button class="primary" type="button" onclick={() => oncontinue(mode)}>Continue</button>
  </div>
</main>

<style>
  .timeline-step { position: relative; z-index: 1; display: flex; flex-direction: column; align-items: center; width: min(560px, calc(100vw - 96px)); padding-top: 48px; text-align: center; -webkit-app-region: no-drag; }
  /* Same staggered entrance as the workspace step (welcome-enter in welcome.css). */
  .timeline-step > * { animation: welcome-enter 800ms cubic-bezier(0.16, 1, 0.3, 1) calc(240ms + var(--i, 0) * 120ms) backwards; }
  .timeline-step > h1 { --i: 1; }
  .timeline-step > .intro { --i: 2; }
  .timeline-step > .choices { --i: 3; }
  .timeline-step > .actions { --i: 4; }
  /* Lines up with the back arrow on the suitcase step, which sits at the top-left of a 340px column. */
  .back { position: absolute; top: 0; left: calc(50% - 170px); width: 32px; height: 32px; display: grid; place-items: center; background: transparent; color: var(--tx-3); }
  .back:hover { background: var(--ink-1); color: var(--tx); }
  h1 { margin: 0 0 12px; font-size: clamp(30px, 4vw, 36px); line-height: 1.1; font-weight: var(--fw-regular); letter-spacing: -.035em; white-space: nowrap; outline: none; }
  .intro { margin: 0; color: var(--tx-2); font-size: var(--fs-lg); line-height: 1.5; }
  .choices { display: flex; flex-direction: column; gap: 10px; width: 100%; margin-top: 28px; }
  .choice { position: relative; display: grid; grid-template-columns: 112px 1fr; align-items: center; gap: 16px; padding: 12px 12px 12px 18px; border-radius: var(--r-md); background: var(--ink-1); text-align: left; cursor: default; transition: background var(--dur-1) var(--ease), box-shadow var(--dur-1) var(--ease); }
  .choice:hover { background: var(--ink-2); }
  .choice.selected { background: var(--bg-float); box-shadow: var(--shadow-raise); }
  .choice:has(input:focus-visible) { outline: 2px solid var(--accent); outline-offset: 3px; }
  input { position: absolute; opacity: 0; pointer-events: none; }
  .label { display: flex; align-items: center; gap: 10px; }
  .name { color: var(--tx); font-size: var(--fs-lg); font-weight: var(--fw-medium); }
  /* Selected: accent disc with a check. Unselected: an empty disc in the same spot. */
  .check { flex: none; display: grid; place-items: center; width: 18px; height: 18px; border-radius: 50%; background: var(--ink-2); transition: background var(--dur-1) var(--ease); }
  .check svg { width: 14px; height: 14px; stroke: var(--on-accent); stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; opacity: 0; }
  .selected .check { background: var(--accent); }
  .selected .check svg { opacity: 1; }
  .preview { display: block; border-radius: var(--r-sm); background: var(--bg-window); }
  .preview svg { display: block; width: 100%; height: auto; }
  .choice:not(.selected) .preview svg { opacity: .7; transition: opacity var(--dur-1) var(--ease); }
  .choice:not(.selected):hover .preview svg { opacity: .9; }
  .track-label { fill: var(--tx-2); font: 500 9px 'Geist', sans-serif; }
  .lane { fill: var(--ink-1); }
  /* Same treatment as the real timeline: in light theme the type's body lifted toward paper with ink from the body;
     in dark theme the body itself with its light foreground. */
  .text { --body: #3A4756; --fg: #EAF0F6; }
  .video { --body: #2E4266; --fg: #E2EAF8; }
  .audio { --body: #25473C; --fg: #DDF5EA; }
  .clip { fill: color-mix(in srgb, var(--body), white 72%); }
  .clip-label { fill: color-mix(in srgb, var(--body), black 35%); font: 500 8.5px 'Geist', sans-serif; }
  :global([data-theme='dark']) .clip { fill: var(--body); }
  :global([data-theme='dark']) .clip-label { fill: var(--fg); }
  .wave { stroke: color-mix(in srgb, var(--body), white 18%); stroke-width: 1.5; stroke-linecap: round; }
  :global([data-theme='dark']) .wave { stroke: #5FC29C; }
  .playhead { fill: var(--accent); }
  .actions { display: flex; align-items: center; gap: 8px; margin-top: 28px; }
  svg { fill: none; stroke: none; }
  .back svg { width: 20px; height: 20px; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  button { border: 0; border-radius: var(--r-sm); font: 500 var(--fs-sm) 'Geist', sans-serif; cursor: default; transition: background var(--dur-1) var(--ease); }
  .primary { min-height: 38px; padding: 0 18px; color: var(--on-accent); background: var(--accent); }
  .primary:hover { background: var(--accent-hover); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  /* Short windows: narrower previews and tighter spacing so the whole step fits without scrolling. */
  @media (max-height: 600px) { .timeline-step { width: min(480px, calc(100vw - 96px)); padding-top: 40px; } .choices, .actions { margin-top: 20px; } }
  @media (prefers-reduced-motion: reduce) { .timeline-step > * { animation: none; } button, .choice { transition: none; } }
</style>
