<script lang="ts">
  import type { Snippet } from 'svelte';
  import { agentState } from './agent-state.svelte';
  import Timeline from './Timeline.svelte';
  import WorkingTimer from './WorkingTimer.svelte';
  import Turn from './Turn.svelte';
  import ResultActions from './ResultActions.svelte';
  import { groupConversation, workDuration } from './work-groups';

  /* `welcome` replaces the default empty state, for hosts that already know what is being changed. */
  let { PM, welcome }: { PM: Record<string, any>; welcome?: Snippet } = $props();

  const suggestions = $derived(agentState.context === 'app' ? [
    ['Create an extension', 'Help me design and build a new Powermove extension'],
    ['Improve an extension', 'Help me improve one of my existing Powermove extensions'],
    ['Check compatibility', 'Help me check an extension for Store sandbox compatibility']
  ] : agentState.accessMode === 'editor' ? [
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

  /* A finished turn folds its work behind one row; the running turn stays open. */
  const entries = $derived(groupConversation(agentState.conversation, answeringIndex >= 0 ? answeringIndex : null));
  // The panel remounts this per thread, so keys never cross threads.
  let opened = $state<Record<string, boolean>>({});
</script>

{#if !agentState.conversation.length && !agentState.activity && welcome}
  {@render welcome()}
{:else if !agentState.conversation.length && !agentState.activity}
  <div class="agent-welcome">
    <div class="agent-welcome-heading">
      <div><b>Make your next move</b><span>{agentState.context === 'app'
        ? 'Build and improve your extensions.'
        : agentState.accessMode === 'editor'
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
{#snippet turn(message: (typeof agentState.conversation)[number], index: number)}
  {#if agentState.workingStartedAt !== null && index === agentState.workingConversationIndex}
    <WorkingTimer startedAt={agentState.workingStartedAt} />
  {/if}
  <Turn {PM} {message} messageIndex={index} answering={message.role === 'user' && index === answeringIndex} />
{/snippet}
{#each entries as entry (entry.key)}
  {#if entry.kind === 'work'}
    {@const open = Boolean(opened[entry.key])}
    <button type="button" class="agent-work-toggle" aria-expanded={open}
      onclick={() => opened = { ...opened, [entry.key]: !open }}>
      <span>{entry.durationMs === undefined ? 'Work' : `Worked for ${workDuration(entry.durationMs)}`}</span>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg>
    </button>
    {#if open}
      <div class="agent-work-details">
        {#each entry.items as item (item.key)}{@render turn(item.message, item.index)}{/each}
      </div>
    {/if}
  {:else}
    {@render turn(entry.message, entry.index)}
  {/if}
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

<style>
  .agent-work-toggle {
    display: flex; align-items: center; gap: 4px; width: 100%; margin: 4px 0 0; padding: 0 0 10px;
    border: 0; border-bottom: 1px solid var(--line); background: none; text-align: left;
    color: var(--tx-3); font: inherit; font-size: var(--fs-sm); line-height: 1.5; font-variant-numeric: tabular-nums; cursor: pointer;
  }
  .agent-work-toggle:hover { color: var(--tx-2); }
  .agent-work-toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: var(--r-xs); }
  .agent-work-toggle svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; transition: transform var(--dur-1); }
  .agent-work-toggle[aria-expanded="true"] svg { transform: rotate(90deg); }
  .agent-work-details { display: flex; flex-direction: column; gap: 16px; }
</style>
