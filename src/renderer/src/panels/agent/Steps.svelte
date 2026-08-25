<script lang="ts">
  import Icon from '../Icon.svelte';
  import { agentState } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();

  // Deliberate deviation from legacy HEAD: completed steps leave the fixed
  // panel chrome after preview instead of lingering through result review.
  const needed = $derived(agentState.phase === 'running' || agentState.phase === 'preview');
  const complete = $derived(agentState.steps.filter((step) => step.status === 'complete').length);
  const active = $derived(agentState.steps.findIndex((step) => step.status === 'active'));
  const current = $derived(active >= 0 ? active + 1 : Math.min(complete + 1, agentState.steps.length));
  const label = $derived(complete === agentState.steps.length
    ? `Done · ${agentState.steps.length} steps`
    : `Step ${current} / ${agentState.steps.length}`);
</script>

{#if needed && agentState.steps.length}
  <div class="agent-steps">
    <button
      class="agent-steps-toggle"
      type="button"
      aria-expanded={agentState.stepsExpanded}
      onclick={() => PM.AgentUI?.setStepsExpanded(!agentState.stepsExpanded)}
    >
      <i aria-hidden="true"></i>
      <span>{label}</span>
      <Icon {PM} name="chev" />
    </button>
    {#if agentState.stepsExpanded}
      <ol class="agent-todo">
        {#each agentState.steps as step (step.id)}
          <li class={step.status || 'pending'}>
            <i aria-hidden="true"></i>
            <span>{step.title}</span>
          </li>
        {/each}
      </ol>
    {/if}
  </div>
{/if}
