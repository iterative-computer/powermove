<script lang="ts">
  import { onMount } from 'svelte';
  import type { OnboardingTranscriptionBridge, TranscriptionModelInfo, TranscriptionStatus } from '../../../shared/transcription';
  import ModelMeters from '../transcription/ModelMeters.svelte';
  import { formatBytes } from '../transcription/format';

  /* Optional: a speech model for captions and the agent. Choosing one starts
     its download in main right away; it keeps going after onboarding ends. */
  let { bridge, onback, oncontinue }: {
    bridge: OnboardingTranscriptionBridge | null;
    onback: () => void;
    oncontinue: () => void;
  } = $props();

  let status = $state<TranscriptionStatus | null>(null);
  let chosen = $state<string | null>(null);
  /** The download this step started, so "Not now" after going back can undo it. */
  let started = $state<string | null>(null);

  const models = $derived(status?.models ?? []);
  const ready = $derived(models.find((model) => model.id === status?.activeModelId) ?? null);
  const selected = $derived(models.find((model) => model.id === chosen)
    ?? ready ?? models.find((model) => model.state === 'downloading') ?? models.find((model) => model.recommended) ?? models[0] ?? null);

  function detail(model: TranscriptionModelInfo): string {
    if (model.state === 'ready') return `${model.description} Downloaded.`;
    if (model.state === 'downloading') return `${model.description} Downloading…`;
    return model.description;
  }

  async function choose(): Promise<void> {
    const model = selected;
    if (model && bridge && model.state !== 'ready' && model.state !== 'downloading') {
      started = model.id;
      void bridge.download(model.id).catch(() => undefined);
    }
    oncontinue();
  }

  function skip(): void {
    if (started && bridge) void bridge.cancelDownload(started).catch(() => undefined);
    started = null;
    oncontinue();
  }

  onMount(() => {
    if (!bridge) return;
    const off = bridge.onStatus((next) => { status = next; });
    void bridge.status().then((next) => { status = next; }).catch(() => undefined);
    return off;
  });
</script>

