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
    let cardElement: HTMLElement | null = null;
    let restorePosition: (() => void) | null = null;
    const clear = () => {
      if (card) void unmount(card);
      card = null;
      cardElement = null;
      restorePosition?.();
      restorePosition = null;
    };
    const sync = () => {
      const next = document.getElementById(`panel-${current.id}`);
      const panel = next && root.contains(next) ? next : null;
      let scoped: HTMLElement | null = null;
      if (panel && current.selector) {
        try {
          // Generated selectors are panel-scoped and must never retarget the
          // ghost's own skeleton when a broad class happens to match it.
          const match = [...panel.querySelectorAll(current.selector)].find(
            candidate => candidate instanceof HTMLElement && !candidate.closest('[data-ui-placement-ghost]')
          );
          scoped = match instanceof HTMLElement ? match : null;
        } catch { /* A stale or malformed generated selector falls back to the panel. */ }
      }
      const resolved = scoped ?? panel;
      const insertionIsPlaced = Boolean(scoped && current.insert && cardElement?.isConnected
        && cardElement.parentElement === scoped.parentElement
        && (current.insert === 'before'
          ? cardElement.nextElementSibling === scoped
          : cardElement.previousElementSibling === scoped));
      const overlayIsPlaced = Boolean(!(scoped && current.insert) && cardElement?.isConnected && cardElement.parentElement === resolved);
      if (resolved === target && (insertionIsPlaced || overlayIsPlaced)) return;
      clear();
      target = resolved;
      // Living inside the panel gives the overlay its exact frame and native
      // clipping through every scrolling ancestor, including rounded docks.
      if (!target) return;
      if (scoped && current.insert) {
        const parent = scoped.parentElement;
        if (!parent) return;
        const anchor = current.insert === 'before' ? scoped : scoped.nextSibling;
        card = mount(GhostCard, { target: parent, anchor: anchor ?? undefined, props: { placement: current, section: true } });
        cardElement = [...parent.children].find(child =>
          child instanceof HTMLElement && child.dataset.uiPlacementGhost === current.id
          && !child.closest('[data-ui-placement-ghost] [data-ui-placement-ghost]')
        ) as HTMLElement | null;
        return;
      }
      if (getComputedStyle(target).position === 'static') {
        const value = target.style.getPropertyValue('position');
        const priority = target.style.getPropertyPriority('position');
        target.style.setProperty('position', 'relative');
        const positionedTarget = target;
        restorePosition = () => {
          // Preserve a live UI change that deliberately chose another position.
          if (positionedTarget.style.getPropertyValue('position') !== 'relative') return;
          if (value) positionedTarget.style.setProperty('position', value, priority);
          else positionedTarget.style.removeProperty('position');
        };
      }
      card = mount(GhostCard, { target, props: { placement: current, pinned: true, section: Boolean(scoped) } });
      cardElement = [...target.children].find(child =>
        child instanceof HTMLElement && child.dataset.uiPlacementGhost === current.id
      ) as HTMLElement | null;
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
