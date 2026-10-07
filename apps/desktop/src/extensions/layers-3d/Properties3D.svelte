<script lang="ts">
  import { untrack } from 'svelte';
  import type { ControlEditBinding, Layer, PowermoveAPI } from 'powermove';
  import SceneField from './SceneField.svelte';
  import MaterialInspector from './MaterialInspector.svelte';
  import ModelInspector from './ModelInspector.svelte';
  import RenderingControls from './RenderingControls.svelte';
  import { LIGHT_TYPES, lightFields, cameraFields, ENVIRONMENT_FIELDS } from './scene-model';
  import { PROPERTY_TABS, tabsFor, type PropertyTab } from './property-tabs';
  import type { SceneUiState } from './scene-state.svelte';

  /*
   * Blender's Properties editor for one 3D layer: a column of context tabs
   * (Render, World, Object, Modifiers, Data, Material) beside their panels.
   * Transform stays in the native Transform section, as in Blender's Object tab.
   */
  let { api, ui, layerId }: { api: PowermoveAPI; ui: SceneUiState; layerId: string } = $props();
  const { Row, Section, ToggleField, SelectField, ColorField, Disclosure } = untrack(() => api.ui.controls);
  const STORAGE = 'powermove.properties3d.tab';

  const layer = $derived.by(() => { void ui.version; return api.model.layer(layerId) as Layer | null; });
  const d = $derived(layer?.d as any);
  const data = $derived(d?.data);
  const object = $derived(data?.object), light = $derived(data?.light), camera = $derived(data?.camera);
  const generatedBy = $derived(object?.generation?.groupId ? api.model.layer(object.generation.groupId) : null);
  const modelLayer = $derived(d?.modeling ? layer : (generatedBy as any)?.d?.modeling ? generatedBy : null);
  /* The World lives on the composition's active camera; other layers edit it there. */
  const worldCamera = $derived.by(() => {
    void ui.version;
    if (camera) return layer;
    return api.model.curComp().layers.find((l: any) => l.d?.definition === 'powermove.3d.camera' && api.anim.active(l, ui.time)) ?? null;
  });
  const tabs = $derived(tabsFor({ object: !!object, light: !!light, camera: !!camera, model: !!modelLayer }));
  /* Like Blender, the last chosen tab is kept while it applies; otherwise each kind of layer opens on its main tab. */
  let chosen = $state<PropertyTab | null>(untrack(() => globalThis.localStorage?.getItem(STORAGE) as PropertyTab | null));
  const tab = $derived<PropertyTab>(chosen && tabs.includes(chosen) ? chosen : modelLayer && !object ? 'modifiers' : object ? 'material' : light || camera ? 'data' : tabs[0]!);
  function choose(next: PropertyTab) { chosen = next; try { globalThis.localStorage?.setItem(STORAGE, next); } catch { /* optional */ } }

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
</script>

