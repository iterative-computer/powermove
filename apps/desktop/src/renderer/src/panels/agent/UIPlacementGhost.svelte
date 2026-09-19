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
    let restoreHighlight: (() => void) | null = null;
    const clear = () => {
      if (card) void unmount(card);
      card = null;
      cardElement = null;
      restoreHighlight?.();
      restoreHighlight = null;
    };
    const sync = () => {
      const next = document.getElementById(`panel-${current.id}`);
      const panel = next && root.contains(next) ? next : null;
      let scoped: HTMLElement | null = null;
      if (panel && current.selector) {
        try {
          // Generated selectors are panel-scoped and must never retarget the
          // ghost's own skeleton when a broad class happens to match it.
          const matches = [...panel.querySelectorAll(current.selector)].filter(
            candidate => candidate instanceof HTMLElement && !candidate.closest('[data-ui-placement-ghost]')
          );
          scoped = matches.length === 1 && matches[0] instanceof HTMLElement ? matches[0] : null;
        } catch { /* Unknown targets stay in the agent panel; never cover the whole panel. */ }
      }
      const resolved = scoped;
      const insertionIsPlaced = Boolean(scoped && current.insert && cardElement?.isConnected
        && cardElement.parentElement === scoped.parentElement
        && (current.insert === 'before'
          ? cardElement.nextElementSibling === scoped
          : cardElement.previousElementSibling === scoped));
      const highlightIsPlaced = Boolean(!current.insert && resolved?.getAttribute('data-agent-editing') === current.label);
      if (resolved === target && (insertionIsPlaced || highlightIsPlaced)) return;
      clear();
      target = resolved;
      // A verified descendant is required. A missing or broad target must not
      // turn unrelated existing controls into a loading placeholder.
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
      const highlighted = target;
      const previous = highlighted.getAttribute('data-agent-editing');
      highlighted.setAttribute('data-agent-editing', current.label);
      restoreHighlight = () => {
        if (highlighted.getAttribute('data-agent-editing') !== current.label) return;
        if (previous === null) highlighted.removeAttribute('data-agent-editing');
        else highlighted.setAttribute('data-agent-editing', previous);
      };
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

<style>
  /* Mark only the actual control or section; retain its content and hit targets. */
  :global([data-agent-editing]) {
    outline: 1px solid color-mix(in srgb, var(--accent, currentColor) 55%, transparent);
    outline-offset: 2px;
  }
</style>
