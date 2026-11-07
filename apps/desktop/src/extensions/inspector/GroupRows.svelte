<script lang="ts">
  /* A group's own setting: isolating keeps its members' blend modes from
     reaching the layers below (Figma's "Normal" group); off passes them through. */
  import AnimatedRow from './AnimatedRow.svelte';
  import { inspectorContext } from './context';
  import { isProperty } from 'powermove';

  let { layer }: { layer: any } = $props();
  const { api, doc, transport, mixed } = inspectorContext();
  const { ToggleField } = api.ui.controls;
  const { contentBinding } = api.ui.controls.binding;
  const isolate = () => (doc.tick.values, transport.time,
    !!(isProperty(layer.d?.isolate) ? api.anim.evP(layer, layer.d.isolate, transport.time, 'c.isolate') : layer.d?.isolate));
</script>

<div class="group-rows">
  <AnimatedRow {layer} label="Isolate" path="c.isolate">
    <ToggleField {api} {mixed} get={isolate} edit={contentBinding(layer.id, 'isolate', { label: 'Isolate blending', origin: 'inspector' })} label="Isolate blending" />
  </AnimatedRow>
</div>

<style>
  .group-rows { padding-top: 8px; }
</style>
