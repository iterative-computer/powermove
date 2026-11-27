<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { api, doc, mixed } = inspectorContext();
  const { Section, Row, ToggleField } = api.ui.controls;

  let { layer }: {
    layer: Record<string, any>;
  } = $props();
  const threeD = $derived((doc.tick.values, doc.tick.history, doc.proj, !!layer.threeD));
  const edit3D = { mode: 'command' as const, label: '3D layer', origin: 'inspector' as const, command: (value: unknown) => ({type: 'set_layer' as const, target: layer.id, patch: {threeD: !!value}}) };
</script>

<Section {api} title="Transform" />
{#if layer.type !== 'adjustment'}<Row {api} label="3D layer"><ToggleField {api} {mixed} get={() => threeD} edit={edit3D} label="3D layer" /></Row>{/if}
<ChannelRow {layer} channel="position.x" label="Position X" prefix="X" />
<ChannelRow {layer} channel="position.y" label="Position Y" prefix="Y" />
{#if threeD}<ChannelRow {layer} channel="position.z" label="Position Z" prefix="Z" />{/if}
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
{#if threeD}<ChannelRow {layer} channel="perspective" label="Perspective" min={0.001} unit="mm" />{/if}
<ChannelRow {layer} channel="opacity" label="Opacity" />
<ChannelRow {layer} channel="anchor.x" label="Anchor X" prefix="X" />
<ChannelRow {layer} channel="anchor.y" label="Anchor Y" prefix="Y" />
{#if threeD}<ChannelRow {layer} channel="anchor.z" label="Anchor Z" prefix="Z" />{/if}
<ChannelRow {layer} channel="skew" label="Skew" />
