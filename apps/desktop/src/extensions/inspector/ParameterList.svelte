<script lang="ts">
  import type { EffectParamDefinition, ParameterWidget } from 'powermove';
  import { inspectorContext, type EditBinding } from './context';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';
  import PropertyStopwatch from './PropertyStopwatch.svelte';
  import { parameterLayout } from './parameter-layout';
  import { inspectorRefresh } from './refresh.svelte';
  import type { Snippet } from 'svelte';
  const {api,doc,transport,mixed}=inspectorContext();
  const {ColorField,ToggleField,SelectField,Row,RampField,PointField,Disclosure}=api.ui.controls;
  let {layer,params,properties,prefix,ui,extra}:{layer:any;params:EffectParamDefinition[];properties:Record<string,any>;prefix:string;ui?:ParameterWidget[];extra?:Snippet}=$props();
  const layout=$derived(parameterLayout(params,ui));
  const path=(key:string)=>`${prefix}.${key}`;
  const param=(key:string)=>params.find(p=>p.k===key)!;
  const value=(key:string)=>(doc.tick.history,doc.tick.values,doc.proj,transport.time,inspectorRefresh.version,api.anim.evP(layer,properties[key],transport.time,prefix==='u'?path(key):key));
  const binding=(key:string):EditBinding=>({mode:'command',label:param(key).label,origin:'inspector',command:(next)=>({type:'set_property',target:layer.id,path:path(key),value:next as any,time:transport.time,preserveHandEdits:false})});
  const changed=$derived((doc.tick.history,doc.tick.values,doc.proj,inspectorRefresh.version,[...layout.more,...layout.widgets.filter(w=>w.kind==='point'&&w.advanced).flatMap(w=>w.kind==='point'?[param(w.x),param(w.y)]:[])].some(p=>properties[p.k]&&(properties[p.k]?.kf?.length||properties[p.k]?.expr||JSON.stringify(properties[p.k]?.v)!==JSON.stringify(p.def)))));
</script>

{#snippet field(parameter:EffectParamDefinition, precise=false)}
  {@const property=properties[parameter.k]}
  {#if property}
    {#if parameter.options?.length}
      <AnimatedRow {layer} path={path(parameter.k)} label={parameter.label}><SelectField {api} {mixed} label={parameter.label} get={()=>value(parameter.k)} edit={binding(parameter.k)} options={parameter.options} /></AnimatedRow>
    {:else if parameter.type==='color'}
      <AnimatedRow {layer} path={path(parameter.k)} label={parameter.label}><ColorField {api} {mixed} label={parameter.label} get={()=>value(parameter.k)} edit={binding(parameter.k)} /></AnimatedRow>
    {:else if parameter.type==='toggle'}
      <AnimatedRow {layer} path={path(parameter.k)} label={parameter.label}><ToggleField {api} {mixed} label={parameter.label} get={()=>value(parameter.k)} edit={binding(parameter.k)} /></AnimatedRow>
    {:else}
      <ChannelRow {layer} channel={path(parameter.k)} label={parameter.label} {property} defaultValue={typeof parameter.def==='number'?parameter.def:undefined} getValue={()=>value(parameter.k)} step={parameter.step} min={parameter.min} max={parameter.max} unit={parameter.unit} visual={!precise} />
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
      <PointField {api} label={w.label??'Position'} get={()=>[Number(value(w.x)),Number(value(w.y))]} xEdit={binding(w.x)} yEdit={binding(w.y)} xLabel={x.label} yLabel={y.label} minX={x.min} maxX={x.max} minY={y.min} maxY={y.max} step={x.step??1} unit={x.unit}
        edit={{mode:'command',label:`Move ${w.label??'position'}`,origin:'inspector',command:(next:any)=>[binding(w.x),binding(w.y)].map((b:any,index)=>b.command(next[index]))}}>
        {#snippet xAction()}<PropertyStopwatch {layer} path={path(w.x)} label={x.label} />{/snippet}
        {#snippet yAction()}<PropertyStopwatch {layer} path={path(w.y)} label={y.label} />{/snippet}
        {#snippet xField()}<ChannelRow {layer} channel={path(w.x)} property={properties[w.x]} defaultValue={x.def} label={x.label} getValue={()=>value(w.x)} min={x.min} max={x.max} step={x.step} unit={x.unit} visual={false} compact prefix="X" />{/snippet}
        {#snippet yField()}<ChannelRow {layer} channel={path(w.y)} property={properties[w.y]} defaultValue={y.def} label={y.label} getValue={()=>value(w.y)} min={y.min} max={y.max} step={y.step} unit={y.unit} visual={false} compact prefix="Y" />{/snippet}
      </PointField>
    </Row>
  {/if}
{/snippet}

<div data-parameter-prefix={prefix} class="parameter-list">
  {#each layout.widgets.filter(w=>w.kind==='ramp'||!w.advanced) as w}{@render widget(w)}{/each}
  {#each layout.primary as parameter (parameter.k)}
    <div data-color-key={parameter.k}>{@render field(parameter)}</div>
  {/each}
  {#if layout.more.length||extra||layout.widgets.some(w=>w.kind==='point'&&w.advanced)}
    <Disclosure count={layout.more.length+layout.widgets.filter(w=>w.kind==='point'&&w.advanced).length+(extra?1:0)} {changed} remember={`${doc.proj.id}:${layer.id}:${prefix}`}>
      {#each layout.widgets.filter(w=>w.kind==='point'&&w.advanced) as w}{@render widget(w)}{/each}
      {#each layout.more as parameter (parameter.k)}{@render field(parameter,true)}{/each}
      {#if extra}{@render extra()}{/if}
    </Disclosure>
  {/if}
</div>
