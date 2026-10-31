<script lang="ts">
  /* The 3D viewer's toolbar, left to right:
     - what you look through: the shot's camera, or a free view that never changes the shot;
     - one row of tools, each saying what a drag does: move, rotate or scale the selection;
       orbit, pan or zoom the view; or point light at a model;
     - for move, rotate and scale: which way the handles face (the light tool's options are in its panel);
     - frame the selection;
     - how the viewer draws 3D: fast (real time) or quality (Studio's path tracer). */
  import {onDestroy,untrack} from 'svelte';
  import type {PowermoveAPI} from 'powermove';
  let {api}:{api:PowermoveAPI}=$props();
  const scene=untrack(()=>api.scene3d);
  type Tool='translate'|'rotate'|'scale'|'orbit'|'pan'|'dolly'|'light';

  let mode=$state(untrack(()=>scene.getMode())),space=$state(untrack(()=>scene.getSpace()));
  let navigation=$state(untrack(()=>scene.getNavigationMode()));
  let view=$state(untrack(()=>scene.getView()));
  let light=$state(untrack(()=>scene.lights.getTool()));
  let preview=$state(untrack(()=>scene.studio.getPreview()));
  const subscriptions=untrack(()=>[
    scene.onGizmoChange(state=>{mode=state.mode;space=state.space;}),
    scene.onNavigationChange(next=>navigation=next),
    scene.onViewChange(next=>view=next),
    scene.lights.onToolChange(next=>light=next),
    scene.studio.onPreviewChange(next=>preview=next),
  ]);
  onDestroy(()=>{for(const s of subscriptions)s.dispose();});

  const icon=(name:string)=>api.ui.icon(name);
  const tools:({id:Tool;icon:string;label:string;tip:string}|null)[]=[
    {id:'translate',icon:'move',label:'Move',tip:'Move — drag the selected object, or one of its arrows to slide along that axis'},
    {id:'rotate',icon:'rotate',label:'Rotate',tip:'Rotate — turn the selected object with its rings'},
    {id:'scale',icon:'aspectRatio',label:'Scale',tip:'Scale — resize the selected object'},
    null,
    {id:'orbit',icon:'orbit',label:'Orbit',tip:'Orbit — drag to circle the view around the scene (anytime: middle-drag or Alt-drag)'},
    {id:'pan',icon:'hand',label:'Pan',tip:'Pan — drag to slide the view (anytime: Shift + Alt-drag)'},
    {id:'dolly',icon:'zoom',label:'Zoom',tip:'Zoom — drag up and down to move the view closer or farther (anytime: scroll)'},
    null,
    {id:'light',icon:'lightbulb',label:'Light',tip:'Light — click a model to put light there. Aims the selected light, or adds one'},
  ];
  const active=$derived<Tool>(light.on?'light':navigation!=='select'?navigation:mode);
  function pick(tool:Tool){
    if(tool==='light'){scene.lights.setTool({on:!light.on});return;}
    scene.lights.setTool({on:false});
    if(tool==='orbit'||tool==='pan'||tool==='dolly'){scene.setNavigationMode(tool);return;}
    scene.setNavigationMode('select');scene.setMode(tool);
  }
  const transforming=$derived(active==='translate'||active==='rotate'||active==='scale');

</script>

<div class="bar" role="toolbar" aria-label="3D view">
  <div class="group" role="radiogroup" aria-label="Look through">
    <button class:on={view==='camera'} role="radio" aria-checked={view==='camera'} aria-label="Camera" title="Camera — see exactly what the shot renders" onclick={()=>scene.setView('camera')}>
      <span class="ic">{@html icon('cam')}</span><span class="lbl">Camera</span>
    </button>
    <button class:on={view==='editor'} role="radio" aria-checked={view==='editor'} aria-label="Free view" title="Free view — look around the scene without changing the shot" onclick={()=>scene.setView('editor')}>
      <span class="ic">{@html icon('cube')}</span><span class="lbl">Free view</span>
    </button>
  </div>

  <div class="group" role="radiogroup" aria-label="Tool">
    {#each tools as tool, i (tool?.id ?? `sep${i}`)}
      {#if tool}
        <button class="icon-only" class:on={active===tool.id} role="radio" aria-checked={active===tool.id} aria-label={tool.label} title={tool.tip} onclick={()=>pick(tool.id)}>
          <span class="ic">{@html icon(tool.icon)}</span>
        </button>
      {:else}
        <span class="sep" aria-hidden="true"></span>
      {/if}
    {/each}
  </div>

  {#if transforming}
    <button class="chip" aria-label={space==='world'?'Scene axes':'Object axes'} title={space==='world'?'Handles line up with the scene (up is up). Click to line them up with the object instead':'Handles line up with the object, however it is turned. Click to line them up with the scene instead'}
      onclick={()=>scene.setSpace(space==='world'?'local':'world')}>
      <span class="ic">{@html icon(space==='world'?'globe':'cube')}</span><span class="lbl">{space==='world'?'Scene axes':'Object axes'}</span>
    </button>
  {/if}

  <button class="plain icon-only" aria-label="Frame" title="Frame — fit the selection in view, or everything when nothing is selected (F, Home)" onclick={()=>scene.frame(true)}>
    <span class="ic">{@html icon('frame')}</span>
  </button>

  <div class="group" role="radiogroup" aria-label="Preview">
    <button class:on={preview==='fast'} role="radio" aria-checked={preview==='fast'} aria-label="Fast" title="Fast — real-time preview that keeps up while you work. Exports always render at full quality" onclick={()=>scene.studio.setPreview('fast')}>
      <span class="ic">{@html icon('lightning')}</span><span class="lbl">Fast</span>
    </button>
    <button class:on={preview==='quality'} role="radio" aria-checked={preview==='quality'} aria-label="Quality" title="Quality — path-traced, as the export will look: true reflections, soft shadows and glass. Sharpens when you stop" onclick={()=>scene.studio.setPreview('quality')}>
      <span class="ic">{@html icon('sparkle')}</span><span class="lbl">Quality</span>
    </button>
  </div>
</div>

<style>
  .bar { display: flex; align-items: center; gap: 6px; }
  .group { display: inline-flex; align-items: center; gap: 1px; padding: 2px; border-radius: var(--r-md); background: var(--ink-2); }
  button { display: inline-flex; align-items: center; gap: 5px; height: calc(var(--ctl-h) - 4px); padding: 0 9px; border: 0; border-radius: calc(var(--r-md) - 2px); background: transparent; color: var(--tx-2); font-size: var(--fs-sm); font-weight: var(--fw-medium); white-space: nowrap; cursor: default; transition: color var(--dur-1) var(--ease), background var(--dur-1) var(--ease); }
  button:hover { color: var(--tx); }
  button.on { background: var(--ctl-raised); box-shadow: var(--ctl-edge); color: var(--tx); }
  button:focus-visible { outline: none; box-shadow: var(--focus-ring); }
  button.icon-only { width: calc(var(--ctl-h) - 4px); padding: 0; justify-content: center; }
  button.plain, button.chip { height: var(--ctl-h); border-radius: var(--r-md); background: var(--ink-2); }
  button.plain.icon-only { width: var(--ctl-h); }
  .ic { display: inline-flex; width: 14px; height: 14px; }
  .ic :global(svg) { width: 14px; height: 14px; }
  /* A narrow viewer keeps the icons (each has its tooltip and name). */
  @container viewer-foot (max-width: 860px) { .lbl { display: none; } button:not(.icon-only) { padding: 0 7px; } }
  .sep { width: 1px; height: 14px; margin: 0 3px; background: var(--ink-3); }
</style>
