<script lang="ts">
  import { mount, unmount } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import GhostCard from './GhostCard.svelte';

  const placement = $derived(
    agentState.phase === 'running' && agentState.uiPlacement?.kind === 'panel' ? agentState.uiPlacement : null
  );

  $effect(() => {
    const current = placement;
    const root = document.getElementById('body');
    if (!current || !root) return;
    let target: HTMLElement | null = null;
    let card: ReturnType<typeof mount> | null = null;
    const clear = () => {
      if (card) void unmount(card);
      card = null;
    };
    const sync = () => {
      const next = document.getElementById(`panel-${current.id}`);
      const panel = next && root.contains(next) ? next : null;
      if (panel === target) return;
      clear();
      target = panel;
      // Living inside the panel gives the overlay its exact frame and native
      // clipping through every scrolling ancestor, including rounded docks.
      if (target) card = mount(GhostCard, { target, props: { placement: current, pinned: true } });
    };
    const mutations = new MutationObserver(sync);
    mutations.observe(root, { childList: true, subtree: true });
    sync();
    return () => {
      mutations.disconnect();
      clear();
    };
  });
</script>
