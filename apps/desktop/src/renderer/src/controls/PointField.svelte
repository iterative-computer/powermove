<script lang="ts">
  import NumField from './NumField.svelte';
  import type { Snippet } from 'svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { visualDrag } from './visual-drag';
  import { doc } from '../state/document.svelte';
  import { controlTime } from '../state/transport.svelte';
  import type { PowermoveAPI } from '../kernel/api';
  let { api, get, edit, xEdit, yEdit, label = 'Position', xLabel = 'X', yLabel = 'Y', minX = -100, maxX = 100, minY = -100, maxY = 100, step = 1, unit = '', xAction, yAction, xField, yField }:
    { api: PowermoveAPI; get: () => [number, number]; edit: EditBinding; xEdit: EditBinding; yEdit: EditBinding; label?: string; xLabel?: string; yLabel?: string; minX?: number; maxX?: number; minY?: number; maxY?: number; step?: number; unit?: string; xAction?: Snippet; yAction?: Snippet; xField?: Snippet; yField?: Snippet } = $props();
  const value = $derived((doc.tick.values, doc.proj, controlTime(), get()));
  const clamp = (n:number,min:number,max:number) => Math.max(min,Math.min(max,Number((Math.round(n / step) * step).toFixed(6))));
  function key(event: KeyboardEvent) {
    if (!['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const [x,y] = get(), n = step * (event.shiftKey ? 10 : 1);
    new EditGesture(api,edit).once(event.key === 'Home' ? [clamp(0,minX,maxX),clamp(0,minY,maxY)] : [clamp(x + (event.key==='ArrowLeft'?-n:event.key==='ArrowRight'?n:0),minX,maxX),clamp(y + (event.key==='ArrowUp'?-n:event.key==='ArrowDown'?n:0),minY,maxY)]);
  }
</script>
<div class="point-field">
  <button type="button" class="point-pad" aria-roledescription="position control" aria-label={`${label}: ${value[0]}${unit}, ${value[1]}${unit}. Use arrow keys to move.`} onkeydown={key}
    use:visualDrag={{api,edit,get,value:(event,bounds)=>[clamp(minX+(event.clientX-bounds.left)/bounds.width*(maxX-minX),minX,maxX),clamp(minY+(event.clientY-bounds.top)/bounds.height*(maxY-minY),minY,maxY)]}}>
    <span class="point-dot" style:left={`${(value[0]-minX)/(maxX-minX)*100}%`} style:top={`${(value[1]-minY)/(maxY-minY)*100}%`}></span>
  </button>
  <div class="point-values">
    <div class="point-axis">{#if xAction}{@render xAction()}{:else}<span>X</span>{/if}{#if xField}{@render xField()}{:else}<NumField {api} get={()=>get()[0]} edit={xEdit} label={xLabel} ariaLabel={xLabel} min={minX} max={maxX} {step} {unit} />{/if}</div>
    <div class="point-axis">{#if yAction}{@render yAction()}{:else}<span>Y</span>{/if}{#if yField}{@render yField()}{:else}<NumField {api} get={()=>get()[1]} edit={yEdit} label={yLabel} ariaLabel={yLabel} min={minY} max={maxY} {step} {unit} />{/if}</div>
  </div>
</div>
<style>
  .point-field { width:100%; min-width:0; display:flex; align-items:center; gap:8px; }
  .point-pad { position:relative; flex:1; min-width:70px; height:calc(var(--ctl-h) * 2); padding:0; border:0; background:var(--ink-1); border-radius:var(--r-sm); touch-action:none; cursor:crosshair; outline:none; }
  .point-pad::before,.point-pad::after { content:''; position:absolute; background:var(--ink-3); }
  .point-pad::before { top:50%; left:0; right:0; height:1px; }
  .point-pad::after { left:50%; top:0; bottom:0; width:1px; }
  .point-dot { position:absolute; width:8px; height:8px; background:var(--tx); border-radius:50%; transform:translate(-50%,-50%); pointer-events:none; }
  .point-values { width:90px; display:flex; flex-direction:column; gap:4px; }
  .point-axis { display:flex; align-items:center; gap:3px; min-width:0; }
  .point-axis > span { color:var(--tx-3); font-size:var(--fs-xs); }
  .point-axis :global(.well) { min-width:0; flex:1; }
  .point-pad:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
</style>
