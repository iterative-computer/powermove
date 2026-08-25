<script lang="ts">
  import Icon from '../Icon.svelte';
  import { agentState } from './agent-state.svelte';
  import PlanPreview from './PlanPreview.svelte';
  import ResultActions from './ResultActions.svelte';
  import Turn from './Turn.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let log: HTMLDivElement;

  const suggestions = $derived(agentState.accessMode === 'editor' ? [
    ['Organize for animation', 'Organize my panels into a focused animation workspace'],
    ['Move Timeline right', 'Move the Timeline to the right dock and give it more room'],
    ['Open Inspector + Effects', 'Open the Inspector and Effects panels beside the composition'],
    ['Focus the canvas', 'Focus the composition by hiding panels I do not need right now']
  ] : [
    ['Find useful footage', 'Research and download useful licensed footage for this composition, then import the strongest choices'],
    ['Build an integration', 'Build and test the project-local integration needed to complete this project'],
    ['Use Blender', 'Create the missing 3D asset in Blender, render it, and bring the result into this composition'],
    ['Finish the project', 'Use any useful project tools and web research to finish this composition end to end']
  ]);
  const activityWords = $derived(String(agentState.activity || '').split(/\s+/).filter(Boolean));

  // Deliberate deviation from legacy HEAD: proposals and results are narrowed
  // to their mapped view phase so stale plan/run data cannot render together.
  const showPreview = $derived(agentState.phase === 'preview');
  const showResult = $derived(agentState.phase === 'result');

  $effect(() => {
    agentState.revision;
    if (log) queueMicrotask(() => { log.scrollTop = log.scrollHeight; });
  });
</script>

<div class="spatial-conversation-log" role="log" aria-label="Agent conversation" aria-live="polite" aria-atomic="false" bind:this={log}>
  {#if !agentState.conversation.length && !agentState.activity}
    <div class="agent-welcome">
      <div class="agent-welcome-icon"><Icon {PM} name="sparkle" /></div>
      <b>{agentState.accessMode === 'editor' ? 'Build or rearrange anything' : 'Work across the whole project'}</b>
      <span>{agentState.accessMode === 'editor'
        ? 'Edit the composition, build controls, or tell me exactly how to arrange your panels.'
        : 'Research, create files, run tools, build integrations, and return editable results to Powermove.'}</span>
      <div class="agent-suggestions">
        {#each suggestions as [label, prompt]}
          <button type="button" onclick={() => PM.AgentUI?.setDraft(prompt, true)}>{label}</button>
        {/each}
      </div>
    </div>
  {/if}
  {#each agentState.conversation.slice(-30) as message, index (`${message.role}-${index}`)}
    <Turn {PM} {message} />
  {/each}
  <div class="agent-progress-region">
    {#if agentState.activity}
      <div class:spatial-message={true} class:assistant={true} class:pending={true} class:is-entering={agentState.pendingEntering}>
        <i aria-hidden="true"></i>
        <span class="agent-thinking-copy">
          {#each activityWords as word, index}
            <span class="agent-thinking-word" style={`--word-index:${Math.min(index, 28)}`}>{word}{index < activityWords.length - 1 ? ' ' : ''}</span>
          {/each}
        </span>
      </div>
    {/if}
  </div>
  {#if showPreview}<PlanPreview {PM} />{/if}
  {#if showResult}<ResultActions {PM} />{/if}
</div>

<style>
  .agent-progress-region:empty {
    display: none;
  }

  .agent-thinking-copy {
    display: block;
  }
</style>
