<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { api, doc, mixed } = inspectorContext();
  const { Section, Row, ToggleField } = api.ui.controls;

  let { layer }: {
    layer: Record<string, any>;
  } = $props();
  const modelGroup = $derived((doc.tick.structure,doc.tick.values,doc.proj,!!api.scene3d?.isGroup?.(layer.id)));
  const threeD = $derived((doc.tick.values, doc.tick.history, doc.proj, !!layer.threeD || modelGroup));
  /* A 3D scene layer edits its objects in the Scene section; this section
     only places the rendered scene in the composition. */
  const sceneLayer = $derived((doc.tick.structure, doc.proj,
    layer.type === 'extension' && api.layers.get(String(layer.d?.definition ?? ''))?.renderer.kind === 'scene3d'));
  const modelRole = $derived(modelGroup?'object':layer.type==='extension' && api.layers.get(String(layer.d?.definition ?? ''))?.renderer.kind === 'layer3d' ? (layer.d?.data?.object?'object':layer.d?.data?.light?'light':'camera') : null);
  const edit3D = { mode: 'command' as const, label: '3D layer', origin: 'inspector' as const, command: (value: unknown) => ({type: 'set_layer' as const, target: layer.id, patch: {threeD: !!value}}) };
</script>

<Section {api} title={sceneLayer ? 'Layer Transform' : 'Transform'} />
{#if layer.type !== 'adjustment' && !sceneLayer && !modelRole}<Row {api} label="3D layer"><ToggleField {api} {mixed} get={() => threeD} edit={edit3D} label="3D layer" /></Row>{/if}
<ChannelRow {layer} channel="position.x" label="Position X" prefix="X" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
<ChannelRow {layer} channel="position.y" label="Position Y" prefix="Y" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
{#if threeD}<ChannelRow {layer} channel="position.z" label="Position Z" prefix="Z" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />{/if}
{#if !modelRole || modelRole === 'object'}
<ChannelRow {layer} channel="scale.x" label="Scale" />
{#if threeD}
  <ChannelRow {layer} channel="scale.z" label="Scale Z" prefix="Z" />
  <ChannelRow {layer} channel="orientation.x" label="Orientation X" />
  <ChannelRow {layer} channel="orientation.y" label="Orientation Y" />
  <ChannelRow {layer} channel="orientation.z" label="Orientation Z" />
  <ChannelRow {layer} channel="rotation.x" label="X Rotation" />
  <ChannelRow {layer} channel="rotation.y" label="Y Rotation" />
{/if}
<ChannelRow {layer} channel="rotation" label={threeD ? 'Z Rotation' : 'Rotation'} />
{/if}
{#if threeD && !modelRole}<ChannelRow {layer} channel="perspective" label="Perspective" min={0.001} unit="mm" />{/if}
<ChannelRow {layer} channel="opacity" label="Opacity" />
{#if !modelRole || modelRole==='object'}
<ChannelRow {layer} channel="anchor.x" label="Anchor X" prefix="X" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
<ChannelRow {layer} channel="anchor.y" label="Anchor Y" prefix="Y" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
{#if threeD}<ChannelRow {layer} channel="anchor.z" label="Anchor Z" prefix="Z" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />{/if}
{/if}
{#if !modelRole}<ChannelRow {layer} channel="skew" label="Skew" />{/if}
