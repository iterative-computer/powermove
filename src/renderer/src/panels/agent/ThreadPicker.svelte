<script lang="ts">
  import { agentState } from './agent-state.svelte';
  let { PM }: { PM: Record<string, any> } = $props();
  const blocked = $derived(agentState.threadSwitchBlocked);
  const hint = $derived(blocked ? 'Finish or stop the current run to switch threads' : 'Switch thread');
</script>

{#if agentState.threadId}
  <div class="thread-bar" role="group" aria-label="Agent threads">
    <span class="thread-select-wrap">
      <select class="thread-select" aria-label="Switch thread" title={hint} value={agentState.threadId} disabled={blocked}
        onchange={(event) => PM.AgentUI?.switchThread(event.currentTarget.value)}>
        {#each agentState.threads || [] as thread (thread.id)}
          <option value={thread.id}>{thread.title}</option>
        {/each}
      </select>
      <svg class="thread-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m4.5 6.25 3.5 3.5 3.5-3.5" /></svg>
    </span>
    <button class="thread-new" type="button" aria-label="New thread" title={blocked ? hint : 'New thread'} disabled={blocked}
      onclick={() => PM.AgentUI?.newThread()}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg>
    </button>
  </div>
  {#if agentState.threadSaveError}<p class="thread-warning" role="status">Thread history couldn’t be saved. Keep this window open.</p>{/if}
{/if}

<style>
  .thread-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    margin: 0 10px 8px 16px;
    flex-shrink: 0;
  }
  .thread-select-wrap { position: relative; min-width: 0; flex: 1; height: 28px; }
  .thread-select { appearance: none; width: 100%; min-width: 0; height: 28px; padding: 0 20px 0 2px; border: 0; border-radius: 0; background: transparent; color: var(--tx-3, #9a9a9a); font: inherit; font-size: 11px; text-overflow: ellipsis; cursor: pointer; }
  .thread-chevron { position: absolute; right: 1px; top: 7px; width: 14px; height: 14px; pointer-events: none; color: var(--tx-3, #9a9a9a); }
  .thread-new { display: grid; place-items: center; width: 28px; height: 28px; flex-shrink: 0; padding: 6px; border: 0; border-radius: 50%; background: transparent; color: var(--tx-3, #9a9a9a); cursor: pointer; }
  .thread-select:hover:not(:disabled), .thread-new:hover:not(:disabled) { color: var(--tx-2, #c0c0c0); }
  .thread-new:hover:not(:disabled) { background: var(--hover, #ffffff0a); }
  .thread-new:focus-visible, .thread-select:focus-visible { outline: 2px solid var(--accent, #ff6b18); outline-offset: 2px; }
  :disabled { opacity: .5; cursor: default; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .thread-warning { margin: 0 10px 6px 18px; font-size: 11px; color: var(--accent, #ff6b18); }
</style>