{#if layer && (data || modelLayer)}
  <div class="props3d" data-properties-3d>
    <div class="tabs" role="tablist" aria-orientation="vertical" aria-label="3D properties">
      {#each tabs as id (id)}
        {@const info = PROPERTY_TABS[id]}
        <button type="button" role="tab" aria-selected={tab === id} aria-label={info.label} title={info.label}
          style={`--tab:${info.color}`} onclick={() => choose(id)}>
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html info.icon(light?.type, !!camera)}</svg>
        </button>
      {/each}
    </div>
    <div class="panel" role="tabpanel" aria-label={PROPERTY_TABS[tab].label}>
      {#if tab === 'render'}
        <RenderingControls {api} {ui} />
      {:else if tab === 'world'}
        <Section {api} title="World" />
        {#if worldCamera}
          {@const environment = (worldCamera.d as any).data.environment}
          {#each ENVIRONMENT_FIELDS as field (field.key)}<SceneField {api} {ui} layer={worldCamera} {field} path={`environment.${field.key}`} channel={value(environment, field.key)} />{/each}
          <Row {api} label="Shadows"><ToggleField {api} get={() => environment.shadows} edit={structural('set_environment', 'shadows', 'Shadows', worldCamera.id)} label="Shadows" /></Row>
          <Row {api} label="Background"><ToggleField {api} get={() => !!environment.background} edit={{ mode: 'set', label: 'Background', set: (v: unknown) => edit('set_environment', { background: v ? '#161616' : null }, 'Background', worldCamera.id) }} label="Background" />{#if environment.background}<ColorField {api} get={() => environment.background} edit={structural('set_environment', 'background', 'Background color', worldCamera.id)} label="Background color" />{/if}</Row>
          {#if !camera}<Row {api} label="Camera"><button type="button" class="chip" onclick={() => api.selection.select([worldCamera.id])}>{worldCamera.name}</button></Row>{/if}
        {:else}
          <Row {api} label="World"><button type="button" class="chip" onclick={() => api.commands.run('3d.add-camera')}>Add camera</button></Row>
        {/if}
      {:else if tab === 'object' && object}
        <Section {api} title="Object" />
        <Row {api} label="Source"><span class="k">{sourceName}</span></Row>
        {#if object.source.mesh}<Row {api} label="Triangles"><span class="k">{Math.floor((object.source.mesh.indices?.length ?? object.source.mesh.positions.length / 3) / 3).toLocaleString()}</span></Row>{/if}
        {#if generatedBy}<Row {api} label="Model"><button type="button" class="chip" onclick={() => api.selection.select([generatedBy.id])}>{generatedBy.name}</button></Row>{/if}
        {#if object.source.assetId}<Row {api} label="Model materials"><ToggleField {api} get={() => object.useSourceMaterials} edit={structural('update_object', 'useSourceMaterials', 'Model materials')} label="Model materials" /></Row>{/if}
        <Disclosure label="Visibility" remember={`visibility:${layerId}`} changed={!object.castShadow || !object.receiveShadow}>
          <Row {api} label="Cast shadows"><ToggleField {api} get={() => object.castShadow} edit={structural('update_object', 'castShadow', 'Cast shadows')} label="Cast shadows" /></Row>
          <Row {api} label="Receive shadows"><ToggleField {api} get={() => object.receiveShadow} edit={structural('update_object', 'receiveShadow', 'Receive shadows')} label="Receive shadows" /></Row>
        </Disclosure>
      {:else if tab === 'modifiers' && modelLayer}
        <ModelInspector {api} {ui} layerId={modelLayer.id} />
      {:else if tab === 'data' && light}
        <Section {api} title="Light" />
        <Row {api} label="Type"><SelectField {api} get={() => light.type} options={LIGHT_TYPES.map((t) => ({ v: t.id, label: t.label }))} edit={structural('update_light', 'type', 'Light type')} label="Light type" /></Row>
        {#each lightFields(light.type).filter((f) => !['x', 'y', 'z'].includes(f.key) && (!api.scene3d.getRendering().enabled || f.key !== 'decay')) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`light.${field.key}`} channel={value(light, field.key)} />{/each}
        <Row {api} label="Cast shadows"><ToggleField {api} get={() => light.castShadow} edit={structural('update_light', 'castShadow', 'Cast shadows')} label="Cast shadows" /></Row>
      {:else if tab === 'data' && camera}
        <Section {api} title="Camera" />
        <Row {api} label="Projection"><SelectField {api} get={() => camera.projection} options={[{ v: 'perspective', label: 'Perspective' }, { v: 'orthographic', label: 'Orthographic' }]} edit={structural('set_camera', 'projection', 'Camera projection')} label="Projection" /></Row>
        {#each cameraFields(camera.projection).filter((f) => !['x', 'y', 'z'].includes(f.key)) as field (field.key)}<SceneField {api} {ui} {layer} {field} path={`camera.${field.key}`} channel={value(camera, field.key)} />{/each}
        <Row {api} label="Active camera"><button type="button" class="chip" onclick={() => api.scene3d.viewport.run('view.set-active-camera')}>Set active</button><button type="button" class="chip" onclick={() => api.scene3d.viewport.run('view.camera')}>Look through</button></Row>
      {:else if tab === 'material' && object}
        {#if !object.useSourceMaterials || object.slots?.length}
          <MaterialInspector {api} {ui} {layer} {object} />
        {:else}
          <Section {api} title="Material" />
          <Row {api} label="Materials"><span class="k">From the imported model</span><button type="button" class="chip" onclick={() => edit('update_object', { useSourceMaterials: false }, 'Model materials')}>Edit here</button></Row>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .props3d{display:flex;align-items:flex-start;gap:6px;min-width:0}
  .tabs{position:sticky;top:0;display:flex;flex-direction:column;gap:2px;padding:4px 0;flex:none}
  .tabs button{all:unset;box-sizing:border-box;width:28px;height:28px;display:grid;place-items:center;border-radius:6px;color:var(--tab);cursor:pointer}
  .tabs button:hover{background:color-mix(in srgb,var(--tx) 8%,transparent)}
  .tabs button[aria-selected="true"]{background:color-mix(in srgb,var(--tx) 13%,transparent)}
  .tabs button:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}
  .tabs svg{width:17px;height:17px}
  .panel{flex:1;min-width:0}
</style>
