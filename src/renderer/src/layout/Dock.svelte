<script lang="ts">
  import type { PMRegistry } from '../legacy/registry';
  import type { DockSpec, PanelSpec } from './model';
  import { panelSlot } from './portal';
  import Splitter from './Splitter.svelte';

  let {
    PM,
    dock,
    specs,
    tick,
    oncommit
  }: {
    PM: PMRegistry;
    dock: DockSpec;
    specs: PanelSpec[];
    tick: number;
    oncommit: () => void;
  } = $props();

  const dockFlex = $derived(dock.id === 'center' || dock.flex
    ? '1 1 auto'
    : `0 0 ${dock.size || (dock.id === 'right' ? 300 : 250)}px`);
</script>

<div class="dock col" id={`dock-${dock.id}`} data-dock={dock.id} style:flex={dockFlex}>
  {#each specs as spec, index (spec.id)}
    {#if index > 0}
      <Splitter
        {PM}
        mode="horizontal"
        beforeSpec={specs[index - 1]}
        afterSpec={spec}
        {tick}
        {oncommit}
      />
    {/if}
    <div
      class="panel-slot"
      data-panel-slot={spec.id}
      style="display:contents"
      use:panelSlot={{ PM, id: spec.id, spec, dock }}
    ></div>
  {/each}
</div>
