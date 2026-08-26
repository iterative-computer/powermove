<script lang="ts">
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
<span class="sp" aria-hidden="true"></span>
<span class="status-field">
  {#if webglLive}<b class="live" aria-hidden="true">●</b> WebGL2{:else}WebGL unavailable{/if}
</span>
<span class="status-field">{displayFps} fps</span>
<span class="status-field">{frameMs} ms</span>
<span class="status-field">{dimensions}</span>
