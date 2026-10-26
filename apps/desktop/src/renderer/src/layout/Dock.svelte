<script lang="ts">
  import type { PMRegistry } from '../legacy/registry';
  import { agentState } from '../panels/agent/agent-state.svelte';
  import GhostCard from '../panels/agent/GhostCard.svelte';
  import { ghostSlotIndex } from '../panels/agent/ui-placement-geometry';
  import { defaultDockSize, dockSlots, markStackedPanels, type DockSpec, type PanelSpec } from './model';
  import { syncPanelTabs } from './panel';
  import { panelSlot } from './portal';
  import { roundedScroll } from './rounded-scroll';
  import { routeSideWheel, SIDE_DOCKS } from './side-scroll';
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
    : `0 0 ${dock.size || defaultDockSize(dock.id)}px`);

  /* A panel the agent is still building takes its place in the dock now, so the
     surrounding panels settle into their final sizes before it arrives. */
  const ghost = $derived(agentState.phase === 'running' ? agentState.uiPlacement : null);
  /* Runs of panels that share a group stack as tabs in one slot. */
  const slots = $derived(markStackedPanels(dockSlots(specs), (spec) => !!PM.PANELS[spec.id]?.fitContent));
  const ghostSlot = $derived(ghostSlotIndex(ghost, dock.id, slots.map(slot => slot.active.id)));
  $effect(() => {
    void tick;
    for (const slot of slots) syncPanelTabs(PM, slot.specs, slot.active);
  });

  /* Not passive: when no panel inside can move, the column takes the wheel. */
  function sideWheel(node: HTMLElement, id: string) {
    const onWheel = (event: WheelEvent): void => routeSideWheel(node, event);
    if (SIDE_DOCKS.has(id)) node.addEventListener('wheel', onWheel, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', onWheel) };
  }
</script>

<div class="dock col" id={`dock-${dock.id}`} data-dock={dock.id} style:flex={dockFlex} use:roundedScroll use:sideWheel={dock.id}>
  {#each slots as slot, index (slot.specs[0]!.id)}
    {#if index > 0}
      <Splitter
        {PM}
        mode="horizontal"
        beforeSpec={slots[index - 1]!.active}
        afterSpec={slot.active}
        {tick}
        {oncommit}
      />
    {/if}
    {#if ghost && ghostSlot === index}
      <GhostCard placement={ghost} />
    {/if}
    {#each slot.specs as spec (spec.id)}
      <div
        class="panel-slot"
        data-panel-slot={spec.id}
        data-tab-hidden={slot.specs.length > 1 && spec !== slot.active ? '' : undefined}
        style="display:contents"
        use:panelSlot={{ PM, id: spec.id, spec, dock }}
      ></div>
    {/each}
  {/each}
  {#if ghost && ghostSlot === slots.length}
    <GhostCard placement={ghost} />
  {/if}
</div>
