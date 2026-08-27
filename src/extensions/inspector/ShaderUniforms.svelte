<script lang="ts">
  import { inspectorContext, type EditBinding } from './context';
  import ChannelRow from './ChannelRow.svelte';

  const { api, doc } = inspectorContext();
  const { ColorField, Row, Section, ToggleField } = api.ui.controls;

  let { PM, layer }: {
    PM: Record<string, any>;
    layer: Record<string, any>;
  } = $props();

  let definitions = $state<any[]>([]);

  $effect(() => {
    doc.tick.structure;
    doc.proj;
    layer.d?.code;
    PM.syncShaderUniforms?.(layer);
    definitions = PM.UIState?.getShaderMeta?.(layer)?.udefs ?? [];
  });

  function fieldBinding(path: string, label: string): EditBinding {
    return {
      mode: 'command',
      label,
      origin: 'inspector',
      command: (value: unknown) => ({
        type: 'set_property',
        target: layer.id,
        path,
        value: value as any,
        time: PM.time,
        preserveHandEdits: false
      })
    };
  }
</script>

{#if definitions.length}
  <Section title="Shader" />
  {#each definitions as definition (definition.name)}
    {@const property = layer.d?.uniforms?.[definition.name]}
    {#if property}
      {@const path = `u.${definition.name}`}
      {#if definition.control === 'color'}
        <Row label={definition.label}>
          <ColorField
            {PM}
            get={() => (doc.tick.values, doc.proj, property.v)}
            edit={fieldBinding(path, definition.label)}
            label={definition.label}
          />
        </Row>
      {:else if definition.control === 'toggle'}
        <Row label={definition.label}>
          <ToggleField
            {PM}
            get={() => (doc.tick.values, doc.proj, property.v)}
            edit={fieldBinding(path, definition.label)}
            label={definition.label}
          />
        </Row>
      {:else}
        <ChannelRow
          {PM}
          {layer}
          channel={path}
          label={definition.label}
          {property}
          getValue={(time) => PM.evP(layer, property, time, definition.name)}
          step={(definition.max - definition.min) / 200 || .01}
          min={definition.min}
          max={definition.max}
          precision={3}
          showDiamond={false}
        />
      {/if}
    {/if}
  {/each}
{/if}
