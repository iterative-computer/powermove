<script lang="ts">
  import { agentState } from './agent-state.svelte';

  let { PM: _PM }: { PM: Record<string, any> } = $props();

  /* Supermove step timeline: a flat column of quiet 13px rows. Completed steps
     rest muted, the active one carries the motion, errors turn red. The live
     activity line renders word-by-word below the steps. */
  const activityWords = $derived(String(agentState.activity || '').split(/\s+/).filter(Boolean));
  const steps = $derived(agentState.phase === 'running' || agentState.phase === 'preview' ? agentState.steps : []);
</script>

<div class="agent-timeline">
  {#each steps as step (step.id)}
    <div class="agent-step {step.status || 'pending'}">
      <i aria-hidden="true"></i>
      <span>{step.title}</span>
      {#if step.status === 'error'}<em>Failed</em>{/if}
    </div>
  {/each}
  {#if agentState.activity}
    <div class="agent-activity" class:is-entering={agentState.pendingEntering}>
      <span class="agent-thinking-copy">
        {#each activityWords as word, index}
          <span class="agent-thinking-word" style={`--word-index:${Math.min(index, 28)}`}>{word}{index < activityWords.length - 1 ? ' ' : ''}</span>
        {/each}
      </span>
    </div>
  {/if}
</div>
