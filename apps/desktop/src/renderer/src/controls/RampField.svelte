<script lang="ts">
  import { EditGesture, type EditBinding } from './gesture';
  import { visualDrag } from './visual-drag';
  import { doc } from '../state/document.svelte';
  import { controlTime } from '../state/transport.svelte';
  import type { PowermoveAPI } from '../kernel/api';
  import type { FillStop } from './control-utils';
  let { api, get, edit, label = 'Gradient', selected = '', onSelect, midpoint, midpointEdit }:
    { api: PowermoveAPI; get: () => FillStop[]; edit?: EditBinding; label?: string; selected?: string; onSelect?: (id:string)=>void; midpoint?: ()=>number; midpointEdit?: EditBinding } = $props();
  const stops = $derived((doc.tick.values, doc.proj, controlTime(), get()));
  const middle = $derived((doc.tick.values, doc.proj, controlTime(), midpoint?.() ?? 50));
  const background = $derived(`linear-gradient(90deg,${midpoint && stops.length===2 ? `${stops[0]!.color} 0%,${middle}%,${stops[1]!.color} 100%` : [...stops].sort((a,b)=>a.position-b.position).map(stop=>`${stop.color} ${stop.position}%`).join(',')})`);
  const clamp = (n:number) => Math.max(0,Math.min(100,Math.round(n * 10)/10));
  function dragStop(node: HTMLElement, id: string) {
    if (!edit) return {};
    // Bind directly to the fill's transaction; all stops remain one value.
    return visualDrag(node,{api,edit,get,value:(event)=>{
      const bounds=node.parentElement!.getBoundingClientRect();
      return get().map(stop=>stop.id===id ? {...stop,position:clamp((event.clientX-bounds.left)/bounds.width*100)} : {...stop});
    }});
  }
  function key(event:KeyboardEvent, stop:FillStop) {
    if (!edit || !['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const position=event.key==='Home'?0:event.key==='End'?100:clamp(stop.position+(event.key==='ArrowLeft'?-1:1)*(event.shiftKey?10:1));
    new EditGesture(api,edit).once(get().map(item=>item.id===stop.id?{...item,position}:{...item}));
  }
  function middleKey(event:KeyboardEvent) {
    if (!midpointEdit || !['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    new EditGesture(api,midpointEdit).once(event.key==='Home'?1:event.key==='End'?99:Math.max(1,Math.min(99,middle+(event.key==='ArrowLeft'?-1:1)*(event.shiftKey?10:1))));
  }
</script>
<div class="ramp-field" role="group" aria-label={label}>
  <div class="ramp-bar cp-checker">
    <span class="ramp-paint" style:background={background}></span>
    {#if midpoint && midpointEdit}
      <div class="ramp-midpoint-track" tabindex="0" role="slider" aria-label={`${label} midpoint`} aria-valuemin={1} aria-valuemax={99} aria-valuenow={middle} onkeydown={middleKey}
        use:visualDrag={{api,edit:midpointEdit,get:midpoint,value:(event,bounds)=>Math.max(1,Math.min(99,clamp((event.clientX-bounds.left)/bounds.width*100)))}}>
        <span class="ramp-diamond" style:left={`${middle}%`}></span>
      </div>
    {/if}
    {#each stops as stop,index (stop.id)}
      <button type="button" class="ramp-stop" class:active={stop.id===selected} style:left={`${stop.position}%`} style:background={stop.color}
        role={edit?'slider':undefined} aria-valuemin={edit?0:undefined} aria-valuemax={edit?100:undefined} aria-valuenow={edit?stop.position:undefined} aria-label={`${label} stop ${index+1}`} aria-pressed={edit?undefined:stop.id===selected} title={edit?`${stop.position}%`:`Edit ${stop.color}`} onclick={()=>onSelect?.(stop.id)} onkeydown={(event)=>key(event,stop)} use:dragStop={stop.id}></button>
    {/each}
  </div>
</div>
<style>
  .ramp-field { padding:10px 7px 12px; width:100%; min-width:0; }
  .ramp-bar { position:relative; width:100%; height:var(--ctl-h); border-radius:var(--r-sm); }
  .ramp-paint { position:absolute; inset:0; border-radius:inherit; pointer-events:none; }
  .ramp-stop { position:absolute; bottom:-7px; width:12px; height:14px; transform:translateX(-50%); border:2px solid var(--tx-2); border-radius:3px; cursor:pointer; touch-action:none; }
  .ramp-stop.active { border-color:var(--tx); box-shadow:0 0 0 2px var(--bg-panel); }
  .ramp-stop:focus-visible,.ramp-midpoint-track:focus-visible { outline:2px solid var(--accent); outline-offset:3px; }
  .ramp-midpoint-track { position:absolute; left:0; right:0; top:-10px; height:18px; cursor:ew-resize; touch-action:none; outline:none; }
  .ramp-diamond { position:absolute; top:5px; width:7px; height:7px; transform:translateX(-50%) rotate(45deg); background:var(--tx-2); border:1px solid var(--bg-panel); pointer-events:none; }
</style>
