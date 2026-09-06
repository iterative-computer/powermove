<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import { activityRows, type TraceStep } from './activity-rows';
  import { toRichWords } from './rich-words';
  import { stepIn } from './motion';
  import { revealText } from './text-reveal';
  import ToolActivity from './ToolActivity.svelte';

  let { PM: _PM }: { PM: Record<string, any> } = $props();

  /* Public commentary, reply text, and expandable tool activity retain their
     stream order. Only the newest active tool group carries the sweep. */

  /* Frozen contract from agent-state; the fallback survives an older snapshot. */
  const trace = $derived((agentState.trace ?? []) as TraceStep[]);
  const rows = $derived(activityRows(trace));

  /* Plan steps only mean something in the proposal, where they are the plan.
     During a run the trace replaces them entirely. */
  const planSteps = $derived(agentState.phase === 'preview' ? agentState.steps : []);

  /* First seconds of a run: the trace is empty but the legacy activity string
     is already flowing. One shimmer line covers the gap. */
  const fallback = $derived(rows.length === 0 ? String(agentState.activity || '') : '');
</script>

{#if planSteps.length}
  <div class="agent-timeline">
    {#each planSteps as step (step.id)}
      <div class="agent-step {step.status || 'pending'}">
        <i aria-hidden="true"></i>
        <span>{step.title}</span>
        {#if step.status === 'error'}<em>Failed</em>{/if}
      </div>
    {/each}
  </div>
{/if}

{#if fallback}
  <div class="agent-trace">
    <p class="agent-trace-thought shimmer-text">{fallback}</p>
  </div>
{:else if rows.length}
  <div class="agent-trace">
    {#each rows as row (row.renderKey)}
      {#if row.kind === 'thought'}
        {@const words = toRichWords(row.label)}
        {@const isAction = words.some((word) => word.b)}
        <p
          class="agent-trace-thought"
          class:is-action={isAction}
          class:is-pulsing={row.pulsing && isAction}
          class:shimmer-text={row.pulsing && !isAction}
        >{#each words as word, wi (wi)}<span use:revealText={row.live && Boolean(word.w.trim())} class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
      {:else if row.kind === 'text'}
        <p class="agent-trace-text">{#each toRichWords(row.text) as word, wi (wi)}<span use:revealText={agentState.phase === 'running' && Boolean(word.w.trim())} class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
      {:else}
        <div in:stepIn><ToolActivity {row} animated /></div>
      {/if}
    {/each}
  </div>
{/if}
