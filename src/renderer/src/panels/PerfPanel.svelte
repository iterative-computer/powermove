<script lang="ts">
  /* Pilot Svelte panel (replaces legacy ui/panels.js 'perf'). Same rows and
     the same .row.split markup PM.row() produced, so css/app.css applies
     unchanged — but no manual DOM sync and no bus listener: the perf store is
     written ≤2 Hz by the runtime bridge, the counts derive from the ticks. */
  import { doc } from '../state/document.svelte';
  import { perf } from '../state/transport.svelte';
  import type { PanelProps } from './registerSveltePanel';

  let { panelId }: PanelProps = $props();

  const layers = $derived((doc.tick.structure, doc.proj?.layers ?? []));
  const keyframes = $derived(
    (doc.tick.values, layers.reduce((total: number, layer: any) => {
      for (const key in layer.p ?? {}) total += layer.p[key]?.kf?.length ?? 0;
      return total;
    }, 0))
  );
  const rows = $derived<[string, string | number][]>([
    ['FPS', perf.fps || '—'],
    ['Frame ms', Math.round(perf.ms * 100) / 100],
    ['GL draws', perf.draws],
    ['FX passes', perf.passes],
    ['Programs', perf.progs],
    ['Raster cache', perf.raster],
    ['Layers', layers.length],
    ['Keyframes', keyframes]
  ]);
</script>

<div class="insp" data-svelte-panel={panelId}>
  {#each rows as [label, value] (label)}
    <div class="row split">
      <div class="k">{label}</div>
      <div class="vwrap"><span style="font-size:11.5px;font-variant-numeric:tabular-nums">{value}</span></div>
    </div>
  {/each}
</div>
