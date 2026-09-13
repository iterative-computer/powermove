<script lang="ts">
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';
  import AnimatedRow from './AnimatedRow.svelte';
  import Icon from './Icon.svelte';
  import { pathTargets, structuredProperties } from 'powermove';
  import { inspectorRefresh } from './refresh.svelte';

  const {api,doc,transport,controlProps,edit:inspectorEdit,tools,viewer}=inspectorContext();
  const {Section,Row,SelectField,ColorField}=api.ui.controls;
  let {layer}:{layer:any}=$props();

  const targets=$derived((doc.tick.structure,doc.tick.values,doc.proj,inspectorRefresh.version,pathTargets(layer)));
  const shapeTargets=$derived(targets.filter((target:any)=>target.prefix.startsWith('g.')));
  const maskTargets=$derived(targets.filter((target:any)=>target.prefix.startsWith('mp.')));
  const textItems=$derived((doc.tick.structure,doc.tick.values,doc.proj,inspectorRefresh.version,[...(layer.d.animators||[]),...(layer.d.styles||[])]));
  const records=$derived((doc.tick.structure,doc.tick.values,doc.proj,inspectorRefresh.version,structuredProperties(layer)));
  let activePathId=$state<string|null>(null);
  const activeTarget=$derived(shapeTargets.find((target:any)=>target.path.id===activePathId) ?? shapeTargets[0] ?? null);
  const activeValues=$derived.by(()=>{
    doc.tick.history;doc.tick.values;doc.tick.structure;doc.proj;inspectorRefresh.version;
    if(!activeTarget)return {} as Record<string,any>;
    return Object.fromEntries(Object.entries(activeTarget.path.p||{}).map(([key,property])=>[
      key,api.anim.evP(layer,property as any,transport.time,`${activeTarget.prefix}.${key}`)
    ]));
  });

  const bind=(path:string)=>({mode:'command' as const,label:'Edit '+path,origin:'inspector' as const,command:(value:any)=>({type:'set_property' as const,target:layer.id,path,value,time:transport.time,preserveHandEdits:false})});
  const channel=(key:string)=>`${activeTarget.prefix}.${key}`;
  const property=(key:string)=>activeTarget?.path.p?.[key];
  function mutate(label:string,fn:()=>void){inspectorEdit.mutate(label,fn,{origin:'inspector'});api.transport.invalidate();inspectorRefresh.bump();}
  function applyPathValue(key:string,value:any,label:string){
    if(!activeTarget)return;
    inspectorEdit.apply({type:'set_property',target:layer.id,path:channel(key),value,time:transport.time,preserveHandEdits:false},{label,origin:'inspector'});
    api.transport.invalidate();
  }
  function setActivePath(value:string|null){const service=viewer();if(service)service.activePath=value;}
  function makePath(){const values={x:0,y:0,rotation:0,scaleX:100,scaleY:100,closed:false,fill:'#E8E2CF',fillEnabled:true,fillOpacity:100,stroke:'#ffffff',strokeWidth:2,strokeOpacity:100,trimStart:0,trimEnd:100,trimOffset:0,copies:1,repeatX:30,repeatY:0,repeatRotation:0};return{id:api.util.uid('path'),name:'Path',parent:null,vertices:[],p:Object.fromEntries(Object.entries(values).map(([key,value])=>[key,api.model.P(value)]))};}
  function selectPath(target:any,startEditing=false){activePathId=target.path.id;setActivePath(target.path.id);if(startEditing)tools()?.setTool('pen');}
  function addPath(){mutate('Add path',()=>{const path=makePath();(layer.d.paths||=[]).push(path);activePathId=path.id;setActivePath(path.id);});tools()?.setTool('pen');}
  function removePath(target:any){mutate('Remove path',()=>{layer.d.paths=layer.d.paths.filter((path:any)=>path.id!==target.path.id);for(const path of layer.d.paths)if(path.parent===target.path.id)path.parent=null;activePathId=layer.d.paths[0]?.id??null;setActivePath(activePathId);});}
  function addAnimator(){mutate('Add text animator',()=>{const values={unit:'characters',start:0,end:100,offset:0,smoothness:0,x:0,y:0,rotation:0,scale:100,opacity:100,tracking:0};(layer.d.animators||=[]).push({id:api.util.uid('ta'),name:'Animator '+((layer.d.animators?.length||0)+1),p:Object.fromEntries(Object.entries(values).map(([k,v])=>[k,api.model.P(v)]))});});}
  function range(){const current=viewer()?.textSelection;const selection=current?.layer===layer.id?current:null;if(!selection || selection.end<=selection.start){api.ui.toast('Select text on the canvas first');return;}mutate('Style text range',()=>{(layer.d.styles||=[]).push({id:api.util.uid('ts'),start:selection.start,end:selection.end,p:{color:api.model.P('#ffffff'),weight:api.model.P(700),size:api.model.P(Number(api.anim.evP(layer,layer.d.size,api.transport.time(),'c.size'))||64)}});});}
  const label=(key:string)=>key.replace(/([A-Z])/g,' $1').replace(/^./,x=>x.toUpperCase());
