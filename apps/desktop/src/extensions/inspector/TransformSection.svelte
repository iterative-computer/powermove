<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';
  import AnimatedRow from './AnimatedRow.svelte';
  import { isProperty } from 'powermove';

  const { api, doc, mixed, transport } = inspectorContext();
  const { Section, SelectField, Disclosure } = api.ui.controls;

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
  /* More reads as changed while Anchor Point or Skew is animated or moved. */
  const moreChanged = $derived((doc.tick.values, doc.tick.history, doc.proj, ['anchor.x', 'anchor.y', 'anchor.z', 'skew']
    .some((key) => { const p = layer.p?.[key]; return !!(p?.kf?.length || p?.expr || (key === 'skew' && Number(p?.v) !== 0)); })));
</script>

<Section {api} title={sceneLayer ? 'Layer Transform' : 'Transform'} />
<!-- After the prototype: Position, Scale, Rotation, Opacity and Blend Mode;
     Anchor Point and Skew fold under More. 3D is the timeline's cube switch. -->
<ChannelRow {layer} channel="position.x" pairWith="position.y" label="Position" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
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
{#if layer.type !== 'adjustment' && !sceneLayer}
  <AnimatedRow {layer} path="l.blend" label="Blend Mode"><SelectField {api} {mixed}
    get={() => (doc.tick.values, transport.time, isProperty(layer.blend) ? api.anim.evP(layer as any, layer.blend, transport.time, 'l.blend') : layer.blend)}
    edit={{ mode: 'command', label: 'Blend', origin: 'inspector', command: (value: unknown) => ({ type: 'set_property', target: String(layer.id), path: 'l.blend', value: value as any, time: transport.time, mode: 'auto', preserveHandEdits: false }) }}
    options={api.model.BLENDS ?? []} label="Blend mode" /></AnimatedRow>
{/if}
{#if !modelRole || modelRole === 'object'}
  <Disclosure count={(threeD ? 2 : 1) + (modelRole ? 0 : 1)} changed={moreChanged} remember={`${doc.proj.id}:${layer.id}:transform-more`}>
    <ChannelRow {layer} channel="anchor.x" pairWith="anchor.y" label="Anchor Point" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
    {#if threeD}<ChannelRow {layer} channel="anchor.z" label="Anchor Z" prefix="Z" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />{/if}
    {#if !modelRole}<ChannelRow {layer} channel="skew" label="Skew" />{/if}
  </Disclosure>
{/if}
