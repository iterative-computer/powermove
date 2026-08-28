<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import Timeline from './Timeline.svelte';
  import Turn from './Turn.svelte';
  import ResultActions from './ResultActions.svelte';

  let { PM }: { PM: Record<string, any> } = $props();

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

  const showTimeline = $derived(agentState.phase === 'running' || agentState.phase === 'preview');
</script>

{#if !agentState.conversation.length && !agentState.activity}
  <div class="agent-welcome">
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
{#if showTimeline || agentState.activity}
  <Timeline {PM} />
{/if}
{#if agentState.phase === 'result' && agentState.run?.autonomous && !agentState.panelRun}
  <ResultActions {PM} />
{/if}
