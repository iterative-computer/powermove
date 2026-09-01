<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import GhostCard from './GhostCard.svelte';
  import { ghostRect, type GhostRect } from './ui-placement-geometry';

  /* New panels reserve a real slot in their dock (see layout/Dock.svelte). Only
     a panel that already exists is covered by a pinned ghost. */
  const placement = $derived(
    agentState.phase === 'running' && agentState.uiPlacement?.kind === 'panel' ? agentState.uiPlacement : null
  );
  const targetId = $derived(placement?.id);
  let rect = $state<GhostRect | null>(null);

  $effect(() => {
    const id = targetId;
    rect = null;
    if (!id) return;
    const root = document.getElementById('body');
    if (!root) return;
    let frame = 0;
    const observed = new Set<Element>();
    const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
    const resize = new ResizeObserver(schedule);
    const measure = () => {
      frame = 0;
      const target = document.getElementById(`panel-${id}`);
      const elements = new Set<Element>([root, ...root.querySelectorAll('.dock, .panel')]);
      // Changes to a sibling's height can move the target without resizing it.
      for (const element of observed) {
        if (!elements.has(element)) { resize.unobserve(element); observed.delete(element); }
      }
      for (const element of elements) {
        if (!observed.has(element)) { resize.observe(element); observed.add(element); }
      }
      rect = target && root.contains(target) ? ghostRect(target.getBoundingClientRect()) : null;
    };
    // Follow reparenting/hot replacement without polling or touching the layout.
    const mutations = new MutationObserver(records => {
      if (records.some(record => !(record.target instanceof Element) || !record.target.closest('[data-ui-placement-ghost]'))) schedule();
    });
    mutations.observe(root, { childList: true, subtree: true });
    window.addEventListener('resize', schedule);
    window.addEventListener('scroll', schedule, true);
    measure();
    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      mutations.disconnect();
      window.removeEventListener('resize', schedule);
      window.removeEventListener('scroll', schedule, true);
    };
  });
</script>

{#if placement && rect}
  <GhostCard {placement} {rect} />
{/if}