</script>

{#if layer.type==='shape' && activeTarget}
  <div class="section-head"><Section title={shapeTargets.length===1?'Path':'Paths'} /><button class="section-action" onclick={addPath} aria-label="Add path" title="Add path"><Icon name="plus" /></button></div>
  <div class="path-list" class:multiple={shapeTargets.length>1}>
    {#each shapeTargets as target (target.path.id)}
      <div class="path-item" class:active={target.path.id===activeTarget.path.id}>
        <button class="path-edit" onclick={()=>selectPath(target,true)} aria-label={`Edit ${target.path.name} vertices`}>
          <span class="path-icon" aria-hidden="true">⌁</span><span>{target.path.name}</span><span class="path-edit-label">Edit vertices</span>
        </button>
        {#if shapeTargets.length>1}<button class="path-remove" onclick={()=>removePath(target)} aria-label={`Remove ${target.path.name}`} title="Remove path"><span aria-hidden="true">−</span></button>{/if}
      </div>
    {/each}
  </div>

  <details class="advanced">
    <summary>Advanced path options</summary>
    {#if shapeTargets.length>1}
      <Row label="Group"><SelectField {...controlProps} label="Path parent group" get={()=>activeTarget.path.parent} options={[{v:null,label:'None'},...shapeTargets.filter((target:any)=>target!==activeTarget&&!target.path.parent).map((target:any)=>({v:target.path.id,label:target.path.name}))]} edit={{mode:'set',label:'Group path',set:(value:any)=>{activeTarget.path.parent=value;api.transport.invalidate();inspectorRefresh.bump();}}} /></Row>
    {/if}
    <div class="advanced-label">Path transform</div>
    {#each ['x','y','rotation','scaleX','scaleY'] as key (key)}
      <ChannelRow {layer} channel={channel(key)} property={property(key)} label={label(key)} step={1} unit={key==='rotation'?'°':key.startsWith('scale')?'%':undefined} />
    {/each}
    <AnimatedRow {layer} path={channel('closed')} label="Closed">
      <button class="compact-toggle" class:on={!!activeValues.closed} onclick={()=>applyPathValue('closed',!activeValues.closed,'Toggle closed path')} aria-pressed={!!activeValues.closed}>{activeValues.closed?'Closed':'Open'}</button>
    </AnimatedRow>
    <div class="advanced-label">Trim paths</div>
    <ChannelRow {layer} channel={channel('trimStart')} property={property('trimStart')} label="Start" step={1} min={0} max={100} unit="%" />
    <ChannelRow {layer} channel={channel('trimEnd')} property={property('trimEnd')} label="End" step={1} min={0} max={100} unit="%" />
    <ChannelRow {layer} channel={channel('trimOffset')} property={property('trimOffset')} label="Offset" step={1} unit="°" />
    <div class="advanced-label">Repeater</div>
    <ChannelRow {layer} channel={channel('copies')} property={property('copies')} label="Copies" step={1} min={1} max={256} />
    {#if Number(activeValues.copies)>1}
      <ChannelRow {layer} channel={channel('repeatX')} property={property('repeatX')} label="Offset X" step={1} />
      <ChannelRow {layer} channel={channel('repeatY')} property={property('repeatY')} label="Offset Y" step={1} />
      <ChannelRow {layer} channel={channel('repeatRotation')} property={property('repeatRotation')} label="Rotation" step={1} unit="°" />
    {/if}
  </details>

  <div class="section-head"><Section title="Fill" /><button class="section-action" onclick={()=>applyPathValue('fillEnabled',!activeValues.fillEnabled,activeValues.fillEnabled?'Remove fill':'Add fill')} aria-label={activeValues.fillEnabled?'Remove fill':'Add fill'} title={activeValues.fillEnabled?'Remove fill':'Add fill'}><span aria-hidden="true">{activeValues.fillEnabled?'−':'+'}</span></button></div>
  {#if activeValues.fillEnabled}
    <AnimatedRow {layer} path={channel('fill')} label="Color"><ColorField {...controlProps} label="Fill color" get={()=>activeValues.fill} edit={bind(channel('fill'))} /></AnimatedRow>
    <ChannelRow {layer} channel={channel('fillOpacity')} property={property('fillOpacity')} label="Opacity" step={1} min={0} max={100} unit="%" />
  {/if}

  <div class="section-head"><Section title="Stroke" /><button class="section-action" onclick={()=>applyPathValue('strokeWidth',Number(activeValues.strokeWidth)>0?0:1,Number(activeValues.strokeWidth)>0?'Remove stroke':'Add stroke')} aria-label={Number(activeValues.strokeWidth)>0?'Remove stroke':'Add stroke'} title={Number(activeValues.strokeWidth)>0?'Remove stroke':'Add stroke'}><span aria-hidden="true">{Number(activeValues.strokeWidth)>0?'−':'+'}</span></button></div>
  {#if Number(activeValues.strokeWidth)>0}
    <AnimatedRow {layer} path={channel('stroke')} label="Color"><ColorField {...controlProps} label="Stroke color" get={()=>activeValues.stroke} edit={bind(channel('stroke'))} /></AnimatedRow>
    <ChannelRow {layer} channel={channel('strokeWidth')} property={property('strokeWidth')} label="Weight" step={0.5} min={0} unit="px" />
    <ChannelRow {layer} channel={channel('strokeOpacity')} property={property('strokeOpacity')} label="Opacity" step={1} min={0} max={100} unit="%" />
  {/if}
{/if}

{#if maskTargets.length}
  <Section title="Mask paths" />
  {#each maskTargets as target (target.path.id)}
    <details><summary>{target.path.name}</summary>
      <div class="buttons"><button class="chip" onclick={()=>selectPath(target,true)}>Edit vertices</button><button class="chip" onclick={()=>mutate('Remove path',()=>{const mask=layer.masks.find((candidate:any)=>candidate.path===target.path);if(mask)layer.masks=layer.masks.filter((candidate:any)=>candidate!==mask);})}>Remove</button></div>
      {#each records.filter((record:any)=>record.key.startsWith(target.prefix+'.')&&!record.key.includes('.v.')) as record (record.key)}
        {@const value=api.anim.evP(layer,record.prop,transport.time,record.key)}
        {#if typeof value==='number'}<ChannelRow {layer} channel={record.key} property={record.prop} label={label(record.label)} step={1} />
        {:else}<AnimatedRow {layer} path={record.key} label={label(record.label)}>{#if typeof value==='boolean'}<button class="compact-toggle" class:on={!!value} onclick={()=>inspectorEdit.apply({type:'set_property',target:layer.id,path:record.key,value:!value,time:transport.time,preserveHandEdits:false},{label:'Edit '+record.key,origin:'inspector'})}>{value?'On':'Off'}</button>{:else}<ColorField {...controlProps} label={label(record.label)} get={()=>value} edit={bind(record.key)} />{/if}</AnimatedRow>{/if}
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
      <button class="chip" onclick={()=>mutate('Remove text control',()=>{layer.d.animators=layer.d.animators?.filter((candidate:any)=>candidate.id!==item.id);layer.d.styles=layer.d.styles?.filter((candidate:any)=>candidate.id!==item.id);})}>Remove</button>
      {#each records.filter((record:any)=>record.key.startsWith(prefix+'.')) as record (record.key)}
        {@const value=api.anim.evP(layer,record.prop,transport.time,record.key)}
        {#if typeof value==='number'}<ChannelRow {layer} channel={record.key} property={record.prop} label={label(record.label)} step={1} />
        {:else}<AnimatedRow {layer} path={record.key} label={label(record.label)}>{#if record.label==='unit'}<SelectField {...controlProps} label="Based on" get={()=>value} edit={bind(record.key)} options={['characters','words','lines']} />{:else}<ColorField {...controlProps} label="Range color" get={()=>value} edit={bind(record.key)} />{/if}</AnimatedRow>{/if}
      {/each}
    </details>
  {/each}
{/if}

<style>
  .section-head {
    position: relative;
  }

  /* Section.svelte suppresses the rule when it is the first child of a
     wrapper. These nested SVG sections still need Motioner's full-width
     separators, so opt them back in explicitly. */
  .section-head :global(.sec) {
    margin-top: 8px;
    border-top: 1px solid var(--section-line);
  }

  .section-head:first-of-type :global(.sec) {
    margin-top: 6px;
  }

  .section-action {
    position: absolute;
    right: 0;
    top: 50%;
    display: grid;
    width: 24px;
    height: 24px;
    padding: 0;
    place-items: center;
    transform: translateY(-50%);
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--tx-3);
    font-size: 18px;
    cursor: pointer;
  }

  .section-action:hover {
    background: var(--ink-1);
    color: var(--tx);
  }

  .path-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .path-item {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .path-edit {
    display: flex;
    min-width: 0;
    flex: 1;
    align-items: center;
    gap: 7px;
    height: var(--ctl-h);
    padding: 0 9px;
    border: 0;
    border-radius: var(--r-sm);
    background: var(--bg-field);
    color: var(--tx-2);
    font: inherit;
    cursor: pointer;
  }

  .path-edit:hover,
  .path-item.active .path-edit {
    background: var(--bg-row-hi);
    color: var(--tx);
  }

  .path-icon {
    color: var(--tx-3);
    font-size: 16px;
  }

  .path-edit-label {
    margin-left: auto;
    color: var(--tx-3);
    font-size: var(--fs-xs);
  }

  .path-remove {
    display: grid;
    width: 24px;
    height: 24px;
    padding: 0;
    place-items: center;
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--tx-3);
    font-size: 18px;
    cursor: pointer;
  }

  .path-remove:hover {
    background: var(--ink-1);
    color: var(--tx);
  }

  .advanced {
    margin: 12px 0 0;
    border-top: var(--hairline) solid var(--line);
    padding-top: 8px;
  }

  .advanced summary {
    padding: 4px 0;
    color: var(--tx-3);
    font-size: var(--fs-xs);
    font-weight: var(--fw-medium);
    cursor: pointer;
  }

  .advanced[open] summary {
    margin-bottom: 6px;
    color: var(--tx-2);
  }

  .advanced-label {
    padding: 10px 0 3px;
    color: var(--tx-3);
    font-size: var(--fs-xs);
  }

  .compact-toggle {
    min-width: 58px;
    height: 24px;
    border: 0;
    border-radius: var(--r-sm);
    background: var(--bg-field);
    color: var(--tx-3);
    font: inherit;
    cursor: pointer;
  }

  .compact-toggle.on {
    background: color-mix(in srgb,var(--accent) 18%,var(--bg-field));
    color: var(--accent);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  details:not(.advanced) {
    margin: 6px 0;
  }

  details:not(.advanced) > summary {
    padding: 6px 0;
    font-weight: var(--fw-medium);
    cursor: pointer;
  }
</style>
