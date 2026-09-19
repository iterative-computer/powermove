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
      if (event.target !== body || event.ctrlKey || event.metaKey || event.shiftKey || !event.deltaY) return;
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
</script>

{#if manifest}
  <PanelPool {PM} workspace={manifest} {tick} />
  {#each plan as entry, index (entry.dock.id)}
    {#if index > 0}
      <Splitter
        {PM}
        mode="vertical"
        beforeDock={plan[index - 1]?.dock}
        afterDock={entry.dock}
        {tick}
        oncommit={commit}
      />
    {/if}
    <Dock {PM} dock={entry.dock} specs={entry.specs} {tick} oncommit={commit} />
  {/each}
{/if}

<UIPlacementGhost />
