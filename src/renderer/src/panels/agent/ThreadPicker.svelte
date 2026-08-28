<script lang="ts">
  import { agentState } from './agent-state.svelte';
  let { PM }: { PM: Record<string, any> } = $props();
  const blocked = $derived(agentState.threadSwitchBlocked);
  const hint = $derived(blocked ? 'Finish or stop the current run to switch threads' : 'Switch thread');
</script>

{#if agentState.threadId}
  <div class="thread-bar">
    <select aria-label="Switch thread" title={hint} value={agentState.threadId} disabled={blocked}
      onchange={(event) => PM.AgentUI?.switchThread(event.currentTarget.value)}>
      {#each agentState.threads || [] as thread (thread.id)}
        <option value={thread.id}>{thread.title}</option>
      {/each}
    </select>
    <button type="button" aria-label="New thread" title={blocked ? hint : 'New thread'} disabled={blocked}
      onclick={() => PM.AgentUI?.newThread()}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg>
    </button>
  </div>
  {#if agentState.threadSaveError}<p class="thread-warning" role="status">Thread history couldn’t be saved. Keep this window open.</p>{/if}
{/if}

<style>
  .thread-bar { display: flex; align-items: center; gap: 6px; padding: 2px 2px 8px; flex-shrink: 0; }
  select { min-width: 0; flex: 1; height: 28px; padding: 0 7px; border: 1px solid var(--line, #ffffff12); border-radius: 7px; background: transparent; color: inherit; font: inherit; font-size: 11px; text-overflow: ellipsis; cursor: pointer; }
  button { display: grid; place-items: center; width: 28px; height: 28px; flex-shrink: 0; padding: 6px; border: 0; border-radius: 7px; background: transparent; color: inherit; cursor: pointer; }
  button:hover:not(:disabled) { background: var(--hover, #ffffff0a); }
  button:focus-visible, select:focus-visible { outline: 2px solid var(--accent, #ff6b18); outline-offset: 2px; }
  :disabled { opacity: .5; cursor: default; }
  svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; }
  .thread-warning { margin: 0 2px 6px; font-size: 11px; color: var(--accent, #ff6b18); }
</style>
