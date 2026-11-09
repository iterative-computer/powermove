<script lang="ts">
  import type { Layer, PowermoveAPI } from 'powermove';
  import MaterialOptionsRows from './MaterialOptionsRows.svelte';
  import type { SceneUiState } from './scene-state.svelte';
  /* Material Options for 3D artwork: solids, footage, text, shapes and pre-comps. */
  let { api, ui, layerId }: { api: PowermoveAPI; ui: SceneUiState; layerId: string } = $props();
  /* The layer object keeps its identity across edits; read its options through ui.version. */
  const materialOptionsOf = (l: any): any => (ui.version, l?.d?.materialOptions);
  const layer = $derived((ui.version, api.model.layer(layerId)) as Layer | null);
</script>
{#if layer}
  <MaterialOptionsRows {api} {ui} {layer} options={materialOptionsOf(layer)} fallback={(key) => key === 'castsShadows' ? 'on' : true} />
{/if}
