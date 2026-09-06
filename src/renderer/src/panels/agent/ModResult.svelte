<script lang="ts">
  import { onMount } from 'svelte';
  import type { AgentModResult } from './mod-result';
  let { PM, result }: { PM: Record<string, any>; result: AgentModResult } = $props();
  let revision = $state(0);
  onMount(() => {
    const subscription = PM.Kernel?.events?.on?.('extensions:changed', () => { revision += 1; });
    return () => subscription?.dispose?.();
  });
  const panels = $derived.by(() => {
    void revision;
    if (result.status !== 'ready') return [];
    return (PM.Kernel?.panels?.entries?.() || []).filter((entry: any) => entry.ownerId === result.id);
  });
  const status = $derived(result.status === 'error' ? 'Needs attention' : result.action === 'removed' ? 'Mod removed' : result.action === 'updated' ? 'Mod updated' : 'Mod added');
</script>

<div class="agent-mod-result" class:needs-attention={result.status === 'error'}>
  <div class="mod-mark" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="m12 3 9 5-9 5-9-5Z M3 12l9 5 9-5 M3 16l9 5 9-5" /></svg></div>
  <div class="mod-copy"><small>{status}</small><b>{result.name}</b></div>
  {#if panels.length === 1}
    <button class="agent-btn" type="button" aria-label={`Open ${result.name}`} onclick={() => PM.LibraryUI?.reveal?.(panels[0].id)}>Open <span aria-hidden="true">↗</span></button>
  {:else if panels.length > 1}
    <div class="mod-panels">{#each panels as panel (panel.id)}<button class="agent-btn" type="button" onclick={() => PM.LibraryUI?.reveal?.(panel.id)}>Open {panel.item.title || panel.id}</button>{/each}</div>
  {/if}
</div>

<style>
  .agent-mod-result{--mod-tint:var(--accent);display:flex;align-items:center;flex-wrap:wrap;gap:10px;padding:11px;border-radius:var(--r-md);background:linear-gradient(to top,color-mix(in srgb,var(--mod-tint) 4%,var(--bg-field)),color-mix(in srgb,var(--mod-tint) 1%,var(--bg-field)) 42%,var(--bg-field) 78%);min-width:0}
  .mod-mark{display:grid;place-items:center;flex:none;width:30px;height:30px;border-radius:var(--r-sm);color:var(--accent-tx);background:var(--accent-dim)}
  svg{width:19px;height:19px;fill:none;stroke:currentColor;stroke-width:1.3;stroke-linejoin:round;stroke-linecap:round}
  .mod-copy{display:flex;flex:1;flex-direction:column;min-width:70px;gap:2px;overflow-wrap:anywhere}
  small{font-size:var(--fs-xs);color:var(--tx-3);line-height:1.35}
  b{font-size:var(--fs-md);font-weight:var(--fw-medium);line-height:1.4;color:var(--tx)}
  button{flex:none;background:var(--ink-1);padding:0 8px}
  button span{margin-left:3px;color:var(--tx-3)}
  .mod-panels{flex-basis:100%;display:flex;flex-wrap:wrap;gap:4px}
  .needs-attention{--mod-tint:var(--warning)}
</style>
