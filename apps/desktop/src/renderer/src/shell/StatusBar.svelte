<script lang="ts">
  import { kernelSignals } from '../kernel/signals.svelte';
  import { frameBus } from '../runtime/frame-bus';
  import { doc } from '../state/document.svelte';
  import { perf, transport } from '../state/transport.svelte';
  import { performanceMonitor } from '../runtime/performance-monitor';

  let { PM }: { PM: Record<string, any> } = $props();
  let refreshToken = $state(0);
  const performanceIssues = $derived.by(() => { refreshToken; return performanceMonitor.issues(); });
  function showPerformanceIssues() {
    const body = document.createElement('div');
    for (const issue of performanceIssues) {
      const row = document.createElement('div'); row.className = 'settings-row';
      const copy = document.createElement('div'); copy.className = 'settings-copy';
      const title = document.createElement('b'); title.textContent = issue.name;
      const detail = document.createElement('span');
      detail.textContent = `${issue.ms.toFixed(1)} ms ${issue.kind === 'extension' ? 'on the editor thread' : issue.id.startsWith('compile:') ? 'compiling' : 'on the GPU'}. A 60 fps frame has 16.7 ms available.`;
      copy.append(title, detail); row.append(copy);
      const action = document.createElement('button'); action.className = 'btn';
      if (issue.layerId) {
        action.textContent = 'Select layer';
        action.onclick = () => { PM.selectLayers?.([issue.layerId]); PM.toast('Reduce shader complexity or turn off its visibility while editing.'); };
      } else if (issue.kind === 'extension') {
        action.textContent = 'Turn off';
        action.onclick = async () => {
          try { await PM.Kernel.bridge.setEnabled({ id: issue.id.slice(10), enabled: false }); performanceMonitor.clear(issue.id); refreshToken++; }
          catch { PM.toast('Could not turn off the extension. Open Mods to try again.'); }
        };
      }
      if (action.textContent) row.append(action);
      body.append(row);
    }
    PM.modal({ title: 'What is slowing the editor down', body, width: 520, actions: [{ label: 'Done' }] });
  }

  const saveStatus = $derived.by(() => {
    refreshToken;
    return PM.app?.saving ? 'SAVING…' : PM.app?.dirty ? 'UNSAVED' : 'SAVED';
  });
  const layerCount = $derived.by(() => {
    doc.tick.structure;
    const project = doc.proj?.layers ? doc.proj : PM.proj;
    return project?.layers?.length ?? 0;
  });
  const keyframeCount = $derived.by(() => {
    doc.tick.structure;
    doc.tick.values;
    const project = doc.proj?.layers ? doc.proj : PM.proj;
    const layers = project?.layers ?? [];
    return layers.reduce((total: number, layer: any) => {
      const props = typeof PM.allProps === 'function'
        ? PM.allProps(layer)
        : Object.values(layer?.p ?? {});
      return total + props.reduce((count: number, entry: any) => count + (entry?.prop?.kf ?? entry?.kf ?? []).length, 0);
    }, 0);
  });
  const webglLive = $derived.by(() => {
    refreshToken;
    return !!PM.GL?.gl;
  });
  const playbackStatus = $derived.by(() => {
    refreshToken;
    if (PM.GL?.contextLost) return 'Restoring GPU preview…';
    if (PM.GL?.compiling) return `Preparing ${PM.GL.compiling} shader${PM.GL.compiling === 1 ? '' : 's'}…`;
    if (PM.Preview?.preparing) return 'Preparing preview';
    if (PM.Preview?.active) return 'Cached preview';
    if (!transport.playing) return 'Playback paused';
    return perf.fps > 0 ? `Preview ${perf.fps} fps` : 'Measuring FPS…';
  });
  const frameMs = $derived.by(() => {
    const value = perf.ms || 0;
    return typeof PM.round === 'function'
      ? PM.round(value, 1)
      : Math.round(value * 10) / 10;
  });
  const dimensions = $derived.by(() => {
    doc.tick.project;
    const project = doc.proj?.layers ? doc.proj : PM.proj;
    return `${project?.w ?? 0}×${project?.h ?? 0} · ${project?.fps ?? 0} fps`;
  });

  /* Extension status items. `text()` is evaluated on the same tick the built-in
     fields use (`refreshToken`), and a null result hides the item entirely. */
  const contributions = $derived.by(() => {
    kernelSignals.status;
    refreshToken;
    const items = (PM.Kernel?.status?.list?.() ?? []) as Array<any>;
    return items
      .map((item) => ({ item, text: typeof item?.text === 'function' ? item.text() : null }))
      .filter((entry): entry is { item: any; text: string } => typeof entry.text === 'string');
  });
  const leftItems = $derived(contributions.filter((entry) => entry.item.side !== 'right'));
  const rightItems = $derived(contributions.filter((entry) => entry.item.side === 'right'));

  $effect(() => frameBus.on('status', () => refreshToken++));

  $effect(() => {
    const events = ['layers', 'project', 'history', 'assets'];
    const offs = events.map((event) => PM.bus?.on?.(event, () => refreshToken++));
    const interval = window.setInterval(() => PM.invalidate?.('status'), 1000);
    return () => {
      offs.forEach((off) => off?.());
      window.clearInterval(interval);
    };
  });
</script>

<span class="status-field">{saveStatus}</span>
<span class="status-field">{layerCount} layers</span>
<span class="status-field">{keyframeCount} keys</span>
{#if performanceIssues.length}
  <button type="button" class="status-field status-contribution" title={performanceIssues.map(issue => `${issue.name}: ${issue.ms.toFixed(1)} ms`).join('\n')} onclick={showPerformanceIssues}>⚠ Performance · {performanceIssues[0]?.name}</button>
{/if}
{#each leftItems as entry (entry.item.id)}
  {@render statusItem(entry)}
{/each}
<span class="sp" aria-hidden="true"></span>
<span class="status-field">
  {#if webglLive}<b class="live" aria-hidden="true">●</b> WebGL2{:else}WebGL unavailable{/if}
</span>
<span class="status-field" title="Measured preview redraws per second during playback">{playbackStatus}</span>
<span class="status-field" title="Preview render time">{frameMs} ms</span>
<span class="status-field">{dimensions}</span>
{#each rightItems as entry (entry.item.id)}
  {@render statusItem(entry)}
{/each}

{#snippet statusItem(entry: { item: any; text: string })}
  {#if entry.item.onClick}
    <button
      class="status-field status-contribution"
      type="button"
      title={entry.item.title ?? entry.text}
      onclick={() => entry.item.onClick()}
    >{entry.text}</button>
  {:else}
    <span class="status-field status-contribution" title={entry.item.title ?? entry.text}>{entry.text}</span>
  {/if}
{/snippet}

<style>
  /* Contributed items sit in the same rhythm as the built-in fields; the
     clickable variant only adds a pointer and a hover tint. */
  button.status-contribution {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }
  button.status-contribution:hover {
    color: var(--tx);
  }
</style>
