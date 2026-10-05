<script lang="ts">
  import Icon from '../Icon.svelte';
  import Markdown from './Markdown.svelte';
  import { isPreviewableImageType, openImagePreview } from './attachments';
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
  // Files the agent wrote stay on disk in its workspace; this is the only way to reach them.
  const files = $derived(run?.artifacts ?? []);

  function fileSize(bytes: unknown): string {
    const size = Math.max(0, Number(bytes) || 0);
    if (size < 1024 * 1024) return `${Math.max(1, Math.round(size / 1024))} KB`;
    return `${(size / (1024 * 1024)).toFixed(size < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  }

  async function openFile(artifact: Record<string, any>): Promise<void> {
    if (!isPreviewableImageType(artifact.mime)) {
      PM.AgentUI?.revealArtifact(artifact);
      return;
    }
    try {
      const file = await PM.AgentArtifacts.load(artifact);
      const src = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(new Error(`Could not preview ${artifact.name}.`));
        reader.readAsDataURL(file);
      });
      openImagePreview(PM, artifact.name || artifact.path, src);
    } catch (error) {
      PM.toast(error instanceof Error ? error.message : `Could not preview ${artifact.name}.`, 6000);
    }
  }

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
    <div class="result-summary"><Markdown text={panelRun.summary} /></div>
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
    {#if showReviewMessage}
      <div class="agent-review-markdown"><Markdown text={reviewMessage || 'The rendered change is ready.'} /></div>
    {/if}
    {#if run.review?.critique}<Markdown text={run.review.critique} />{/if}
    {#if reversible && !run.autonomous}<p>Powermove source changes from this run are one Command-Z Undo step.</p>{/if}
    {#if run.reviewError}
      <div class="spatial-review-warning"><Markdown text={run.autonomous ? run.reviewError : `Visual review stopped: ${run.reviewError.slice(0, 130)}. You can still inspect and undo the rendered change.`} /></div>
    {/if}
    {#if run.frames?.images?.length}
      {#if run.autonomous}
        <details><summary>View rendered frames</summary>{@render frames()}</details>
      {:else}
        {@render frames()}
      {/if}
    {/if}
    {#if files.length}
      <details class="agent-run-files">
        <summary>{files.length === 1 ? '1 file' : `${files.length} files`}</summary>
        <ul>
          {#each files as artifact (artifact.path)}
            <li>
              <button class="agent-run-file-name" type="button" title={isPreviewableImageType(artifact.mime) ? `View ${artifact.name}` : 'Reveal in Finder'} onclick={() => void openFile(artifact)}>{artifact.name}</button>
              <small>{fileSize(artifact.size)}</small>
              {#if PM.proj && PM.assetKind({ name: artifact.name, type: artifact.mime })}
                <button type="button" disabled={artifact.importing || artifact.imported} aria-label={artifact.imported ? `${artifact.name} is already on the timeline` : `Add ${artifact.name} to timeline`} title={artifact.imported ? 'Already added to the timeline' : 'Add to timeline'} onclick={() => PM.AgentUI?.importArtifact(artifact)}><Icon {PM} name={artifact.imported ? 'link' : 'plus'} /></button>
              {/if}
              <button type="button" aria-label={`Reveal ${artifact.name} in Finder`} title="Reveal in Finder" onclick={() => PM.AgentUI?.revealArtifact(artifact)}><Icon {PM} name="project" /></button>
            </li>
          {/each}
        </ul>
      </details>
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
  .result-summary { font-weight: var(--fw-semibold); }
  .agent-run-details { flex: none; min-width: 0; display: flex; flex-direction: column; gap: 8px; color: var(--tx); font-size: var(--fs-md); line-height: 1.6; overflow-wrap: anywhere; }
  .agent-run-details:empty { display: none; }
  .agent-run-details p { margin: 0; white-space: pre-wrap; }
  .agent-review-markdown { display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .agent-run-details details { color: var(--tx-3); font-size: var(--fs-sm); }
  .agent-run-files { color: var(--tx-3); font-size: var(--fs-sm); }
  .agent-run-files ul { display: flex; flex-direction: column; gap: 2px; margin: 6px 0 0; padding: 0; list-style: none; }
  .agent-run-files li { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .agent-run-file-name { flex: 1; min-width: 0; overflow: hidden; color: var(--tx); text-align: left; text-overflow: ellipsis; white-space: nowrap; }
  .agent-run-files small { flex: none; font-variant-numeric: tabular-nums; }
  .agent-run-files li > button:not(.agent-run-file-name) { flex: none; display: grid; place-items: center; width: 20px; height: 20px; color: var(--tx-3); }
  .agent-run-files li > button:hover:not(:disabled) { color: var(--tx); }
  .agent-run-files :global(.pm-icon) { width: 13px; height: 13px; }
  .agent-run-undo { align-self: flex-start; padding: 2px 0; color: var(--tx-3); font-size: var(--fs-sm); }
  .agent-run-undo:hover { color: var(--tx); }
</style>
