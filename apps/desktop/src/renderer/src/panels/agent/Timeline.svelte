<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import { activityRows, type TraceStep } from './activity-rows';
  import TextRow from './TextRow.svelte';
  import ToolActivity from './ToolActivity.svelte';

  let { PM: _PM }: { PM: Record<string, any> } = $props();

  /* The live activity trail. Rows keep their stream order: prose streams with
     a caret; reasoning and tool calls share flat work groups that disclose
     their rows. Exactly one row carries motion at a time. */

  /* Frozen contract from agent-state; the fallback survives an older snapshot. */
  const trace = $derived((agentState.trace ?? []) as TraceStep[]);
  const rows = $derived(activityRows(trace));
  const running = $derived(agentState.phase === 'running');

  /* Plan steps only mean something in the proposal, where they are the plan.
     During a run the trace replaces them entirely. */
  const planSteps = $derived(agentState.phase === 'preview' ? agentState.steps : []);

  /* First seconds of a run: the trace is empty but the legacy activity string
     is already flowing. One shimmer line with the pixel loader covers the gap. */
  const fallback = $derived(rows.length === 0 ? String(agentState.activity || '') : '');

  /* The newest text row streams while the run is live and nothing has landed
     after it — a tool call or a new thought means that paragraph is finished. */
  const streamingIndex = $derived(running && rows.at(-1)?.kind === 'text' ? rows.length - 1 : -1);
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
    <p class="agent-trace-loading">
      <span class="agent-pixel-loader" aria-hidden="true">{#each Array(9) as _, cell (cell)}<i style="--cell-delay:{((cell % 3) + Math.abs(Math.floor(cell / 3) - 1)) * 90}ms"></i>{/each}</span>
      <span class="agent-trace-thought shimmer-text">{fallback}</span>
    </p>
  </div>
{:else if rows.length}
  <div class="agent-trace is-live">
    {#each rows as row, index (row.renderKey)}
      {#if row.kind === 'text'}
        <TextRow text={row.text} streaming={index === streamingIndex} animated />
      {:else}
        <ToolActivity {row} animated live={running && index === rows.length - 1} />
      {/if}
    {/each}
  </div>
{/if}
