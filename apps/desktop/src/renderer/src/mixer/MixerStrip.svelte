<script lang="ts">
  /* One channel strip: level readout, fader beside a stereo meter on one
     shared dB scale, mute and solo, and the layer's name. The master strip is the
     same anatomy without mute and solo. */
  import type { PowermoveAPI } from '../kernel/api';
  import Fader from './Fader.svelte';
  import LevelReadout from './LevelReadout.svelte';
  import type { LevelControl } from './level-control';
  import { METER_MARKS, meterPosition, scaleLabel } from './levels';
  import type { MeterLoop } from './meter-loop';
  import type { MixerStrip } from './strips';

  let {
    api,
    id,
    name,
    gain,
    control,
    loop,
    strip = null,
    soloed = false,
    soloActive = false,
    selected = false,
    follow,
    liveGain,
    onMute,
    onSolo,
    onKey,
    onSelect
  }: {
    api: PowermoveAPI;
    /** Meter tap id: the layer id, or the master tap. */
    id: string;
    name: string;
    gain: number;
    control: LevelControl;
    loop: MeterLoop;
    strip?: MixerStrip | null;
    soloed?: boolean;
    soloActive?: boolean;
    selected?: boolean;
    follow?: (callback: () => void) => () => void;
    liveGain?: () => number;
    onMute?: () => void;
    onSolo?: (exclusive: boolean) => void;
    onKey?: () => void;
    onSelect?: () => void;
  } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let clipped = $state(false);

  const master = $derived(!strip);
  const locked = $derived(!!strip?.locked);
  /* Why this strip is quiet, in words, when its own controls don't say so. */
  const quiet = $derived.by(() => {
    if (!strip) return '';
    if (strip.muted) return 'Muted';
    if (strip.silencedBy === 'off') return 'Switched off on the timeline';
    if (strip.silencedBy === 'remap') return 'Time remapping mutes the sound';
    if (strip.silencedBy === 'solo') return 'Silenced by a soloed layer';
    if (soloActive && !soloed) return 'Silenced by solo';
    return '';
  });
  const readoutLabel = $derived(`${name} level in decibels`);

  $effect(() => {
    if (!canvas) return;
    const element = canvas;
    const detach = loop.attach(id, element, (value) => { clipped = value; });
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      loop.resize(id, entry.contentRect.width, entry.contentRect.height, window.devicePixelRatio || 1);
    });
    observer.observe(element);
    return () => { observer.disconnect(); detach(); };
  });

  /* One scale per strip: the labels read the fader and the meter alike. The
     in-between marks drop out on short strips. */
  const scale = METER_MARKS.map((mark) => ({ mark, label: scaleLabel(mark), position: meterPosition(mark), minor: mark === 6 || mark === -6 || mark === -30 || mark === -40 }));
</script>

