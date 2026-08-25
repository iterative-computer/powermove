<script lang="ts">
  import Icon from '../Icon.svelte';
  import { agentState } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  const value = $derived(`${agentState.model}|${agentState.reasoningEffort}`);

  function change(event: Event): void {
    const [model, effort] = (event.currentTarget as HTMLSelectElement).value.split('|');
    PM.AgentUI?.setModel(model, effort);
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') (event.currentTarget as HTMLSelectElement).blur();
  }
</script>

<label class="agent-model" title="Choose model and reasoning">
  <select aria-label="Model and reasoning effort" {value} onchange={change} onkeydown={keydown}>
    {#each agentState.models as model (model.id)}
      {#each agentState.reasoningEfforts as effort (effort)}
        <option value={`${model.id}|${effort}`}>{model.label} · {effort.replace(/^./, (letter) => letter.toUpperCase())}</option>
      {/each}
    {/each}
  </select>
  <Icon {PM} name="chev" />
</label>
