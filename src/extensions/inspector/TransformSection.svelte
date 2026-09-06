<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { Section } = inspectorContext().api.ui.controls;

  let { PM, layer }: {
    PM: Record<string, any>;
    layer: Record<string, any>;
  } = $props();
</script>

<Section title="Transform" />
<ChannelRow {PM} {layer} channel="position.x" label="Position X" prefix="X" />
<ChannelRow {PM} {layer} channel="position.y" label="Position Y" prefix="Y" />
<ChannelRow {PM} {layer} channel="scale.x" label="Scale" />
<ChannelRow {PM} {layer} channel="rotation" label="Rotation" />
<ChannelRow {PM} {layer} channel="opacity" label="Opacity" />
<ChannelRow {PM} {layer} channel="anchor.x" label="Anchor X" prefix="X" />
<ChannelRow {PM} {layer} channel="anchor.y" label="Anchor Y" prefix="Y" />
<ChannelRow {PM} {layer} channel="skew" label="Skew" />
