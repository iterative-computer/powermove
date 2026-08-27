<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import { activityRows, type TraceStep } from './activity-rows';
  import { toRichWords } from './rich-words';
  import { stepIn } from './motion';

  let { PM: _PM }: { PM: Record<string, any> } = $props();

  /* The thinking feed: one flat column of quiet rows, no cards, no borders, no
     timestamps. Thinking is muted prose, model text is primary prose, tool work
     collapses into a single wrench row. Exactly one row pulses — the newest
     live one — so the eye always knows where the agent is right now. */

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
          in:stepIn
          class="agent-trace-thought"
          class:is-action={isAction}
          class:is-pulsing={row.pulsing && isAction}
          class:shimmer-text={row.pulsing && !isAction}
        >{#each words as word, wi (wi)}<span class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
      {:else if row.kind === 'text'}
        <p in:stepIn class="agent-trace-text">{#each toRichWords(row.text) as word, wi (wi)}<span class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
      {:else}
        <div
          in:stepIn
          class="agent-trace-tool"
          class:is-error={row.status === 'error'}
          class:is-pulsing={row.pulsing}
          title={row.detail?.length ? row.detail.join('\n') : undefined}
        >
          <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false"><path d="M226.76,69a8,8,0,0,0-12.84-2.88l-40.3,37.19-17.23-3.7-3.7-17.23,37.19-40.3A8,8,0,0,0,187,29.24,72,72,0,0,0,88,96,72.34,72.34,0,0,0,94,124.94L33.79,177.79c-.15.12-.29.26-.43.39a32,32,0,0,0,45.26,45.26c.13-.14.27-.28.39-.43L131.06,162A72,72,0,0,0,232,96,71.56,71.56,0,0,0,226.76,69Zm-71,89a56.14,56.14,0,0,1-21.31-4.18,8,8,0,0,0-9,1.87l-.29.3-57.14,65a16,16,0,0,1-22.62-22.62l65-57.14.3-.29a8,8,0,0,0,1.87-9A56,56,0,0,1,168.72,45.53L138.83,77.9a8,8,0,0,0-1.94,7.1L142.83,113a8,8,0,0,0,6.14,6.14l28,6a8,8,0,0,0,7.1-1.94l32.37-29.89A56.09,56.09,0,0,1,155.76,158Z" /></svg>
          <span>{row.label}</span>
          {#if row.status === 'error'}<em>Failed</em>{/if}
        </div>
      {/if}
    {/each}
  </div>
{/if}
