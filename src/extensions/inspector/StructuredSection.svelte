<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';
  import AnimatedRow from './AnimatedRow.svelte';
  import { makeVectorPath, structuredProperties } from 'powermove';
  import { pathTargets } from 'powermove';
  import { inspectorRefresh } from './refresh.svelte';
  const {api,doc,transport}=inspectorContext();
  const {Section,Row,SelectField,ColorField,ToggleField}=api.ui.controls;
  let {PM,layer}:{PM:any;layer:any}=$props();
  const targets=$derived((doc.tick.structure,doc.tick.values,doc.proj,inspectorRefresh.version,pathTargets(layer)));
  const textItems=$derived((doc.tick.structure,doc.tick.values,doc.proj,inspectorRefresh.version,[...(layer.d.animators||[]),...(layer.d.styles||[])]));
  const records=$derived((doc.tick.structure,doc.tick.values,doc.proj,inspectorRefresh.version,structuredProperties(layer)));
  const bind=(path:string)=>({mode:'command' as const,label:'Edit '+path,origin:'inspector' as const,command:(value:any)=>({type:'set_property' as const,target:layer.id,path,value,time:transport.time,preserveHandEdits:false})});
  function mutate(label:string,fn:()=>void){PM.Edit.mutate(label,fn,{origin:'inspector'});PM.invalidate();inspectorRefresh.bump();}
  function addPath(){mutate('Add path',()=>{const path=makeVectorPath(PM);(layer.d.paths||=[]).push(path);PM.activePath=path.id;});PM.setTool('pen');}
  function addAnimator(){mutate('Add text animator',()=>{const values={unit:'characters',start:0,end:100,offset:0,smoothness:0,x:0,y:0,rotation:0,scale:100,opacity:100,tracking:0};(layer.d.animators||=[]).push({id:PM.uid('ta'),name:'Animator '+((layer.d.animators?.length||0)+1),p:Object.fromEntries(Object.entries(values).map(([k,v])=>[k,PM.P(v)]))});});}
  function range(){const selection=PM.textSelection?.layer===layer.id?PM.textSelection:null;if(!selection || selection.end<=selection.start){PM.toast('Select text on the canvas first');return;}mutate('Style text range',()=>{(layer.d.styles||=[]).push({id:PM.uid('ts'),start:selection.start,end:selection.end,p:{color:PM.P('#ffffff'),weight:PM.P(700),size:PM.P(Number(PM.evP(layer,layer.d.size,PM.time,'c.size'))||64)}});});}
  const label=(key:string)=>key.replace(/([A-Z])/g,' $1').replace(/^./,x=>x.toUpperCase());
</script>
{#if layer.type==='shape' || targets.length}
  <Section title="Paths" />
  {#if layer.type==='shape'}<button class="chip" onclick={addPath}>Add path</button>{/if}
  <p class="hint">{layer.type==='shape'?'Draws shape paths':'Draws layer masks'} · G: Pen · drag to curve · click the first vertex to close · ⌘-click to delete</p>
  {#each targets as target (target.path.id)}
    <details open><summary>{target.path.name}</summary>
      <div class="buttons"><button class="chip" onclick={()=>{PM.activePath=target.path.id;PM.setTool('pen');}}>Edit vertices</button><button class="chip" onclick={()=>mutate('Remove path',()=>{if(target.prefix.startsWith('g.')){layer.d.paths=layer.d.paths.filter((p:any)=>p.id!==target.path.id);for(const p of layer.d.paths)if(p.parent===target.path.id)p.parent=null;}else{const m=layer.masks.find((m:any)=>m.path===target.path);if(m)layer.masks=layer.masks.filter((x:any)=>x!==m);}})}>Remove</button></div>
      {#if target.prefix.startsWith('g.')}
      <Row label="Group"><SelectField {PM} label="Path parent group" get={()=>target.path.parent} options={[{v:null,label:'None'},...targets.filter((t:any)=>t!==target&&!t.path.parent&&t.prefix.startsWith('g.')).map((t:any)=>({v:t.path.id,label:t.path.name}))]} edit={{mode:'set',label:'Group path',set:(v:any)=>{target.path.parent=v;PM.touch();}}} /></Row>
      {/if}
      {#each records.filter((r:any)=>r.key.startsWith(target.prefix+'.')&&!r.key.includes('.v.')) as record (record.key)}
        {@const value=PM.evP(layer,record.prop,transport.time,record.key)}
        {#if typeof value==='number'}<ChannelRow {PM} {layer} channel={record.key} property={record.prop} label={label(record.label)} step={1} />
        {:else}<AnimatedRow {PM} {layer} path={record.key} label={label(record.label)}>
          {#if typeof value==='boolean'}<ToggleField {PM} label={label(record.label)} get={()=>value} edit={bind(record.key)} />{:else}<ColorField {PM} label={label(record.label)} get={()=>value} edit={bind(record.key)} />{/if}
        </AnimatedRow>{/if}
      {/each}
    </details>
  {/each}
{/if}
{#if layer.type==='text'}
  <Section title="Text animation & ranges" />
  <div class="buttons"><button class="chip" onclick={addAnimator}>Add animator</button><button class="chip" onclick={range}>Style selection</button></div>
  {#each textItems as item (item.id)}
    {@const prefix=(item.start===undefined?'ta.':'ts.')+item.id}
    <details open><summary>{item.name||`Characters ${item.start+1}–${item.end}`}</summary>
      <button class="chip" onclick={()=>mutate('Remove text control',()=>{layer.d.animators=layer.d.animators?.filter((x:any)=>x.id!==item.id);layer.d.styles=layer.d.styles?.filter((x:any)=>x.id!==item.id);})}>Remove</button>
      {#each records.filter((r:any)=>r.key.startsWith(prefix+'.')) as record (record.key)}
        {@const value=PM.evP(layer,record.prop,transport.time,record.key)}
        {#if typeof value==='number'}<ChannelRow {PM} {layer} channel={record.key} property={record.prop} label={label(record.label)} step={1} />
        {:else}<AnimatedRow {PM} {layer} path={record.key} label={label(record.label)}>{#if record.label==='unit'}<SelectField {PM} label="Based on" get={()=>value} edit={bind(record.key)} options={['characters','words','lines']} />{:else}<ColorField {PM} label="Range color" get={()=>value} edit={bind(record.key)} />{/if}</AnimatedRow>{/if}
      {/each}
    </details>
  {/each}
{/if}
<style>.hint{font-size:11px;color:var(--tx-3)}.buttons{display:flex;gap:6px;flex-wrap:wrap}details{margin:6px 0}summary{cursor:pointer;padding:6px 0;font-weight:500}</style>
