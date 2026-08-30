<script lang="ts">
  import { inspectorContext, type EditBinding, type SelectOption } from './context';

  const { api, doc } = inspectorContext();
  const { ColorField, NumField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const { layerFieldBinding } = api.ui.controls.binding;

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  const edit = (field: string, label: string): EditBinding =>
    layerFieldBinding(PM, layer.id, field as any, { label, origin: 'inspector' });
  const parentOptions = $derived<SelectOption[]>((doc.tick.structure, doc.proj, [
    { v: null, label: 'none' },
    ...(PM.ProjectIndex?.parentOptions?.(layer) ?? (PM.proj?.layers ?? [])
      .filter((candidate: any) => candidate.id !== layer.id && !PM.wouldCycle?.(layer, candidate.id))
      .map((candidate: any) => ({ v: candidate.id, label: String(candidate.name) })))
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
