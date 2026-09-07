<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { api, doc } = inspectorContext();
  const { Section, Row, ToggleField } = api.ui.controls;

  let { PM, layer }: {
    PM: Record<string, any>;
    layer: Record<string, any>;
  } = $props();
  const threeD = $derived((doc.tick.values, doc.tick.history, doc.proj, !!layer.threeD));
  const edit3D = { mode: 'command' as const, label: '3D layer', origin: 'inspector' as const, command: (value: unknown) => ({type: 'set_layer' as const, target: layer.id, patch: {threeD: !!value}}) };
</script>

<Section title="Transform" />
{#if layer.type !== 'adjustment'}<Row label="3D layer"><ToggleField {PM} get={() => threeD} edit={edit3D} label="3D layer" /></Row>{/if}
<ChannelRow {PM} {layer} channel="position.x" label="Position X" prefix="X" />
<ChannelRow {PM} {layer} channel="position.y" label="Position Y" prefix="Y" />
{#if threeD}<ChannelRow {PM} {layer} channel="position.z" label="Position Z" prefix="Z" />{/if}
<ChannelRow {PM} {layer} channel="scale.x" label="Scale" />
{#if threeD}
  <ChannelRow {PM} {layer} channel="scale.z" label="Scale Z" prefix="Z" />
  <ChannelRow {PM} {layer} channel="orientation.x" label="Orientation X" />
  <ChannelRow {PM} {layer} channel="orientation.y" label="Orientation Y" />
  <ChannelRow {PM} {layer} channel="orientation.z" label="Orientation Z" />
  <ChannelRow {PM} {layer} channel="rotation.x" label="X Rotation" />
  <ChannelRow {PM} {layer} channel="rotation.y" label="Y Rotation" />
{/if}
<ChannelRow {PM} {layer} channel="rotation" label={threeD ? 'Z Rotation' : 'Rotation'} />
{#if threeD}<ChannelRow {PM} {layer} channel="perspective" label="Perspective" min={0.001} unit="mm" />{/if}
<ChannelRow {PM} {layer} channel="opacity" label="Opacity" />
<ChannelRow {PM} {layer} channel="anchor.x" label="Anchor X" prefix="X" />
<ChannelRow {PM} {layer} channel="anchor.y" label="Anchor Y" prefix="Y" />
{#if threeD}<ChannelRow {PM} {layer} channel="anchor.z" label="Anchor Z" prefix="Z" />{/if}
<ChannelRow {PM} {layer} channel="skew" label="Skew" />