<main class="transcription-step" aria-labelledby="transcription-title">
  <button class="back" type="button" aria-label="Back to agent choice" onclick={onback}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
  </button>
  <h1 id="transcription-title" tabindex="-1">Pick your speech model.</h1>
  <p class="intro">Captions and your agent can transcribe speech on this Mac.<br />It’s optional, and the download runs while you work.</p>
  {#if models.length}
    <div class="choices" role="radiogroup" aria-labelledby="transcription-title">
      {#each models as model (model.id)}
        {@const isSelected = selected?.id === model.id}
        <label class="choice" class:selected={isSelected}>
          <input type="radio" name="transcription-model" value={model.id} checked={isSelected} onchange={() => { chosen = model.id; }} />
          <span class="check" aria-hidden="true">
            <svg viewBox="0 0 16 16"><path d="m4.5 8.25 2.25 2.25 4.75-5" /></svg>
          </span>
          <span class="text">
            <span class="name">{model.name}{#if model.recommended}<span class="tag">Recommended</span>{/if}</span>
            <span class="detail">{detail(model)}</span>
          </span>
          <span class="facts">
            <ModelMeters speed={model.speed} accuracy={model.accuracy} />
            <span class="size">{formatBytes(model.size)}</span>
          </span>
        </label>
      {/each}
    </div>
  {:else}
    <p class="unavailable">Speech models can be downloaded later from Settings.</p>
  {/if}
  <div class="actions">
    {#if models.length && selected}
      <button class="primary" type="button" onclick={choose}>{selected.state === 'ready' || selected.state === 'downloading' ? 'Continue' : 'Download'}</button>
    {/if}
    <button class="secondary" type="button" onclick={skip}>{models.length ? 'Not now' : 'Continue'}</button>
  </div>
  <p class="note">Runs on this Mac. Your audio is never uploaded.</p>
</main>

<style>
  .transcription-step { position: relative; z-index: 1; display: flex; flex-direction: column; align-items: center; width: min(560px, calc(100vw - 96px)); padding-top: 48px; text-align: center; -webkit-app-region: no-drag; }
  /* Same staggered entrance as the other steps (welcome-enter in welcome.css). */
  .transcription-step > * { animation: welcome-enter 800ms cubic-bezier(0.16, 1, 0.3, 1) calc(240ms + var(--i, 0) * 120ms) backwards; }
  .transcription-step > h1 { --i: 1; }
  .transcription-step > .intro { --i: 2; }
  .transcription-step > .choices, .transcription-step > .unavailable { --i: 3; }
  .transcription-step > .actions, .transcription-step > .note { --i: 4; }
  /* Lines up with the back arrow on the other steps. */
  .back { position: absolute; top: 0; left: calc(50% - 170px); width: 32px; height: 32px; display: grid; place-items: center; background: transparent; color: var(--tx-3); }
  .back:hover { background: var(--ink-1); color: var(--tx); }
  h1 { margin: 0 0 12px; font-size: clamp(30px, 4vw, 36px); line-height: 1.1; font-weight: var(--fw-regular); letter-spacing: -.035em; white-space: nowrap; outline: none; }
  .intro { margin: 0; color: var(--tx-2); font-size: var(--fs-lg); line-height: 1.5; }
  .choices { display: flex; flex-direction: column; gap: 10px; width: 100%; margin-top: 28px; }
  .choice { position: relative; display: flex; align-items: center; gap: 12px; min-height: 64px; box-sizing: border-box; padding: 12px 16px 12px 18px; border-radius: var(--r-md); background: var(--ink-1); text-align: left; cursor: default; transition: background var(--dur-1) var(--ease), box-shadow var(--dur-1) var(--ease); }
  .choice:hover { background: var(--ink-2); }
  .choice.selected { background: var(--bg-float); box-shadow: var(--shadow-raise); }
  /* Keyboard focus is a neutral edge, never the accent. */
  .choice:has(input:focus-visible) { box-shadow: var(--shadow-raise), 0 0 0 2px var(--line-strong); }
  input { position: absolute; opacity: 0; pointer-events: none; }
  /* Selected: accent disc with a check, as on the timeline and agent steps. */
  .check { flex: none; display: grid; place-items: center; width: 18px; height: 18px; border-radius: 50%; background: var(--ink-2); transition: background var(--dur-1) var(--ease); }
  .check svg { width: 14px; height: 14px; stroke: var(--on-accent); stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; opacity: 0; }
  .selected .check { background: var(--accent); }
  .selected .check svg { opacity: 1; }
  .text { display: grid; gap: 2px; flex: 1; min-width: 0; }
  .name { display: flex; align-items: center; gap: 7px; color: var(--tx); font-size: var(--fs-lg); font-weight: var(--fw-medium); }
  .tag { padding: 0 6px; border-radius: var(--r-xs); background: var(--ink-1); color: var(--tx-3); font-size: var(--fs-xs); font-weight: var(--fw-regular); line-height: 17px; }
  .detail { color: var(--tx-3); font-size: var(--fs-sm); line-height: 1.4; }
  .facts { flex: none; display: flex; align-items: center; gap: 14px; }
  .size { min-width: 52px; color: var(--tx-3); font-size: var(--fs-sm); font-variant-numeric: tabular-nums; text-align: right; }
  .unavailable { margin: 28px 0 0; color: var(--tx-3); font-size: var(--fs-sm); }
  .actions { display: flex; align-items: center; gap: 8px; margin-top: 28px; }
  .note { margin: 12px 0 0; color: var(--tx-4); font-size: var(--fs-sm); }
  svg { fill: none; stroke: none; }
  .back svg { width: 20px; height: 20px; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  button { border: 0; border-radius: var(--r-sm); font: 500 var(--fs-sm) 'Geist', sans-serif; cursor: default; transition: background var(--dur-1) var(--ease); }
  .primary, .secondary { min-height: 38px; padding: 0 18px; }
  .primary { color: var(--on-accent); background: var(--accent); }
  .primary:hover { background: var(--accent-hover); }
  .secondary { color: var(--tx-2); background: transparent; }
  .secondary:hover { color: var(--tx); background: var(--ink-1); }
  button:focus-visible { outline: 2px solid var(--line-strong); outline-offset: 3px; }
  /* Short windows: tighter spacing so the whole step fits without scrolling. */
  @media (max-height: 600px) { .transcription-step { width: min(500px, calc(100vw - 96px)); padding-top: 40px; } .choices, .actions { margin-top: 20px; } .choice { min-height: 56px; padding-block: 9px; } }
  @media (prefers-reduced-motion: reduce) { .transcription-step > * { animation: none; } button, .choice { transition: none; } }
</style>
