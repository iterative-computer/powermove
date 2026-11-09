<script lang="ts">
  import { untrack } from 'svelte';
  import type { PowermoveAPI } from 'powermove';
  /* Studio shots for a group of models: the whole product turns, or its
     parts move apart (exploded view). Both write ordinary keyframes. */
  let { api, layerId }: { api: PowermoveAPI; layerId: string } = $props();
  const { Section, Row, SelectField } = untrack(() => api.ui.controls);
  let amount = $state('0.5');
  function run(args: any, label: string): void {
    const result = api.scene3d.edit({ target: layerId, duration: 2, ...args }, { label, origin: 'inspector' });
    if (!result.ok) api.ui.toast(result.message, { error: true });
    api.transport.invalidate();
  }
</script>
<Section {api} title="Shots" />
<Row {api} label="Spread"><SelectField {api} get={() => amount} options={[{ v: '0.25', label: 'A little' }, { v: '0.5', label: 'Medium' }, { v: '1', label: 'Far apart' }]} edit={{ mode: 'local', label: 'Exploded view spread', set: (v: unknown) => { amount = String(v); } }} label="Exploded view spread" /></Row>
<div class="shots" role="group" aria-label="Group shots">
  <button type="button" class="chip" onclick={() => run({ operation: 'explode', amount: Number(amount) }, 'Exploded view')}>Exploded view</button>
  <button type="button" class="chip" onclick={() => run({ operation: 'shot', shot: 'turntable', duration: 4 }, 'Turntable')}>Turntable</button>
</div>
<style>
  .shots { display: flex; flex-wrap: wrap; gap: 6px; padding: 2px var(--panel-edge-inset, 10px) 8px; }
</style>
