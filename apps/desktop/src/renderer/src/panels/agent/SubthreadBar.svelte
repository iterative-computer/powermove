<script lang="ts">
  import { agentState } from './agent-state.svelte';

  /* Inside a sub-thread: the way back to the thread that started it, and what
     this sub-thread is. A live subagent takes the composer's messages. */
  let { PM }: { PM: Record<string, any> } = $props();
  const threads = $derived(agentState.threads || []);
  const active = $derived(threads.find(thread => thread.id === agentState.threadId));
  const activeParent = $derived(active?.parentId ? threads.find(thread => thread.id === active.parentId) : undefined);
</script>

{#if activeParent}
    <div class="subthread-bar" data-kind={active?.kind}>
      <button class="subthread-back" type="button" onclick={() => PM.AgentUI?.switchThread(activeParent.id)} aria-label={`Back to ${activeParent.title}`}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9.5 4 5.5 8l4 4" /></svg>
        <span>{activeParent.title}</span>
      </button>
      <span class="subthread-note">
        {active?.kind === 'subagent'
          ? (active.busy ? 'A subagent is working here for this thread. Your messages go to it.' : 'A subagent worked here for this thread.')
          : 'The agent started this thread.'}
      </span>
    </div>
{/if}

<style>
  .subthread-bar { display: flex; flex-direction: column; gap: 2px; margin: 0 10px 8px 16px; padding: 6px 8px 7px; border-radius: var(--r-sm); background: var(--ink-1); font-size: var(--fs-xs); color: var(--tx-3); flex-shrink: 0; }
  .subthread-back { display: inline-flex; align-items: center; gap: 2px; align-self: flex-start; max-width: 100%; min-width: 0; margin-left: -3px; padding: 0 2px; border: 0; border-radius: var(--r-xs, 4px); background: transparent; color: var(--tx-2); font: inherit; font-weight: 500; cursor: default; }
  .subthread-back span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .subthread-back:hover { color: var(--tx); }
  .subthread-back svg { width: 12px; height: 12px; }
  .subthread-note { line-height: 1.35; }
  svg { fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
</style>
