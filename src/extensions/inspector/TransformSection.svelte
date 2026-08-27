<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { Section } = inspectorContext().api.ui.controls;

  let { PM, layer }: {
    PM: Record<string, any>;
    layer: Record<string, any>;
  } = $props();

  const channels = [
    ['position.x', 'Position X'],
    ['position.y', 'Position Y'],
    ['scale.x', 'Scale X'],
    ['scale.y', 'Scale Y'],
    ['rotation', 'Rotation'],
    ['opacity', 'Opacity'],
    ['anchor.x', 'Anchor X'],
    ['anchor.y', 'Anchor Y'],
    ['skew', 'Skew']
  ] as const;
</script>

<Section title="Transform" />
{#each channels as [channel, label] (channel)}
  <ChannelRow {PM} {layer} {channel} {label} />
{/each}
