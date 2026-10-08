<script lang="ts">
  import { untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import SceneField from './SceneField.svelte';
  import MaterialInspector from './MaterialInspector.svelte';
  import ModelInspector from './ModelInspector.svelte';
  import RenderingControls from './RenderingControls.svelte';
  import LightingSection from './LightingSection.svelte';
  import { LIGHT_TYPES, lightFields, cameraFields, ENVIRONMENT_FIELDS } from './scene-model';
  import type { SceneUiState } from './scene-state.svelte';

  /*
   * One 3D layer's settings as ordinary inspector sections: the model and its
   * material, the light, or the camera, then the composition's lighting.
   * Position and rotation stay in the native Transform section.
   */
  let { api, ui, layerId }: { api: PowermoveAPI; ui: SceneUiState; layerId: string } = $props();
  const { Row, Section, ToggleField, SelectField, ColorField, Disclosure } = untrack(() => api.ui.controls);

  const layer = $derived.by(() => { void ui.version; return api.model.layer(layerId) as Layer | null; });
  const d = $derived(layer?.d as any);
  const data = $derived(d?.data);
  const object = $derived(data?.object), light = $derived(data?.light), camera = $derived(data?.camera);
  const generatedBy = $derived(object?.generation?.groupId ? api.model.layer(object.generation.groupId) : null);
  const modelLayer = $derived(d?.modeling ? layer : (generatedBy as any)?.d?.modeling ? generatedBy : null);
  const aim = (key: string) => key.startsWith('target');

  const value = (node: any, key: string) => node?.p?.[key];
  const assetName = (id: string) => (api.project.get().assets[id] as any)?.name;
  function edit(operation: any, patch: any, label: string, target = layerId) {
    const result = api.scene3d.edit({ operation, target, patch }, { label, origin: 'inspector' });
    if (!result.ok) api.ui.toast(result.message, { error: true });
    api.transport.invalidate();
  }
  const structural = (operation: any, key: string, label: string, target = layerId): ControlEditBinding => ({ mode: 'set', label, set: (v) => edit(operation, { [key]: v }, label, target) });
  const sourceName = $derived(object ? object.blender ? assetName(object.blender.assetId) ?? 'Blender' : object.source.assetId ? assetName(object.source.assetId) ?? 'Model'
    : object.source.primitive ?? (object.source.lathe ? 'Lathe' : object.source.extrude ? 'Extrusion' : 'Mesh') : '');
  const viewing = $derived((ui.version, api.scene3d.viewport.state().view.mode === 'camera'));
</script>

{#if layer && (data || modelLayer)}
  <div class="props3d" data-properties-3d>
    {#if object}
      <Section {api} title="Model" />
      <Row {api} label="Source"><span class="k">{sourceName}</span></Row>
      {#if generatedBy}<Row {api} label="Part of"><button type="button" class="chip" onclick={() => api.selection.select([generatedBy.id])}>{generatedBy.name}</button></Row>{/if}
      {#if object.source.assetId}<Row {api} label="Model materials"><ToggleField {api} get={() => object.useSourceMaterials} edit={structural('update_object', 'useSourceMaterials', 'Model materials')} label="Model materials" /></Row>{/if}
      <Disclosure label="Shadows" remember={`shadows:${layerId}`} changed={!object.castShadow || !object.receiveShadow}>
        <Row {api} label="Cast shadows"><ToggleField {api} get={() => object.castShadow} edit={structural('update_object', 'castShadow', 'Cast shadows')} label="Cast shadows" /></Row>
        <Row {api} label="Receive shadows"><ToggleField {api} get={() => object.receiveShadow} edit={structural('update_object', 'receiveShadow', 'Receive shadows')} label="Receive shadows" /></Row>
      </Disclosure>
      {#if !object.useSourceMaterials || object.slots?.length}
        <MaterialInspector {api} {ui} {layer} {object} />
      {:else}
        <Section {api} title="Material" />
        <Row {api} label="Materials"><span class="k">From the model</span><button type="button" class="chip" onclick={() => edit('update_object', { useSourceMaterials: false }, 'Model materials')}>Edit here</button></Row>
      {/if}
    {/if}
    {#if modelLayer}<ModelInspector {api} {ui} layerId={modelLayer.id} />{/if}
    {#if light}
      <Section {api} title="Light" />
      <Row {api} label="Type"><SelectField {api} get={() => light.type} options={LIGHT_TYPES.map((t) => ({ v: t.id, label: t.label }))} edit={structural('update_light', 'type', 'Light type')} label="Light type" /></Row>
      {#each lightFields(light.type).filter((f) => !['x', 'y', 'z', 'color', 'intensity'].includes(f.key) && !aim(f.key) && (!api.scene3d.getRendering().enabled || f.key !== 'decay')) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`light.${field.key}`} channel={value(light, field.key)} />{/each}
      <Row {api} label="Cast shadows"><ToggleField {api} get={() => light.castShadow} edit={structural('update_light', 'castShadow', 'Cast shadows')} label="Cast shadows" /></Row>
      {#if light.type !== 'point'}
        <Disclosure label="Aim point" remember={`aim:${layerId}`}>
          {#each lightFields(light.type).filter((f) => aim(f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`light.${field.key}`} channel={value(light, field.key)} />{/each}
        </Disclosure>
      {/if}
    {/if}
    {#if camera}
      {@const environment = data.environment}
      <Section {api} title="Camera" />
      <Row {api} label="Projection"><SelectField {api} get={() => camera.projection} options={[{ v: 'perspective', label: 'Perspective' }, { v: 'orthographic', label: 'Orthographic' }]} edit={structural('set_camera', 'projection', 'Camera projection')} label="Projection" /></Row>
      {#each cameraFields(camera.projection).filter((f) => f.key === 'fov' || f.key === 'zoom') as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera, field.key)} />{/each}
      <Row {api} label="View"><button type="button" class="chip" aria-pressed={viewing} onclick={() => api.scene3d.viewport.run(viewing ? 'view.free' : 'view.set-active-camera', layerId)}>{viewing ? 'Free view' : 'Look through'}</button></Row>
      <Disclosure label="Aim point" remember={`aim:${layerId}`}>
        {#each cameraFields(camera.projection).filter((f) => aim(f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera, field.key)} />{/each}
      </Disclosure>
      <Disclosure label="Clipping" remember={`clipping:${layerId}`}>
        {#each cameraFields(camera.projection).filter((f) => f.key === 'near' || f.key === 'far') as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera, field.key)} />{/each}
      </Disclosure>
      {#if environment}
        <Section {api} title="Scene" />
        {#each ENVIRONMENT_FIELDS.filter((f) => f.key === 'exposure') as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`environment.${field.key}`} channel={value(environment, field.key)} />{/each}
        <Row {api} label="Shadows"><ToggleField {api} get={() => environment.shadows} edit={structural('set_environment', 'shadows', 'Shadows')} label="Shadows" /></Row>
        <Row {api} label="Background"><ToggleField {api} get={() => !!environment.background} edit={{ mode: 'set', label: 'Background', set: (v: unknown) => edit('set_environment', { background: v ? '#161616' : null }, 'Background') }} label="Background" />{#if environment.background}<ColorField {api} get={() => environment.background} edit={structural('set_environment', 'background', 'Background color')} label="Background color" />{/if}</Row>
      {/if}
      <RenderingControls {api} {ui} />
    {/if}
    <LightingSection {api} {ui} focus={light ? layerId : null} />
  </div>
{/if}

<style>
  .props3d{display:flex;flex-direction:column;min-width:0}
</style>
