<script lang="ts">
  import { untrack } from 'svelte';
  import type { Layer, PowermoveAPI } from 'powermove';
  import SceneField from './SceneField.svelte';
  import type { SceneUiState } from './scene-state.svelte';
  import { MATERIAL_OPTION_FIELDS } from './environment-ui';
  /* After Effects' Material Options. Until a layer first changes one they are
     plain controls; that edit installs all four as keyframeable channels.
     `options` is where the layer keeps them; `fallback` reads older shadow flags. */
  let { api, ui, layer, options, fallback, heading = true }: {
    api: PowermoveAPI; ui: SceneUiState; layer: Layer; options: any;
    fallback: (key: string) => unknown; heading?: boolean;
  } = $props();
  const { Row, Section, SelectField, ToggleField } = untrack(() => api.ui.controls);
  function set(key: string, v: unknown, label: string): void {
    const result = api.scene3d.edit({ operation: 'set_material_options', target: layer.id, patch: { [key]: v } }, { label, origin: 'inspector' });
    if (!result.ok) api.ui.toast(result.message, { error: true });
    api.transport.invalidate();
  }
</script>
{#if heading}<Section {api} title="Material Options" />{/if}
<div class="material-options" data-material-options>
{#each MATERIAL_OPTION_FIELDS as field (field.key)}
  {#if options}<SceneField {api} {ui} {layer} {field} path={`materialOptions.${field.key}`} channel={options.p[field.key]} />
  {:else if field.kind === 'select'}<Row {api} label={field.label}><SelectField {api} get={() => fallback(field.key)} options={(field.options ?? []).map(o => ({ v: String(o.v), label: o.label }))} edit={{ mode: 'set', label: field.label, set: (v: unknown) => set(field.key, v, field.label) }} label={field.label} /></Row>
  {:else}<Row {api} label={field.label}><ToggleField {api} get={() => fallback(field.key)} edit={{ mode: 'set', label: field.label, set: (v: unknown) => set(field.key, !!v, field.label) }} label={field.label} /></Row>{/if}
{/each}
</div>
<style>
  /* After Effects' names are long; they wrap rather than lose their ends. */
  .material-options :global(.row .k) { white-space: normal; overflow: visible; text-overflow: clip; line-height: 1.15; }
</style>
