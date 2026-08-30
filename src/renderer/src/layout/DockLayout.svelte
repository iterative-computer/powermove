<script lang="ts">
  import type { PMRegistry } from '../legacy/registry';
  import Dock from './Dock.svelte';
  import type { DockSpec, PanelSpec, Workspace } from './model';
  import PanelPool from './PanelPool.svelte';
  import Splitter from './Splitter.svelte';
  import UIPlacementGhost from '../panels/agent/UIPlacementGhost.svelte';

  let { PM }: { PM: PMRegistry } = $props();
  let manifest = $state.raw<Workspace | null>(null);
  let tick = $state(0);

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
      (id: string) => PM.Popout?.isOpen?.(id) === true
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
