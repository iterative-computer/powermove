<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import type { PowermoveAPI } from 'powermove';
  import { icon } from './icons';
  import { TOOLS } from './menus';

  /* Blender's toolbar (T): selection tools, the 3D cursor, and the transform tools that show a gizmo. */
  let { api }: { api: PowermoveAPI } = $props();
  const viewport = untrack(() => api.scene3d.viewport);
  let vp = $state(untrack(() => viewport.state()));
  const subscription = untrack(() => viewport.onChange((next) => { vp = next; }));
  onDestroy(() => subscription.dispose());
</script>

{#if vp.hasScene && vp.toolbar}
  <div class="vp-toolbar" role="radiogroup" aria-label="3D tools" data-viewport-ui>
    {#each TOOLS as tool, index (tool.id)}
      {#if index === 2 || index === 3}<span class="gap" aria-hidden="true"></span>{/if}
      <button type="button" role="radio" aria-checked={vp.tool === tool.id} aria-label={tool.label}
        title={`${tool.label}${tool.key ? ` (${tool.key})` : ''}`} onclick={() => viewport.run('tool', tool.id)}>{@html icon(tool.icon, 18)}</button>
    {/each}
  </div>
{/if}

<style>
  .vp-toolbar{position:absolute;left:8px;top:8px;z-index:6;display:flex;flex-direction:column;gap:2px;padding:3px;border-radius:8px;background:rgba(30,30,30,.82);box-shadow:0 1px 4px rgba(0,0,0,.25)}
  button{all:unset;box-sizing:border-box;width:30px;height:30px;display:grid;place-items:center;border-radius:6px;color:rgba(255,255,255,.82);cursor:pointer}
  button:hover{background:rgba(255,255,255,.1);color:#fff}
  button[aria-checked="true"]{background:#4772b3;color:#fff}
  button:focus-visible{outline:2px solid var(--accent,#4772b3);outline-offset:1px}
  .gap{height:5px}
</style>
