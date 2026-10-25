<script lang="ts">
  /* Performance panel with the same .row.split markup used by the shared CSS.
     The perf store is written ≤2 Hz by the runtime bridge and the counts
     derive from those ticks, with no manual DOM sync or bus listener. */
  import { doc } from '../state/document.svelte';
  import { audioStatus, perf } from '../state/transport.svelte';
  import { audioPerfRows } from '../engine-connection/audio-diagnostics';
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
    [perf.native ? 'Layers drawn' : 'GL draws', perf.draws],
    ['FX passes', perf.passes],
    ['Programs', perf.progs],
    ['Raster cache', perf.raster],
    ['Layers', layers.length],
    ['Keyframes', keyframes]
  ]);
  /* The engine's audio health, after the counts (none with the new engine
     off): underruns, overloads, waits, and each problem it reported. */
  const audioRows = $derived(audioPerfRows(audioStatus.health));
</script>

<div class="insp" data-svelte-panel={panelId}>
  {#each rows as [label, value] (label)}
    <div class="row split">
      <div class="k">{label}</div>
      <div class="vwrap"><span style="font-size:var(--fs-sm);font-variant-numeric:tabular-nums">{value}</span></div>
    </div>
  {/each}
  {#each audioRows as row, i (`${i}:${row.label}`)}
    <div class="row split" title={row.title}>
      <div class="k">{row.label}</div>
      <div class="vwrap"><span class="audio-value" style="font-size:var(--fs-sm);font-variant-numeric:tabular-nums">{row.value}</span></div>
    </div>
  {/each}
</div>

<style>
  /* A problem's message is one line; the row's tooltip has all of it. */
  .audio-value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
