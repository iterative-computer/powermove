<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import Timeline from './Timeline.svelte';
  import WorkingTimer from './WorkingTimer.svelte';
  import Turn from './Turn.svelte';
  import ResultActions from './ResultActions.svelte';

  let { PM }: { PM: Record<string, any> } = $props();

  const suggestions = $derived(agentState.accessMode === 'editor' ? [
    ['Animate a title', 'Animate the selected title with a confident entrance using editable keyframes'],
    ['Shape the scene', 'Refine the composition with editable shapes, thoughtful spacing, and a clear visual hierarchy'],
    ['Arrange my workspace', 'Organize my panels into a focused animation workspace']
  ] : [
    ['Find useful footage', 'Research and download useful licensed footage for this composition, then import the strongest choices'],
    ['Build an integration', 'Build and test the project-local integration needed to complete this project'],
    ['Finish the project', 'Use any useful project tools and web research to finish this composition end to end']
  ]);

  const showTimeline = $derived(agentState.phase === 'running' || agentState.phase === 'preview');

  /* Keep the current request marked across steering snapshots. */
  const answeringIndex = $derived(agentState.phase === 'running'
    ? agentState.conversation.reduce((found, message, index) => message.role === 'user' ? index : found, -1)
    : -1);
</script>

{#if !agentState.conversation.length && !agentState.activity}
  <div class="agent-welcome">
    <div class="agent-welcome-heading">
      <div><b>Make your next move</b><span>{agentState.accessMode === 'editor'
        ? 'Your ideas. Editable motion.'
        : 'Create, research, and build with your project.'}</span></div>
    </div>
    <div class="agent-suggestions">
      {#each suggestions as [label, prompt]}
        <button type="button" onclick={() => PM.AgentUI?.setDraft(prompt, true)}><span>{label}</span><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg></button>
      {/each}
    </div>
  </div>
{/if}
{#each agentState.conversation as message, index (`${message.role}-${index}`)}
  {#if agentState.workingStartedAt !== null && index === agentState.workingConversationIndex}
    <WorkingTimer startedAt={agentState.workingStartedAt} />
  {/if}
  <Turn {PM} {message} messageIndex={index} answering={index === answeringIndex} />
{/each}
{#if agentState.workingStartedAt !== null && agentState.workingConversationIndex === agentState.conversation.length}
  <WorkingTimer startedAt={agentState.workingStartedAt} />
{/if}
{#if showTimeline || agentState.activity}
  <Timeline {PM} />
{/if}
{#if agentState.phase === 'result' && agentState.run?.autonomous && !agentState.panelRun}
  <ResultActions {PM} />
{/if}