<div class="mx-strip" class:master class:quiet={!!quiet} class:selected data-mixer-strip={id} title={quiet || undefined}>
  <div class="mx-top">
    {#if strip}
      <button
        type="button"
        class="mx-key"
        class:on={strip.animated}
        class:at-key={strip.keyAtTime}
        disabled={locked}
        aria-pressed={strip.keyAtTime}
        aria-label={strip.keyAtTime ? `Remove ${name} level keyframe` : `Add ${name} level keyframe`}
        title={strip.keyAtTime ? 'Remove keyframe' : 'Add keyframe'}
        onclick={() => onKey?.()}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 12 12 21 4 12Z" /></svg>
      </button>
    {/if}
    <LevelReadout {api} {gain} {control} label={readoutLabel} disabled={locked} />
  </div>

  <div class="mx-body">
    <Fader {api} {gain} {control} label={`${name} level`} disabled={locked} {follow} {liveGain} />
    <div class="mx-meter">
      <button
        type="button"
        class="mx-clip"
        class:on={clipped}
        aria-label={clipped ? `${name} clipped. Clear` : `${name} has not clipped`}
        title={clipped ? 'Clipped · click to clear' : 'Clip indicator'}
        onclick={() => loop.clearClip(id)}
      ></button>
      <canvas bind:this={canvas} class="mx-canvas" aria-hidden="true"></canvas>
    </div>
    <div class="mx-scale" aria-hidden="true">
      {#each scale as mark (mark.mark)}
        <span class:minor={mark.minor} class:unity={mark.mark === 0} style:bottom={`${mark.position * 100}%`}>{mark.label}</span>
      {/each}
    </div>
  </div>

  <div class="mx-switches">
    {#if strip}
      <button
        type="button"
        class="mx-switch mute"
        class:on={strip.muted}
        disabled={strip.muteAnimated}
        aria-pressed={strip.muted}
        aria-label={`Mute ${name}`}
        title={strip.muteAnimated ? 'Mute is animated on the timeline' : strip.muted ? 'Unmute' : 'Mute'}
        onclick={() => onMute?.()}
      >M</button>
      <button
        type="button"
        class="mx-switch solo"
        class:on={soloed}
        aria-pressed={soloed}
        aria-label={`Solo ${name}`}
        title={soloed ? 'Unsolo' : 'Solo while listening · Option-click to hear only this'}
        onclick={(event) => onSolo?.(event.altKey)}
      >S</button>
    {/if}
  </div>

  {#if strip}
    <button type="button" class="mx-name" title={name} onclick={() => onSelect?.()}>{name}</button>
  {:else}
    <span class="mx-name">{name}</span>
  {/if}
</div>

<style>
  .mx-strip {
    flex: none;
    width: 62px;
    height: 100%;
    min-height: 0;
    display: grid;
    grid-template-rows: 20px minmax(0, 1fr) 18px 18px;
    row-gap: 8px;
    justify-items: center;
    padding: 10px 0 8px;
  }

  .mx-top {
    position: relative;
    width: 100%;
    display: flex;
    justify-content: center;
  }

  .mx-key {
    position: absolute;
    left: 0;
    top: 1px;
    width: 14px;
    height: 18px;
    display: grid;
    place-items: center;
    padding: 0;
    border-radius: var(--r-xs);
    color: var(--tx-4);
    opacity: 0;
    transition: opacity var(--dur-1), color var(--dur-1), background var(--dur-1);
  }

  .mx-strip:hover .mx-key,
  .mx-key:focus-visible,
  .mx-key.on { opacity: 1; }
  .mx-key:hover { color: var(--tx-2); }
  .mx-key.on { color: var(--accent); }
  .mx-key:disabled { visibility: hidden; }

  .mx-key svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 2; }
  .mx-key.at-key svg { fill: currentColor; }

  .mx-body {
    height: 100%;
    min-height: 0;
    display: grid;
    grid-template-columns: 18px 11px 16px;
    column-gap: 5px;
  }

  /* The fader, the meter and the scale share one geometry: the knob's centre
     travels from 6px below the top to half a knob (4.5px) above the bottom,
     and the meter canvas and scale span exactly that, so a mark, the meter
     at that level and the knob at that level sit on one line. */
  .mx-meter {
    min-height: 0;
    display: grid;
    grid-template-rows: 3px minmax(0, 1fr);
    row-gap: 3px;
    padding-bottom: 4.5px;
  }

  .mx-body > :global(.mx-fader) { margin-top: 1.5px; height: calc(100% - 1.5px); }

  .mx-clip {
    position: relative;
    width: 100%;
    height: 3px;
    padding: 0;
    border-radius: 1px;
    background: var(--ink-2);
    transition: background var(--dur-1);
  }

  .mx-clip::before { content: ''; position: absolute; inset: -5px -3px -3px; }
  .mx-clip.on { background: var(--danger); }
  .mx-clip:focus-visible { box-shadow: 0 0 0 1.5px color-mix(in srgb, var(--tx) 40%, transparent); outline: none; }

  .mx-canvas {
    display: block;
    width: 100%;
    height: 100%;
    min-height: 0;
    border-radius: 2px;
  }

  .mx-scale {
    position: relative;
    margin: 6px 0 4.5px;
    min-height: 0;
  }

  .mx-scale span {
    position: absolute;
    left: 0;
    transform: translateY(50%);
    font: var(--fw-regular) 9.5px / 1 var(--f-ui);
    font-variant-numeric: tabular-nums;
    color: var(--tx-4);
    white-space: nowrap;
  }

  .mx-scale span.unity { color: var(--tx-3); }

  .mx-switches {
    display: flex;
    gap: 4px;
  }

  .mx-switch {
    width: 22px;
    height: 18px;
    padding: 0;
    border-radius: var(--r-xs);
    background: var(--ink-1);
    color: var(--tx-3);
    font: var(--fw-semibold) 10px / 18px var(--f-ui);
    text-align: center;
    transition: background var(--dur-1), color var(--dur-1);
  }

  .mx-switch:hover:not(:disabled) { background: var(--ink-2); color: var(--tx-2); }
  .mx-switch:disabled { opacity: var(--disabled); }
  .mx-switch.mute.on { background: color-mix(in srgb, var(--blue) 20%, transparent); color: var(--blue); }
  .mx-switch.solo.on { background: color-mix(in srgb, var(--warning) 24%, transparent); color: var(--warning); }

  .mx-switch:focus-visible,
  .mx-name:focus-visible,
  .mx-key:focus-visible {
    outline: none;
    box-shadow: 0 0 0 1.5px color-mix(in srgb, var(--tx) 40%, transparent);
  }

  .mx-name {
    max-width: 58px;
    height: 18px;
    padding: 0 3px;
    border-radius: var(--r-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--fw-regular) var(--fs-xs) / 18px var(--f-ui);
    color: var(--tx-2);
    text-align: center;
    transition: color var(--dur-1);
  }

  button.mx-name:hover { color: var(--tx); }
  .mx-strip.selected .mx-name { color: var(--tx); font-weight: var(--fw-medium); }
  .mx-strip.master .mx-name { color: var(--tx); font-weight: var(--fw-medium); }

  /* A silent strip recedes; its controls stay readable and usable. */
  .mx-strip.quiet .mx-meter,
  .mx-strip.quiet .mx-scale { opacity: .45; }
  .mx-strip.quiet .mx-name { color: var(--tx-4); }

  @container mixer (max-height: 230px) {
    .mx-scale span.minor { display: none; }
  }

  @container mixer (max-height: 170px) {
    .mx-strip { row-gap: 5px; padding-top: 6px; }
  }
</style>
