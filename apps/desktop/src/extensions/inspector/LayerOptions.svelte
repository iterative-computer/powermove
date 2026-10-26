<script lang="ts">
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
  const parentOptions = $derived.by<SelectOption[]>(() => {
    doc.tick.structure; doc.proj; sel.layers;
    // Resolve this fixed selection once, rather than once per possible parent.
    const children = parentingLayers();
    return [
      { v: null, label: 'None' },
      ...api.project.get().layers.filter((candidate: any) => (api.model.TYPE_META as Record<string, { transform?: boolean }>)[candidate.type]?.transform !== false && children.every((item: any) => item.id !== candidate.id && !api.anim.wouldCycle(item, candidate.id)))
        .map((candidate: any) => ({ v: candidate.id, label: String(candidate.name) }))
    ];
  });
</script>

<Section {api} title="Layer" />
<Row {api} label="Solo"><ToggleField {api} {mixed} get={() => layer.solo ?? false} edit={edit('solo','Solo layer')} label="Solo layer" /></Row>

{#if layer.fx?.length}
  <Row {api} label="Effect Space"><SelectField {api} label="Effect Space" get={()=>(doc.tick.values,layer.effectSpace??'composition')} edit={{mode:'command',label:'Effect Space',origin:'inspector',command:(value:unknown)=>({type:'set_layer',target:layer.id,patch:{effectSpace:value as 'composition'|'source'}})}} options={[{v:'composition',label:'Composition'},{v:'source',label:'Layer Source'}]} /></Row>
{/if}
<Row {api} label="Visible"><ToggleField {api} {mixed} get={() => value('on')} edit={edit('visible', 'Visibility')} label="Visibility" /></Row>

{#if layer.type === 'audio'}
  <Row {api} label="Color"><ColorField {api} {mixed} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{:else}
  <AnimatedRow {layer} path="l.mblur" label="Motion blur"><ToggleField {api} {mixed} get={() => value('mblur')} edit={animatedEdit('mblur', 'Motion blur')} label="Motion blur" /></AnimatedRow>
  {#if layer.type === 'precomp'}
    <Row {api} label="Rasterize"><ToggleField {api} {mixed} get={() => (doc.tick.values, !!layer.continuousRaster)} edit={edit('continuousRaster', 'Continuously rasterize')} label="Continuously rasterize" /></Row>
  {/if}
  <Row {api} label="Link Layers"><button class="pickwhip" aria-label="Pick parent layer" title="Drag to a layer to parent · Esc cancels" onpointerdown={(event) => api.ui.beginParentPick(event, api.selection.layers().includes(layer.id) ? api.selection.layers() : [layer.id])}>◎</button><SelectField {api} {mixed} get={() => layer.parent} edit={parentEdit} options={parentOptions} label="Parent" /></Row>
  <Row {api} label="Color"><ColorField {api} {mixed} get={() => layer.color} edit={edit('color', 'Label color')} label="Label color" /></Row>
{/if}

<style>
  .pickwhip { flex: 0 0 24px; height:24px; padding:0; border:0; background:transparent; color:var(--tx-2); cursor:crosshair; font-size:18px; }
  .pickwhip:hover { color:var(--accent); }
</style>
