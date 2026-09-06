<script lang="ts">
  import { MATTE_MODES, validMatteSource } from 'powermove';
  import AnimatedRow from './AnimatedRow.svelte';
  import { isProperty } from 'powermove';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';

  const { api, doc, transport } = inspectorContext();
  const { ColorField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const { layerFieldBinding } = api.ui.controls.binding;

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  const value = (field: string) => (doc.tick.values, transport.time, isProperty(layer[field]) ? PM.evP(layer, layer[field], transport.time, `l.${field}`) : layer[field]);
  const animatedEdit = (field: string, label: string): EditBinding => ({ mode: 'command', label, origin: 'inspector', command: (value) => ({ type: 'set_property', target: layer.id, path: `l.${field}`, value: value as any, time: transport.time, mode: 'auto', preserveHandEdits: false }) });
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
<Row label="Solo"><ToggleField {PM} get={() => layer.solo ?? false} edit={edit('solo','Solo layer')} label="Solo layer" /></Row>
<Row label="Shy"><ToggleField {PM} get={() => layer.shy} edit={edit('shy','Shy layer')} label="Shy layer" /></Row>

<Row label="Visible"><ToggleField {PM} get={() => value('on')} edit={edit('visible', 'Visibility')} label="Visibility" /></Row>

{#if layer.type === 'audio'}
  <Row label="Color"><ColorField {PM} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{:else}
  <AnimatedRow {PM} {layer} path="l.blend" label="Blend mode"><SelectField {PM} get={() => value('blend')} edit={animatedEdit('blend', 'Blend')} options={PM.BLENDS ?? []} label="Blend" /></AnimatedRow>
  <AnimatedRow {PM} {layer} path="l.mblur" label="Motion blur"><ToggleField {PM} get={() => value('mblur')} edit={animatedEdit('mblur', 'Motion blur')} label="Motion blur" /></AnimatedRow>
  <Row label="Track matte"><SelectField {PM} get={()=>layer.matteSource??null} edit={edit('matteSource','Track matte')} options={[{v:null,label:'None'},...PM.proj.layers.filter((l:any)=>validMatteSource(PM.proj.layers,layer,l.id)).map((l:any)=>({v:l.id,label:l.name}))]} label="Track matte source" /></Row>
  {#if layer.matteSource}<AnimatedRow {PM} {layer} path="l.matteMode" label="Matte mode"><SelectField {PM} get={()=>value('matteMode')} edit={animatedEdit('matteMode','Matte mode')} options={MATTE_MODES} label="Matte mode" /></AnimatedRow>{/if}
  <Row label="Parent"><SelectField {PM} get={() => layer.parent} edit={edit('parent', 'Parent')} options={parentOptions} label="Parent" /></Row>
  <Row label="Color"><ColorField {PM} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{/if}
