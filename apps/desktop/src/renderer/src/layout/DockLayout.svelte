<script lang="ts">
  import { onMount } from 'svelte';
  import type { PMRegistry } from '../legacy/registry';
  import Dock from './Dock.svelte';
  import type { DockSpec, PanelSpec, Workspace } from './model';
  import PanelPool from './PanelPool.svelte';
  import Splitter from './Splitter.svelte';
  import UIPlacementGhost from '../panels/agent/UIPlacementGhost.svelte';

  let { PM }: { PM: PMRegistry } = $props();
  let manifest = $state.raw<Workspace | null>(null);
  let tick = $state(0);


  onMount(() => {
    const body = document.getElementById('body');
    if (!body) return;
    const onWheel = (event: WheelEvent): void => {
      // Only the empty outer gutter belongs to this handler. Panel contents,
      // dividers and overlays retain their own scrolling behavior.
      const gutter = event.target === body || (event.target as Element | null)?.matches?.('.dock-stack, .dock-row');
      if (!gutter || event.ctrlKey || event.metaKey || event.shiftKey || !event.deltaY) return;
      for (const side of ['left', 'right']) {
        const dock = document.getElementById(`dock-${side}`);
        if (!dock || !dock.clientHeight) continue;
        const rect = dock.getBoundingClientRect();
        if (event.clientY < rect.top || event.clientY >= rect.bottom) continue;
        if (side === 'left' ? event.clientX >= rect.left : event.clientX < rect.right) continue;
        if (dock.scrollHeight <= dock.clientHeight) return;
        const unit = event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? dock.clientHeight
          : event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
        event.preventDefault();
        dock.scrollTop += event.deltaY * unit;
        return;
      }
    };
    body.addEventListener('wheel', onWheel, { passive: false });
    return () => body.removeEventListener('wheel', onWheel);
  });

  export function apply(workspace: Workspace): void {
    manifest = workspace;
    tick += 1;
  }

  export function commit(): void {
    if (PM.WS?.current) manifest = PM.WS.current;
    tick += 1;
  }

  const plan = $derived.by(() => {
    void tick;
    if (!manifest) return [] as Array<{ dock: DockSpec; specs: PanelSpec[] }>;
    return PM.Layout.visibleDockPlan(
      manifest,
      (id: string) => PM.Popout?.isOpen?.(id) === true,
      (id: string) => !!PM.PANELS?.[id]
    ) as Array<{ dock: DockSpec; specs: PanelSpec[] }>;
  });
  /* A bottom dock spans every column except the right one: those columns sit
     in a row above it, and the right column keeps the full height. */
  const bottom = $derived(plan.find((entry) => entry.dock.id === 'bottom'));
  const upper = $derived(plan.filter((entry) => entry.dock.id !== 'bottom' && entry.dock.id !== 'right'));
  const trailing = $derived(plan.filter((entry) => entry.dock.id === 'right'));
</script>

{#snippet columns(entries: Array<{ dock: DockSpec; specs: PanelSpec[] }>, leading: DockSpec | undefined)}
  {#each entries as entry, index (entry.dock.id)}
    {#if index > 0 || leading}
      <Splitter
        {PM}
        mode="vertical"
        beforeDock={index > 0 ? entries[index - 1]?.dock : leading}
        afterDock={entry.dock}
        {tick}
        oncommit={commit}
      />
    {/if}
    <Dock {PM} dock={entry.dock} specs={entry.specs} {tick} oncommit={commit} />
  {/each}
{/snippet}

{#if manifest}
  <PanelPool {PM} workspace={manifest} {tick} />
  {#if bottom}
    <div class="dock-stack">
      <div class="dock-row">
        {@render columns(upper, undefined)}
      </div>
      <Splitter {PM} mode="horizontal" afterDock={bottom.dock} {tick} oncommit={commit} />
      <Dock {PM} dock={bottom.dock} specs={bottom.specs} {tick} oncommit={commit} />
    </div>
    {@render columns(trailing, upper.at(-1)?.dock)}
  {:else}
    {@render columns(plan, undefined)}
  {/if}
{/if}

<UIPlacementGhost />
