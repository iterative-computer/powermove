<script lang="ts">
  import { inspectorContext, type EditBinding } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';

  const { api, doc, transport, controlProps, inspector } = inspectorContext();
  const { ColorField, Section, ToggleField } = api.ui.controls;

  let { layer }: { layer: any } = $props();

  let definitions = $state<any[]>([]);

  $effect(() => {
    doc.tick.structure;
    doc.proj;
    layer.d?.code;
    const service = inspector();
    service?.syncShaderUniforms(layer as any);
    definitions = service?.shaderDefinitions(layer as any) ?? [];
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
        time: api.transport.time(),
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
        <AnimatedRow {layer} {path} label={definition.label}>
          <ColorField
            {...controlProps}

            get={() => (doc.tick.values, doc.proj, transport.time, api.anim.evP(layer, property, transport.time, path))}
            edit={fieldBinding(path, definition.label)}
            label={definition.label}
          />
        </AnimatedRow>
      {:else if definition.control === 'toggle'}
        <AnimatedRow {layer} {path} label={definition.label}>
          <ToggleField
            {...controlProps}

            get={() => (doc.tick.values, doc.proj, transport.time, api.anim.evP(layer, property, transport.time, path))}
            edit={fieldBinding(path, definition.label)}
            label={definition.label}
          />
        </AnimatedRow>
      {:else}
        <ChannelRow

          {layer}
          channel={path}
          label={definition.label}
          {property}
          getValue={(time) => api.anim.evP(layer, property, time, definition.name)}
          step={(definition.max - definition.min) / 200 || .01}
          min={definition.min}
          max={definition.max}
          precision={3}
        />
      {/if}
    {/if}
  {/each}
{/if}
