<script lang="ts">
  import { kernelSignals } from '../kernel/signals.svelte';
  import { frameBus } from '../runtime/frame-bus';
  import { doc } from '../state/document.svelte';
  import { perf } from '../state/transport.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let refreshToken = $state(0);

  const saveStatus = $derived.by(() => {
    refreshToken;
    return PM.app?.dirty ? 'UNSAVED' : 'SAVED';
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
  const displayFps = $derived.by(() => perf.fps || '—');
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
{#each leftItems as entry (entry.item.id)}
  {@render statusItem(entry)}
{/each}
<span class="sp" aria-hidden="true"></span>
<span class="status-field">
  {#if webglLive}<b class="live" aria-hidden="true">●</b> WebGL2{:else}WebGL unavailable{/if}
</span>
<span class="status-field">{displayFps} fps</span>
<span class="status-field">{frameMs} ms</span>
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
