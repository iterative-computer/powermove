<script lang="ts">
  import NumField from './NumField.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { visualDrag } from './visual-drag';
  import { doc } from '../state/document.svelte';
  import { controlTime } from '../state/transport.svelte';
  import type { PowermoveAPI } from '../kernel/api';
  let { api, get, edit, min = 0, max = 100, step = 1, unit = '', precision, label = 'Value', mixed, angle = false, link = false }:
    { api: PowermoveAPI; get: () => unknown; edit: EditBinding; min?: number; max?: number; step?: number; unit?: string; precision?: number; label?: string; mixed?: (edit: EditBinding, value: unknown) => boolean; angle?: boolean; link?: boolean } = $props();
  const value = $derived((doc.tick.values, doc.proj, controlTime(), Number(get())));
  let surface: HTMLDivElement;
  const dial = $derived(angle && max - min >= 360);
  const interval = $derived(Number.isFinite(step) && step > 0 ? step : 1);
  const fraction = $derived(Math.max(0, Math.min(100, (value - min) / (max - min) * 100)));
  const snap = (n: number) => Math.max(min, Math.min(max, Number((min + Math.round((n - min) / interval) * interval).toFixed(6))));
  function dialValue(degrees: number) {
    // atan2 wraps at ±180; pick the equivalent turn nearest the current value.
    let next = degrees + Math.round((Number(get()) - degrees) / 360) * 360;
    while (next < min) next += 360;
    while (next > max) next -= 360;
    return next;
  }
  function key(event: KeyboardEvent) {
    let next: number;
    if (event.key === 'Home') next = min;
    else if (event.key === 'End') next = max;
    else if (['ArrowLeft','ArrowDown','ArrowRight','ArrowUp'].includes(event.key)) next = Number(get()) + (event.key === 'ArrowLeft' || event.key === 'ArrowDown' ? -1 : 1) * interval * (event.shiftKey ? 10 : 1);
    else return;
    event.preventDefault(); event.stopPropagation(); new EditGesture(api, edit).once(snap(next));
  }
</script>
<div bind:this={surface} class="visual-number" class:angle={dial} class:filled-slider={!dial} style={`--fraction:${fraction}%;--angle:${value}deg`}>
  {#if !dial}<span class="visual-track-fill" aria-hidden="true"></span>{/if}
  <div class="visual-track" class:dial tabindex="0" role="slider" aria-label={`${label} slider`} aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} aria-valuetext={`${value}${unit}`} onkeydown={key}
    use:visualDrag={{ api, edit, get, bounds:node=>(dial?node:surface).getBoundingClientRect(), value: (event, bounds) => snap(dial ? dialValue(Math.atan2(event.clientY - bounds.top - bounds.height / 2, event.clientX - bounds.left - bounds.width / 2) * 180 / Math.PI) : min + Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)) * (max - min)) }}>
    {#if dial}<span class="dial-hand"></span>{/if}
  </div>
  <NumField {api} {get} {edit} {min} {max} {step} {unit} {precision} {label} {mixed} {link} />
</div>
<style>
  .visual-number { display:flex; align-items:center; gap:10px; width:100%; min-width:0; }
  .visual-number :global(.num) { flex:none; width:68px; min-width:0; }
  .visual-track { position:relative; flex:1; height:var(--ctl-h); cursor:ew-resize; touch-action:none; outline:none; }
  .visual-track:focus-visible { outline:2px solid var(--accent); outline-offset:2px; border-radius:var(--r-sm); }
  .filled-slider { position:relative; gap:0; height:var(--ctl-h); border-radius:var(--r-sm); background:var(--ink-1); overflow:hidden; }
  .filled-slider:has(:focus-visible) { outline:2px solid var(--accent); outline-offset:2px; }
  .filled-slider .visual-track:focus-visible { outline:none; }
  .visual-track-fill { position:absolute; inset:0 auto 0 0; width:var(--fraction); max-width:100%; background:var(--ink-3); opacity:.75; pointer-events:none; }
  .filled-slider .visual-track { min-width:0; display:flex; align-items:center; }
  .filled-slider :global(input.num:not([type=range]):not([type=color]):not([type=file])) { position:relative; order:-1; flex:0 0 68px; width:68px; background:transparent; text-align:left; padding-inline:8px; }
  .filled-slider :global(input.num.editing) { background:var(--ink-1); border-radius:var(--r-sm); }
  .dial { width:var(--ctl-h); height:var(--ctl-h); flex:none; border-radius:50%; background:var(--ink-2); }
  .dial-hand { position:absolute; left:50%; top:calc(50% - 1px); height:2px; width:35%; background:var(--tx); transform-origin:0 50%; transform:rotate(var(--angle)); border-radius:1px; }
  .angle :global(.num) { flex:1; width:auto; }
</style>
