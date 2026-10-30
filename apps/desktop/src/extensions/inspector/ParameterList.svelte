<script lang="ts">
  import type { EffectParamDefinition, ParameterWidget } from 'powermove';
  import { inspectorContext, type EditBinding } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';
  import PropertyStopwatch from './PropertyStopwatch.svelte';
  import { parameterLayout } from './parameter-layout';
  import { inspectorRefresh } from './refresh.svelte';
  import { inEffectControls } from './effect-controls';
  import type { Snippet } from 'svelte';
  const {api,doc,transport,mixed,edit:inspectorEdit}=inspectorContext();
  const {ColorField,ToggleField,SelectField,Row,RampField,PointField,Disclosure}=api.ui.controls;
  let {layer,params,properties,prefix,ui,extra,inputs={}}:{layer:any;params:EffectParamDefinition[];properties:Record<string,any>;prefix:string;ui?:ParameterWidget[];extra?:Snippet;inputs?:Record<string,any>}=$props();
  const layout=$derived((doc.tick.structure,doc.tick.history,inspectorRefresh.version,parameterLayout(params.filter(p=>['layer','path','asset'].includes(p.type??'')||properties[p.k]!=null),ui)));
  const currentInputs=()=> (doc.tick.values,doc.tick.structure,doc.tick.history,doc.tick.assets,inspectorRefresh.version,inputLayers().find((l:any)=>l.id===layer.id)?.fx?.find((f:any)=>f.id===prefix)?.inputs??inputs);
  const inputValue=(key:string)=>currentInputs()[key]??{layerId:null,stage:'source'};
  const inputBinding=(key:string,field:string):EditBinding=>({mode:'command',label:param(key).label,origin:'inspector',command:(next)=>({type:'set_effect',target:layer.id,effect:prefix,patch:{inputs:{[key]:{...inputValue(key),[field]:next}}}})});
  const inputLayers=()=> (api.model.curComp?.()??doc.proj).layers;
  const pathOptions=(key:string)=>[{v:null,label:'None'},...(currentInputs()[key]?.layerId&&!inputLayers().some((l:any)=>l.id===currentInputs()[key].layerId&&[...(l.masks??[]),...(l.d?.paths??[])].some((p:any)=>p.id===currentInputs()[key].pathId))?[{v:JSON.stringify([currentInputs()[key].layerId,currentInputs()[key].pathId]),label:'Missing path'}]:[]),...inputLayers().flatMap((l:any)=>[...(l.masks??[]).map((m:any)=>({v:JSON.stringify([l.id,m.id]),label:`${l.name} · ${m.name??'Mask'}`})),...(l.d?.paths??[]).map((p:any)=>({v:JSON.stringify([l.id,p.id]),label:`${l.name} · ${p.name??'Path'}`}))])];
  const referenceEdit=(key:string,kind:'path'|'asset'):EditBinding=>({mode:'command',label:param(key).label,origin:'inspector',command:(value)=>({type:'set_effect',target:layer.id,effect:prefix,patch:{inputs:{[key]:kind==='asset'?{kind,assetId:value==null?null:String(value)}:{kind,layerId:value?JSON.parse(String(value))[0]:null,pathId:value?JSON.parse(String(value))[1]:null}}}})});
  const assetOptions=(key:string)=>{const all=new Map<string,any>(),pending=[doc.proj],seen=new Set<any>();while(pending.length){const comp=pending.pop();if(!comp||seen.has(comp))continue;seen.add(comp);for(const asset of Object.values<any>(comp.assets??{}))all.set(asset.id,asset);pending.push(...Object.values<any>(comp.comps??{}));}const id=currentInputs()[key]?.assetId,selected=all.get(id),kind=(param(key) as any).assetKind??'image';return[{v:null,label:'None'},...(id&&selected?.kind!==kind?[{v:id,label:selected?'Unsupported asset':'Missing asset'}]:[]),...[...all.values()].filter(a=>a.kind===kind).map(a=>({v:a.id,label:a.name??a.id})),{v:'__import_asset__',label:kind==='lut'?'Choose LUT…':'Import Image…'}];};
  let choosingAsset=false;
  function chooseAsset(key:string,next:unknown){if(next!=='__import_asset__')return;void importAsset(key);return false;}
  async function importAsset(key:string){if(choosingAsset)return;choosingAsset=true;const project=doc.proj,target=layer.id,effect=prefix,kind=(param(key) as any).assetKind??'image';
    try{const [file]=await api.assets.pick({accept:kind==='lut'?'.cube':'image/*'});if(!file||doc.proj!==project)return;if(api.media.assets.kind(file)!==kind)throw new Error(`Choose ${kind==='lut'?'a .cube LUT':'an image'}`);const asset=await api.assets.import(file);if(asset.kind!==kind)throw new Error(`Choose ${kind==='lut'?'a .cube LUT':'an image'}`);if(doc.proj!==project||!inputLayers().some((l:any)=>l.id===target&&l.fx?.some((f:any)=>f.id===effect)))return;const result=inspectorEdit.apply({type:'set_effect',target,effect,patch:{inputs:{[key]:{kind:'asset',assetId:asset.id}}}},{label:'Choose '+param(key).label,origin:'inspector'});if(!result.ok)throw new Error(result.message);api.transport.invalidate();}
    catch(error){api.ui.toast(error instanceof Error?error.message:'Could not import asset',{kind:'alert',error:true});}finally{choosingAsset=false;}}
  const inputOptions=(key:string)=>[{v:null,label:'None'},...(inputValue(key).layerId&&!inputLayers().some((l:any)=>l.id===inputValue(key).layerId)?[{v:inputValue(key).layerId,label:'Missing layer'}]:[]),...inputLayers().filter((l:any)=>!['audio','null','adjustment'].includes(l.type)&&!l.threeD).map((l:any)=>({v:l.id,label:l.name}))];
  const effectControls=inEffectControls();
  const path=(key:string)=>`${prefix}.${key}`;
  const param=(key:string)=>params.find(p=>p.k===key)!;
  const defaultValue=(p:EffectParamDefinition)=>{
    if(!p.defaultFrom)return p.def;
    const bounds=api.render?.gl?.bounds?.(layer,transport.time),comp=api.model.curComp?.()??doc.proj;
    const positive=(...values:any[])=>values.map(v=>Number(v&&typeof v==='object'?v.v:v)).find(v=>Number.isFinite(v)&&v>0)??1;
    return api.effects.parameterDefault?.(p,{width:positive(bounds?bounds.x1-bounds.x0:undefined,layer.d?.w,comp.w),height:positive(bounds?bounds.y1-bounds.y0:undefined,layer.d?.h,comp.h)})??p.def;
  };
  const value=(key:string)=>(doc.tick.history,doc.tick.values,doc.proj,transport.time,inspectorRefresh.version,api.anim.evP(layer,properties[key],transport.time,prefix==='u'?path(key):key));
  const binding=(key:string):EditBinding=>({mode:'command',label:param(key).label,origin:'inspector',command:(next)=>({type:'set_property',target:layer.id,path:path(key),value:next as any,time:transport.time,preserveHandEdits:false})});
