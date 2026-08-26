<script lang="ts">
  import { agentState } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();

  /* One trigger, two controls (supermove model menu): the model reads as the
     choice, the effort as its dimmer modifier two pixels away. */
  function changeModel(event: Event): void {
    PM.AgentUI?.setModel((event.currentTarget as HTMLSelectElement).value, agentState.reasoningEffort);
  }

  function changeEffort(event: Event): void {
    PM.AgentUI?.setModel(agentState.model, (event.currentTarget as HTMLSelectElement).value);
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') (event.currentTarget as HTMLSelectElement).blur();
  }
</script>

<div class="agent-modelbar" title="Model and reasoning effort">
  <select aria-label="Model" value={agentState.model} onchange={changeModel} onkeydown={keydown}>
    {#each agentState.models as model (model.id)}
      <option value={model.id}>{model.label}</option>
    {/each}
  </select>
  <select class="effort" aria-label="Reasoning effort" value={agentState.reasoningEffort} onchange={changeEffort} onkeydown={keydown}>
    {#each agentState.reasoningEfforts as effort (effort)}
      <option value={effort}>{effort}</option>
    {/each}
  </select>
</div>
