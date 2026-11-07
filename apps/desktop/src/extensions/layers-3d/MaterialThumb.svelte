<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { PowermoveAPI } from 'powermove';
  import { untrack } from 'svelte';
  import { materialThumbnails } from './material-thumbnails';
  import type { MaterialMapValue } from './materials';

  /* A source's picture in a fixed square: the placeholder (its kind's icon)
     until the engine's preview arrives, then the preview, cropped to fill
     and rounded like a Finder thumbnail. The box never changes size, so
     nothing moves when the picture lands. `refresh` re-asks after edits. */
  let { api, layerId, map, refresh = 0, size = 16, placeholder }: {
    api: PowermoveAPI;
    layerId: string;
    map: MaterialMapValue | null | undefined;
    refresh?: unknown;
    size?: number;
    placeholder: Snippet;
  } = $props();

  const thumbnails = untrack(() => materialThumbnails(api));
  let arrived = $state(0);
  $effect(() => thumbnails.subscribe(() => { arrived++; }));
  const url = $derived.by(() => { void arrived; void refresh; return thumbnails.peek(layerId, map); });
  let loaded = $state<string | null>(null);
</script>

<!-- Once a picture has shown, a refresh swaps it in place rather than
     flashing the placeholder. -->
<span class="mat-thumb" class:ready={!!url && !!loaded} style:--thumb={`${size}px`} data-material-thumb={url && loaded ? 'ready' : 'placeholder'} aria-hidden="true">
  <span class="mat-thumb-placeholder">{@render placeholder()}</span>
  {#if url}<img src={url} alt="" draggable="false" onload={() => { loaded = url; }} />{/if}
</span>

<style>
  .mat-thumb { position: relative; display: grid; place-items: center; flex: none; width: var(--thumb); height: var(--thumb); }
  .mat-thumb-placeholder { display: grid; place-items: center; transition: opacity var(--dur-1, .12s); }
  .mat-thumb img {
    position: absolute; inset: 0; width: 100%; height: 100%;
    object-fit: cover; border-radius: calc(var(--thumb) * .22);
    opacity: 0; transition: opacity var(--dur-2, .18s);
    /* A hairline keeps light pictures from bleeding into the field. */
    box-shadow: 0 0 0 .5px rgb(var(--ink-rgb) / .28);
    background: rgb(var(--ink-rgb) / .08);
  }
  .mat-thumb.ready img { opacity: 1; }
  .mat-thumb.ready .mat-thumb-placeholder { opacity: 0; }
</style>
