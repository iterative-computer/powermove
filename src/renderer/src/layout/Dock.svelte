<script lang="ts">
  import type { PMRegistry } from '../legacy/registry';
  import { agentState } from '../panels/agent/agent-state.svelte';
  import GhostCard from '../panels/agent/GhostCard.svelte';
  import { ghostSlotIndex } from '../panels/agent/ui-placement-geometry';
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

  /* A panel the agent is still building takes its place in the dock now, so the
     surrounding panels settle into their final sizes before it arrives. */
  const ghost = $derived(agentState.phase === 'running' ? agentState.uiPlacement : null);
  const ghostSlot = $derived(ghostSlotIndex(ghost, dock.id, specs.map(spec => spec.id)));
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
    {#if ghost && ghostSlot === index}
      <GhostCard placement={ghost} />
    {/if}
    <div
      class="panel-slot"
      data-panel-slot={spec.id}
      style="display:contents"
      use:panelSlot={{ PM, id: spec.id, spec, dock }}
    ></div>
  {/each}
  {#if ghost && ghostSlot === specs.length}
    <GhostCard placement={ghost} />
  {/if}
</div>