</script>

{#snippet field(parameter:EffectParamDefinition, precise=false)}
  {@const property=properties[parameter.k]}
  {#if parameter.type==='asset'}
    <Row {api} label={parameter.label}><SelectField {api} label={parameter.label} get={()=>currentInputs()[parameter.k]?.assetId??null} edit={referenceEdit(parameter.k,'asset')} options={assetOptions(parameter.k)} onChange={(next:unknown)=>chooseAsset(parameter.k,next)} /></Row>
  {:else if parameter.type==='path'}
    <Row {api} label={parameter.label}><SelectField {api} label={parameter.label} get={()=>currentInputs()[parameter.k]?.layerId?JSON.stringify([currentInputs()[parameter.k].layerId,currentInputs()[parameter.k].pathId]):null} edit={referenceEdit(parameter.k,'path')} options={pathOptions(parameter.k)} /></Row>
  {:else if parameter.type==='layer'}
    <Row {api} label={parameter.label}><SelectField {api} label={parameter.label} get={()=>inputValue(parameter.k).layerId} edit={inputBinding(parameter.k,'layerId')} options={inputOptions(parameter.k)} /></Row>
    <Row {api} label="Input Stage"><SelectField {api} label={`${parameter.label} stage`} get={()=>inputValue(parameter.k).stage} edit={inputBinding(parameter.k,'stage')} options={[{v:'source',label:'Source'},{v:'masks',label:'Masks'},{v:'effects',label:'Effects'}]} /></Row>
    <Row {api} label="Input Space"><SelectField {api} label={`${parameter.label} space`} get={()=>inputValue(parameter.k).space??'composition'} edit={inputBinding(parameter.k,'space')} options={[{v:'composition',label:'Composition'},{v:'source',label:'Layer Source'}]} /></Row>
  {:else if property}
    {#if parameter.options?.length}
      <AnimatedRow {layer} path={path(parameter.k)} label={parameter.label}><SelectField {api} {mixed} label={parameter.label} get={()=>value(parameter.k)} edit={binding(parameter.k)} options={parameter.options} /></AnimatedRow>
    {:else if parameter.type==='color'}
      <AnimatedRow {layer} path={path(parameter.k)} label={parameter.label}><ColorField {api} {mixed} label={parameter.label} get={()=>value(parameter.k)} edit={binding(parameter.k)} /></AnimatedRow>
    {:else if parameter.type==='toggle'}
      <AnimatedRow {layer} path={path(parameter.k)} label={parameter.label}><ToggleField {api} {mixed} label={parameter.label} get={()=>value(parameter.k)} edit={binding(parameter.k)} variant={effectControls?'checkbox':'segmented'} /></AnimatedRow>
    {:else}
      <ChannelRow {layer} channel={path(parameter.k)} label={parameter.label} {property} defaultValue={typeof parameter.def==='number'?Number(defaultValue(parameter)):undefined} getValue={()=>value(parameter.k)} step={parameter.step} min={parameter.min} max={parameter.max} unit={parameter.unit} visual={!precise&&!effectControls} />
    {/if}
  {/if}
{/snippet}

{#snippet widget(w:ParameterWidget)}
  {#if w.kind==='ramp'}
    <RampField {api} label="Gradient" get={()=>[{id:w.start,color:String(value(w.start)),position:0},{id:w.end,color:String(value(w.end)),position:100}]} midpoint={w.midpoint?()=>Number(value(w.midpoint!)):undefined} midpointEdit={w.midpoint?binding(w.midpoint):undefined} onSelect={(key:string)=>document.querySelector<HTMLElement>(`[data-parameter-prefix="${CSS.escape(prefix)}"] [data-color-key="${CSS.escape(key)}"] button.color-field`)?.click()} />
    {#if w.mode}
      <AnimatedRow {layer} path={path(w.mode)} label="Type"><SelectField {api} {mixed} label="Gradient type" get={()=>value(w.mode!)} edit={binding(w.mode)} options={[{v:false,label:'Linear'},{v:true,label:'Radial'}]} /></AnimatedRow>
    {/if}
  {:else if w.kind==='point'}
    {@const x=param(w.x) as any}{@const y=param(w.y) as any}
    <Row {api} label={w.label??'Position'}>
      <PointField {api} label={w.label??'Position'} get={()=>[Number(value(w.x)),Number(value(w.y))]} xEdit={binding(w.x)} yEdit={binding(w.y)} xLabel={x.label} yLabel={y.label} minX={x.min} maxX={x.max} minY={y.min} maxY={y.max} stepX={x.step} stepY={y.step} unit={x.unit}
        edit={{mode:'command',label:`Move ${w.label??'position'}`,origin:'inspector',command:(next:any)=>[binding(w.x),binding(w.y)].map((b:any,index)=>b.command(next[index]))}}>
        {#snippet xAction()}<PropertyStopwatch {layer} path={path(w.x)} label={x.label} />{/snippet}
        {#snippet yAction()}<PropertyStopwatch {layer} path={path(w.y)} label={y.label} />{/snippet}
        {#snippet xField(increment:number)}<ChannelRow {layer} channel={path(w.x)} property={properties[w.x]} defaultValue={Number(defaultValue(x))} label={x.label} getValue={()=>value(w.x)} min={x.min} max={x.max} step={increment} unit={x.unit} visual={false} compact prefix="X" />{/snippet}
        {#snippet yField(increment:number)}<ChannelRow {layer} channel={path(w.y)} property={properties[w.y]} defaultValue={Number(defaultValue(y))} label={y.label} getValue={()=>value(w.y)} min={y.min} max={y.max} step={increment} unit={y.unit} visual={false} compact prefix="Y" />{/snippet}
      </PointField>
    </Row>
  {/if}
{/snippet}

<div data-parameter-prefix={prefix} class="parameter-list">
  {#each layout.widgets.filter(w=>w.kind==='ramp'||!w.advanced) as w}{@render widget(w)}{/each}
  {#each layout.primary as parameter (parameter.k)}
    {#if parameter.group}
      {#if layout.primary.find(p => p.group === parameter.group)?.k === parameter.k}
        {@const grouped = layout.primary.filter(p => p.group === parameter.group)}
        <Disclosure label={parameter.group} count={grouped.length} remember={`${doc.proj.id}:${layer.id}:${prefix}:${parameter.group}`}>
          {#each grouped as member (member.k)}<div data-color-key={member.k}>{@render field(member)}</div>{/each}
        </Disclosure>
      {/if}
    {:else}
      <div data-color-key={parameter.k}>{@render field(parameter)}</div>
    {/if}
  {/each}
  <!-- Secondary settings follow the main ones directly, never folded away. -->
  {#each layout.widgets.filter(w=>w.kind==='point'&&w.advanced) as w}{@render widget(w)}{/each}
  {#each layout.more as parameter (parameter.k)}{@render field(parameter,true)}{/each}
  {#if extra}{@render extra()}{/if}
</div>
