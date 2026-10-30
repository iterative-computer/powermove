<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';
  import AnimatedRow from './AnimatedRow.svelte';
  import { isProperty } from 'powermove';

  const { api, doc, mixed, transport } = inspectorContext();
  const { Section, SelectField } = api.ui.controls;

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
</script>

<Section {api} title={sceneLayer ? 'Layer Transform' : 'Transform'}>
  <!-- In 3D every pair row gains a Z, so the heading names the columns once
       instead of squeezing a letter into each field. -->
  {#if threeD}<div class="axis-head" aria-hidden="true"><span>X</span><span>Y</span><span>Z</span></div>{/if}
</Section>
<!-- One row per property, with X, Y and (in 3D) Z side by side in one well,
     so a 3D layer reads as compactly as a 2D one. 3D is the timeline's cube switch. -->
{#if !modelRole || modelRole === 'object'}
  <ChannelRow {layer} channel="anchor.x" pairWith="anchor.y" thirdWith={threeD ? 'anchor.z' : undefined} label="Anchor Point" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
{/if}
<ChannelRow {layer} channel="position.x" pairWith="position.y" thirdWith={threeD ? 'position.z' : undefined} label="Position" unit={modelRole?'':undefined} step={modelRole?.01:undefined} />
{#if !modelRole || modelRole === 'object'}
<ChannelRow {layer} channel="scale.x" thirdWith={threeD ? 'scale.z' : undefined} label="Scale" />
{#if threeD}
  <ChannelRow {layer} channel="orientation.x" pairWith="orientation.y" thirdWith="orientation.z" label="Orientation" />
  <ChannelRow {layer} channel="rotation.x" pairWith="rotation.y" thirdWith="rotation" label="Rotation" />
{:else}
  <ChannelRow {layer} channel="rotation" label="Rotation" />
{/if}
{#if !modelRole}<ChannelRow {layer} channel="skew" label="Skew" />{/if}
{/if}
{#if threeD && !modelRole}<ChannelRow {layer} channel="perspective" label="Perspective" min={0.001} unit="mm" />{/if}
<ChannelRow {layer} channel="opacity" label="Opacity" />
{#if layer.type !== 'adjustment' && !sceneLayer}
  <AnimatedRow {layer} path="l.blend" label="Blend Mode"><SelectField {api} {mixed}
    get={() => (doc.tick.values, transport.time, isProperty(layer.blend) ? api.anim.evP(layer as any, layer.blend, transport.time, 'l.blend') : layer.blend)}
    edit={{ mode: 'command', label: 'Blend', origin: 'inspector', command: (value: unknown) => ({ type: 'set_property', target: String(layer.id), path: 'l.blend', value: value as any, time: transport.time, mode: 'auto', preserveHandEdits: false }) }}
    options={api.model.BLENDS ?? []} label="Blend mode" /></AnimatedRow>
{/if}

<style>
  /* Lines up over the value column: the row's 11px diamond, 78px label and
     two 8px gaps come before it, and each value starts 6px into its field. */
  .axis-head {
    flex: 0 0 calc(100% - 105px);
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
    font-family: var(--f-mono);
    font-size: 10px;
    font-weight: var(--fw-regular, 400);
    color: var(--tx-3);
  }
  .axis-head span { padding-left: 6px; }
  /* Matches the inspector's stacked X / Y / Z rows, whose fields carry their own letters. */
  @container (max-width: 290px) { .axis-head { display: none; } }
</style>
