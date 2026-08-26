<script lang="ts">
  import Icon from '../Icon.svelte';
  import { agentState, describePanelAction } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  const run = $derived(agentState.run);
  const panelRun = $derived(agentState.panelRun);
  const reversible = $derived(run?.autonomous ? !!run.changed : true);

  function artifactSize(bytes: unknown): string {
    const size = Math.max(0, Number(bytes) || 0);
    if (size < 1024 * 1024) return `${Math.max(1, Math.round(size / 1024))} KB`;
    return `${(size / (1024 * 1024)).toFixed(size < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  }

  function artifactIcon(artifact: Record<string, any>): string {
    return artifact.mime?.startsWith('video/') ? 'cam' : artifact.mime?.startsWith('audio/') ? 'clock' : 'frame';
  }
</script>

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
  <div class="agent-card">
    <div class="agent-card-kicker">{run.autonomous ? 'Autonomous result' : 'Rendered result'}</div>
    <h3>{run.autonomous ? (run.summary || 'Agent work is ready') : 'Review the actual result'}</h3>
    <p>{run.review?.message || 'The rendered change is ready.'}</p>
    {#if run.review?.critique}<p>{run.review.critique}</p>{/if}
    {#if reversible}<p>Powermove source changes from this run are one Command-Z Undo step.</p>{/if}
    {#if run.externalActions?.length}
      <div class="agent-external-actions"><b>External activity</b>{#each run.externalActions as action}<span>{action}</span>{/each}</div>
    {/if}
    {#if run.artifacts?.length}
      <div class="agent-artifacts">
        {#each run.artifacts as artifact}
          {@const artifactMeta = [artifact.mime || '', artifactSize(artifact.size)].filter(Boolean).join(' · ')}
          <div class="agent-artifact">
            <Icon {PM} name={artifactIcon(artifact)} />
            <span class="agent-artifact-copy"><b title={artifact.name || artifact.path}>{artifact.name || artifact.path}</b><small title={artifactMeta}>{artifactMeta}</small></span>
            {#if PM.assetKind({ name: artifact.name, type: artifact.mime })}
              <button type="button" disabled={artifact.importing || artifact.imported} aria-label={artifact.imported ? `${artifact.name} is already on the timeline` : `Add ${artifact.name} to timeline`} title={artifact.imported ? 'Already added to the timeline' : 'Add to timeline'} onclick={() => PM.AgentUI?.importArtifact(artifact)}>
                {#if artifact.importing}<i aria-hidden="true"></i>{:else}<Icon {PM} name={artifact.imported ? 'link' : 'plus'} />{/if}
              </button>
            {/if}
            <button type="button" aria-label={`Reveal ${artifact.name || artifact.path} in Finder`} title="Reveal in Finder" onclick={() => PM.AgentUI?.revealArtifact(artifact)}><Icon {PM} name="project" /></button>
          </div>
        {/each}
      </div>
    {/if}
    {#if run.reviewError}<p class="spatial-review-warning">Visual review stopped: {run.reviewError.slice(0, 130)}. You can still inspect and undo the rendered change.</p>{/if}
    {#if run.frames?.images?.length}
      <div class="spatial-frame-grid">
        {#each run.frames.images as src, index}
          <figure><img {src} alt={`Rendered composition at ${run.frames.times[index]} seconds`} /><figcaption>{run.frames.times[index]}s</figcaption></figure>
        {/each}
      </div>
    {/if}
    <div class="agent-card-actions">
      {#if reversible}<button class="agent-btn" type="button" onclick={() => PM.AgentUI?.undoSceneRun()}>Undo change</button>{/if}
      <button class="agent-btn pri" type="button" onclick={() => PM.AgentUI?.keepSceneRun()}>{reversible ? 'Keep change' : 'Done'}</button>
    </div>
  </div>
{/if}
