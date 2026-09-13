<script lang="ts">
  import { MATTE_MODES, validMatteSource } from 'powermove';
  import AnimatedRow from './AnimatedRow.svelte';
  import { isProperty } from 'powermove';
  import { inspectorContext, type EditBinding, type SelectOption } from './context';

  const { api, doc, transport, sel, mixed } = inspectorContext();
  const { ColorField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const { layerFieldBinding } = api.ui.controls.binding;

  let { layer }: { layer: any } = $props();

  const value = (field: string) => (doc.tick.values, transport.time, isProperty(layer[field]) ? api.anim.evP(layer, layer[field], transport.time, `l.${field}`) : layer[field]);
  const animatedEdit = (field: string, label: string): EditBinding => ({ mode: 'command', label, origin: 'inspector', command: (value) => ({ type: 'set_property', target: layer.id, path: `l.${field}`, value: value as any, time: transport.time, mode: 'auto', preserveHandEdits: false }) });
  const edit = (field: string, label: string): EditBinding =>
    layerFieldBinding(layer.id, field as any, { label, origin: 'inspector' });
  const selectedLayers = () => api.selection.layers().map((id) => api.model.layer(id)).filter((candidate) => candidate !== null);
  const parentingLayers = () => (api.selection.layers().includes(layer.id) ? selectedLayers() : [layer])
    .filter((item: any) => (api.model.TYPE_META as Record<string, { transform?: boolean }>)[item.type]?.transform !== false);
  const parentEdit: EditBinding = { mode:'command', label:'Parent layers', origin:'inspector', command: (parent) => parentingLayers().map((item: any) => ({ type:'set_layer', target:item.id, patch:{parent: parent == null ? null : String(parent)} })) };
  const parentOptions = $derived<SelectOption[]>((doc.tick.structure, doc.proj, sel.layers, [
    { v: null, label: 'None' },
    ...api.project.get().layers.filter((candidate: any) => (api.model.TYPE_META as Record<string, { transform?: boolean }>)[candidate.type]?.transform !== false && parentingLayers().every((item: any) => item.id !== candidate.id && !api.anim.wouldCycle(item, candidate.id)))
      .map((candidate: any) => ({v:candidate.id,label:String(candidate.name)}))
  ]));
</script>

<Section {api} title="Layer" />
<Row {api} label="Solo"><ToggleField {api} {mixed} get={() => layer.solo ?? false} edit={edit('solo','Solo layer')} label="Solo layer" /></Row>
<Row {api} label="Shy"><ToggleField {api} {mixed} get={() => layer.shy} edit={edit('shy','Shy layer')} label="Shy layer" /></Row>

<Row {api} label="Visible"><ToggleField {api} {mixed} get={() => value('on')} edit={edit('visible', 'Visibility')} label="Visibility" /></Row>

{#if layer.type === 'audio'}
  <Row {api} label="Color"><ColorField {api} {mixed} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{:else}
  <AnimatedRow {layer} path="l.blend" label="Blend mode"><SelectField {api} {mixed} get={() => value('blend')} edit={animatedEdit('blend', 'Blend')} options={api.model.BLENDS ?? []} label="Blend" /></AnimatedRow>
  <AnimatedRow {layer} path="l.mblur" label="Motion blur"><ToggleField {api} {mixed} get={() => value('mblur')} edit={animatedEdit('mblur', 'Motion blur')} label="Motion blur" /></AnimatedRow>
  <Row {api} label="Track matte"><SelectField {api} {mixed} get={()=>layer.matteSource??null} edit={edit('matteSource','Track matte')} options={[{v:null,label:'None'},...api.project.get().layers.filter((l:any)=>validMatteSource(api.project.get().layers,layer,l.id)).map((l:any)=>({v:l.id,label:l.name}))]} label="Track matte source" /></Row>
  {#if layer.matteSource}<AnimatedRow {layer} path="l.matteMode" label="Matte mode"><SelectField {api} {mixed} get={()=>value('matteMode')} edit={animatedEdit('matteMode','Matte mode')} options={MATTE_MODES} label="Matte mode" /></AnimatedRow>{/if}
  <Row {api} label="Parent"><button class="pickwhip" aria-label="Pick parent layer" title="Drag to a layer to parent · Esc cancels" onpointerdown={(event) => api.ui.beginParentPick(event, api.selection.layers().includes(layer.id) ? api.selection.layers() : [layer.id])}>◎</button><SelectField {api} {mixed} get={() => layer.parent} edit={parentEdit} options={parentOptions} label="Parent" /></Row>
  <Row {api} label="Color"><ColorField {api} {mixed} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{/if}

<style>
  .pickwhip { flex: 0 0 24px; height:24px; padding:0; border:0; background:transparent; color:var(--tx-2); cursor:crosshair; font-size:18px; }
  .pickwhip:hover { color:var(--accent); }
</style>
