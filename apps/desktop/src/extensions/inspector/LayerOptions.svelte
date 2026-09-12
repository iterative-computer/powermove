<script lang="ts">
  import { MATTE_MODES, validMatteSource } from 'powermove';
  import AnimatedRow from './AnimatedRow.svelte';
  import { isProperty } from 'powermove';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';

  const { api, doc, transport, sel } = inspectorContext();
  const { ColorField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const { layerFieldBinding } = api.ui.controls.binding;

  let { PM, layer }: { PM: Record<string, any>; layer: any } = $props();

  const value = (field: string) => (doc.tick.values, transport.time, isProperty(layer[field]) ? PM.evP(layer, layer[field], transport.time, `l.${field}`) : layer[field]);
  const animatedEdit = (field: string, label: string): EditBinding => ({ mode: 'command', label, origin: 'inspector', command: (value) => ({ type: 'set_property', target: layer.id, path: `l.${field}`, value: value as any, time: transport.time, mode: 'auto', preserveHandEdits: false }) });
  const edit = (field: string, label: string): EditBinding =>
    layerFieldBinding(layer.id, field as any, { label, origin: 'inspector' });
  const parentingLayers = () => (PM.sel.layers.includes(layer.id) ? PM.selLayers() : [layer])
    .filter((item: any) => item.type !== 'group' && PM.TYPE_META[item.type]?.transform !== false);
  const parentEdit: EditBinding = { mode:'command', label:'Parent layers', origin:'inspector', command: (parent) => parentingLayers().map((item: any) => ({ type:'set_layer', target:item.id, patch:{parent} })) };
  const parentOptions = $derived<SelectOption[]>((doc.tick.structure, doc.proj, sel.layers, [
    { v: null, label: 'None' },
    ...(PM.proj?.layers || []).filter((candidate: any) => candidate.type !== 'group' && PM.TYPE_META[candidate.type]?.transform !== false && parentingLayers().every((item: any) => item.id !== candidate.id && !PM.wouldCycle(item, candidate.id)))
      .map((candidate: any) => ({v:candidate.id,label:String(candidate.name)}))
  ]));
</script>

<Section title="Layer" />
<Row label="Solo"><ToggleField {PM} get={() => layer.solo ?? false} edit={edit('solo','Solo layer')} label="Solo layer" /></Row>
<Row label="Shy"><ToggleField {PM} get={() => layer.shy} edit={edit('shy','Shy layer')} label="Shy layer" /></Row>

<Row label="Visible"><ToggleField {PM} get={() => value('on')} edit={edit('visible', 'Visibility')} label="Visibility" /></Row>

{#if layer.type === 'audio' || layer.type === 'group'}
  <Row label="Color"><ColorField {PM} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{:else}
  <AnimatedRow {PM} {layer} path="l.blend" label="Blend mode"><SelectField {PM} get={() => value('blend')} edit={animatedEdit('blend', 'Blend')} options={PM.BLENDS ?? []} label="Blend" /></AnimatedRow>
  <AnimatedRow {PM} {layer} path="l.mblur" label="Motion blur"><ToggleField {PM} get={() => value('mblur')} edit={animatedEdit('mblur', 'Motion blur')} label="Motion blur" /></AnimatedRow>
  <Row label="Track matte"><SelectField {PM} get={()=>layer.matteSource??null} edit={edit('matteSource','Track matte')} options={[{v:null,label:'None'},...PM.proj.layers.filter((l:any)=>validMatteSource(PM.proj.layers,layer,l.id)).map((l:any)=>({v:l.id,label:l.name}))]} label="Track matte source" /></Row>
  {#if layer.matteSource}<AnimatedRow {PM} {layer} path="l.matteMode" label="Matte mode"><SelectField {PM} get={()=>value('matteMode')} edit={animatedEdit('matteMode','Matte mode')} options={MATTE_MODES} label="Matte mode" /></AnimatedRow>{/if}
  <Row label="Parent"><button class="pickwhip" aria-label="Pick parent layer" title="Drag to a layer to parent · Esc cancels" onpointerdown={(event) => PM.beginParentPick(event, PM.sel.layers.includes(layer.id) ? PM.sel.layers : [layer.id])}>◎</button><SelectField {PM} get={() => layer.parent} edit={parentEdit} options={parentOptions} label="Parent" /></Row>
  <Row label="Color"><ColorField {PM} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{/if}

<style>
  .pickwhip { flex: 0 0 24px; height:24px; padding:0; border:0; background:transparent; color:var(--tx-2); cursor:crosshair; font-size:18px; }
  .pickwhip:hover { color:var(--accent); }
</style>
