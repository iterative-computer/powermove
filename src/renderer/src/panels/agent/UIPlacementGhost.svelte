<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import { ghostRect, type GhostRect } from './ui-placement-geometry';

  const placement = $derived(agentState.phase === 'running' ? agentState.uiPlacement : null);
  const kind = $derived(placement?.kind);
  const targetId = $derived(placement?.id);
  const beforeId = $derived(placement?.kind === 'dock' ? placement.beforePanelId : null);
  let rect = $state<GhostRect | null>(null);

  $effect(() => {
    const currentKind = kind;
    const id = targetId;
    const beforePanelId = beforeId;
    rect = null;
    if (!currentKind || !id) return;
    const root = document.getElementById('body');
    if (!root) return;
    let frame = 0;
    const observed = new Set<Element>();
    const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
    const resize = new ResizeObserver(schedule);
    const measure = () => {
      frame = 0;
      const target = document.getElementById(`${currentKind}-${id}`);
      const before = beforePanelId ? document.getElementById(`panel-${beforePanelId}`) : null;
      const elements = new Set<Element>([root, ...root.querySelectorAll('.dock, .panel')]);
      // Changes to a sibling's height can move the target without resizing it.
      for (const element of observed) {
        if (!elements.has(element)) { resize.unobserve(element); observed.delete(element); }
      }
      for (const element of elements) {
        if (!observed.has(element)) { resize.observe(element); observed.add(element); }
      }
      rect = target && root.contains(target)
        ? ghostRect(
          currentKind === 'panel' ? { kind: 'panel', id, label: '' } : { kind: 'dock', id, beforePanelId, label: '' },
          target.getBoundingClientRect(),
          before && target.contains(before) ? before.getBoundingClientRect() : null
        ) : null;
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
  <div
    class="ui-placement-ghost"
    class:compact={rect.height < 90}
    class:new-panel={placement.kind === 'dock'}
    data-ui-placement-ghost={placement.id}
    role="status"
    aria-live="polite"
    aria-atomic="true"
    style:left={`${rect.left}px`}
    style:top={`${rect.top}px`}
    style:width={`${rect.width}px`}
    style:height={`${rect.height}px`}
  >
    <div class="ghost-caption"><i aria-hidden="true"></i><span>{placement.kind === 'dock' ? 'Building' : 'Updating'} {placement.label}…</span></div>
    <div class="ghost-skeleton" aria-hidden="true"><b></b><b></b><b></b></div>
  </div>
{/if}

<style>
  .ui-placement-ghost {
    position: fixed;
    z-index: 35;
    box-sizing: border-box;
    pointer-events: none;
    user-select: none;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--accent, #999) 35%, transparent);
    border-radius: var(--r-md, 8px);
    background: color-mix(in srgb, var(--bg-panel, #242424) 88%, transparent);
    color: var(--tx, #eee);
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 18px;
    padding: 16px;
  }
  .new-panel { border-style: dashed; }
  .ghost-caption { display: flex; align-items: center; justify-content: center; gap: 7px; font: 500 11px var(--f-ui, sans-serif); min-width: 0; }
  .ghost-caption span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ghost-caption i { flex: 0 0 5px; height: 5px; border-radius: 50%; background: currentColor; }
  .ghost-skeleton { width: min(240px, 85%); align-self: center; display: grid; gap: 8px; }
  .ghost-skeleton b { height: 7px; border-radius: 4px; background: color-mix(in srgb, currentColor 12%, transparent); }
  .ghost-skeleton b:nth-child(2) { width: 76%; }
  .ghost-skeleton b:nth-child(3) { width: 55%; }
  .ui-placement-ghost::after {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(110deg, transparent 30%, color-mix(in srgb, var(--tx, #fff) 7%, transparent) 50%, transparent 70%);
    transform: translateX(-100%);
    animation: ghost-sweep 2.4s ease-in-out infinite;
  }
  .compact { padding: 6px; }
  .compact .ghost-skeleton { display: none; }
  @keyframes ghost-sweep { to { transform: translateX(100%); } }
  @media (prefers-reduced-motion: reduce) { .ui-placement-ghost::after { animation: none; display: none; } }
</style>
