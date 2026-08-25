<script lang="ts">
  import ColorField from '../../controls/ColorField.svelte';
  import NumField from '../../controls/NumField.svelte';
  import Row from '../../controls/Row.svelte';
  import Section from '../../controls/Section.svelte';
  import SelectField, { type SelectOption } from '../../controls/SelectField.svelte';
  import ToggleField from '../../controls/ToggleField.svelte';
  import { layerFieldBinding } from '../../controls/binding';
  import type { EditBinding } from '../../controls/gesture';
  import { doc } from '../../state/document.svelte';

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  const edit = (field: string, label: string): EditBinding =>
    layerFieldBinding(PM, layer.id, field as any, { label, origin: 'inspector' });
  const parentOptions = $derived<SelectOption[]>((doc.tick.structure, doc.proj, [
    { v: null, label: 'none' },
    ...(PM.proj?.layers ?? [])
      .filter((candidate: any) => candidate.id !== layer.id && !PM.wouldCycle?.(layer, candidate.id))
      .map((candidate: any) => ({ v: candidate.id, label: String(candidate.name) }))
  ]));
</script>

<Section title="Layer" />

{#if layer.type === 'audio'}
  <Row label="Visible"><ToggleField {PM} get={() => layer.on} edit={edit('visible', 'Visibility')} label="Visibility" /></Row>
  <Row label="Color"><ColorField {PM} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
  <Row label="Start"><NumField {PM} get={() => layer.from} edit={edit('from', 'Start')} label="Start" step={0.05} precision={2} unit="s" /></Row>
  <Row label="Duration"><NumField {PM} get={() => layer.dur} edit={edit('duration', 'Duration')} label="Duration" step={0.05} precision={2} unit="s" /></Row>
{:else}
  <Row label="Blend mode"><SelectField {PM} get={() => layer.blend} edit={edit('blend', 'Blend')} options={PM.BLENDS ?? []} label="Blend" /></Row>
  <Row label="Motion blur"><ToggleField {PM} get={() => layer.mblur} edit={edit('motionBlur', 'Motion blur')} label="Motion blur" /></Row>
  <Row label="Parent"><SelectField {PM} get={() => layer.parent} edit={edit('parent', 'Parent')} options={parentOptions} label="Parent" /></Row>
  <Row label="Color"><ColorField {PM} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
  <Row label="Start"><NumField {PM} get={() => layer.from} edit={edit('from', 'Start')} label="Start" step={0.05} precision={2} unit="s" /></Row>
  <Row label="Duration"><NumField {PM} get={() => layer.dur} edit={edit('duration', 'Duration')} label="Duration" step={0.05} precision={2} unit="s" /></Row>
{/if}
