<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { PanelProps } from 'powermove';

  let { api }: PanelProps = $props();
  let revision = $state(api!.project.revision());
  let note = $state('');
  let runs = $state(0);
  const off = api!.events.on('project:changed', () => { revision = api!.project.revision(); });
  onDestroy(() => off.dispose());

  async function run(): Promise<void> {
    const result = await api!.commands.run(`${api!.id}.command`);
    if (result === 'ran') runs += 1;
  }
</script>

<section class="fixture">
  <p class="row"><span>Project revision</span><b>{revision}</b></p>
  <p class="row"><span>Runs</span><b>{runs}</b></p>
  <input class="field" placeholder="Note" bind:value={note} aria-label="Note" />
  <button class="button" type="button" onclick={run}>Run sandbox command</button>
</section>

<style>
  .fixture { display: flex; flex-direction: column; gap: 6px; padding: var(--pad); }
  .row { display: flex; align-items: center; justify-content: space-between; min-height: var(--row-h); margin: 0; color: var(--tx-2); }
  .row b { color: var(--tx); font-weight: var(--fw-medium); font-variant-numeric: tabular-nums; }
  .field { height: var(--ctl-h); padding: 0 8px; border: 0; border-radius: var(--r-sm); background: var(--bg-field); color: var(--tx); font: inherit; outline: none; }
  .field:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .button { height: var(--ctl-h); border: 0; border-radius: var(--r-sm); background: var(--ink-2); color: var(--tx); font: inherit; font-weight: var(--fw-medium); }
  .button:hover { background: var(--ink-3); }
  .button:focus-visible { box-shadow: 0 0 0 2px var(--accent); outline: none; }
</style>
