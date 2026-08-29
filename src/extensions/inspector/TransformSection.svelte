<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { Section, Row } = inspectorContext().api.ui.controls;

  let { PM, layer }: {
    PM: Record<string, any>;
    layer: Record<string, any>;
  } = $props();
</script>

<Section title="Transform" />
<Row label="Position" pair>
  <ChannelRow {PM} {layer} channel="position.x" label="Position X" compact prefix="X" />
  <ChannelRow {PM} {layer} channel="position.y" label="Position Y" compact prefix="Y" />
</Row>
<ChannelRow {PM} {layer} channel="scale.x" label="Scale" />
<ChannelRow {PM} {layer} channel="rotation" label="Rotation" />
<ChannelRow {PM} {layer} channel="opacity" label="Opacity" />
<Row label="Anchor" pair>
  <ChannelRow {PM} {layer} channel="anchor.x" label="Anchor X" compact prefix="X" />
  <ChannelRow {PM} {layer} channel="anchor.y" label="Anchor Y" compact prefix="Y" />
</Row>
<ChannelRow {PM} {layer} channel="skew" label="Skew" />
