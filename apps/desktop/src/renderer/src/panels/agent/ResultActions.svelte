<script lang="ts">
  import Icon from '../Icon.svelte';
  import { openImagePreview } from './attachments';
  import { agentState, describePanelAction } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  const run = $derived(agentState.run);
  const panelRun = $derived(agentState.panelRun);
  const reversible = $derived(run?.autonomous ? !!run.changed || !!run.extensionChangeSetId : true);
  const reviewMessage = $derived(run?.review?.message || '');
  const showReviewMessage = $derived(!run?.autonomous || (reviewMessage && ![
    'The editable Powermove result is ready to review.',
    'The agent run completed without changing Powermove source.'
  ].includes(reviewMessage)));

</script>

{#snippet frames()}
  {#if run?.frames?.images?.length}
    <div class="spatial-frame-grid">
      {#each run.frames.images as src, index}
        <figure><button type="button" aria-label={`View rendered composition at ${run.frames.times[index]} seconds`} onclick={() => openImagePreview(PM, `Rendered composition at ${run.frames.times[index]} seconds`, src)}><img {src} alt={`Rendered composition at ${run.frames.times[index]} seconds`} /></button><figcaption>{run.frames.times[index]}s</figcaption></figure>
      {/each}
    </div>
  {/if}
{/snippet}

{#if panelRun}
  <div class="agent-card">
    <div class="agent-card-kicker">Reversible agent change</div>
    <h3>{panelRun.summary}</h3>
    {#each panelRun.actions as action}
      <div class="spatial-preview-control panel-action"><Icon {PM} name="panel" /><span>{describePanelAction(PM, action)}</span></div>
    {/each}
    <p>This is also in the normal Command-Z Undo history.</p>
    <div class="agent-card-actions">
      <button class="agent-btn" type="button" onclick={() => PM.AgentUI?.undoPanelRun()}>Undo change</button>
      <button class="agent-btn pri" type="button" onclick={() => PM.AgentUI?.keepPanelRun()}>Keep change</button>
    </div>
  </div>
{:else if run}
  <div class={run.autonomous ? 'agent-run-details' : 'agent-card'}>
    {#if !run.autonomous}
      <div class="agent-card-kicker">Rendered result</div>
      <h3>Review the actual result</h3>
    {/if}
    {#if showReviewMessage}<p>{reviewMessage || 'The rendered change is ready.'}</p>{/if}
    {#if run.review?.critique}<p>{run.review.critique}</p>{/if}
    {#if reversible && !run.autonomous}<p>Powermove source changes from this run are one Command-Z Undo step.</p>{/if}
    {#if run.reviewError}
      <p class="spatial-review-warning">{run.autonomous ? run.reviewError : `Visual review stopped: ${run.reviewError.slice(0, 130)}. You can still inspect and undo the rendered change.`}</p>
    {/if}
    {#if run.frames?.images?.length}
      {#if run.autonomous}
        <details><summary>View rendered frames</summary>{@render frames()}</details>
      {:else}
        {@render frames()}
      {/if}
    {/if}
    {#if run.autonomous}
      {#if reversible}<button class="agent-run-undo" type="button" title="Restore this run's project and app-extension changes" onclick={() => PM.AgentUI?.undoSceneRun()}>Undo change</button>{/if}
    {:else}
      <div class="agent-card-actions">
        {#if reversible}<button class="agent-btn" type="button" onclick={() => PM.AgentUI?.undoSceneRun()}>Undo change</button>{/if}
        <button class="agent-btn pri" type="button" onclick={() => PM.AgentUI?.keepSceneRun()}>{reversible ? 'Keep change' : 'Done'}</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .agent-run-details { flex: none; min-width: 0; display: flex; flex-direction: column; gap: 8px; color: var(--tx); font-size: var(--fs-md); line-height: 1.6; overflow-wrap: anywhere; }
  .agent-run-details:empty { display: none; }
  .agent-run-details p { margin: 0; white-space: pre-wrap; }
  .agent-run-details details { color: var(--tx-3); font-size: var(--fs-sm); }
  .agent-run-undo { align-self: flex-start; padding: 2px 0; color: var(--tx-3); font-size: var(--fs-sm); }
  .agent-run-undo:hover { color: var(--tx); }
</style>
