<script module lang="ts">
  /** Which parts are open in the tree, per model, for the session. */
  const expanded = new Map<string, boolean>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import type { EditCommand, Layer, PowermoveAPI } from 'powermove';
  import type { SceneUiState } from './scene-state.svelte';
  import PartRow from './PartRow.svelte';
  import { partEdited, resetPart, togglePartVisible } from './parts';

  /* Parts: the pieces inside an imported model (a console's body, its two
     controllers…), each movable and keyframeable without splitting the
     model into layers. Pick one here, or double-click it in the viewer; the
     viewer's Move, Rotate and Scale handles then work on that part. */
  let { api, ui, layer }: { api: PowermoveAPI; ui: SceneUiState; layer: Layer } = $props();
  const { Section, Row, TextField, NumField } = untrack(() => api.ui.controls);

  let version = $state(0);
  $effect(() => {
    const subscription = api.scene3d.onActivePartChange(() => { version++; });
    return () => subscription.dispose();
  });
  const parts = $derived((ui.version, api.scene3d.listParts(layer.id)));
  const active = $derived.by(() => { void version; return api.scene3d.getActivePart(layer.id); });
  const selected = $derived.by(() => { void version; void ui.version; return api.scene3d.getSelectedParts(layer.id).filter(key => parts.some(part => part.key === key)); });
  const activePart = $derived(selected.length === 1 ? parts.find(part => part.key === active) ?? null : null);
  const byKey = $derived(new Map(parts.map(part => [part.key, part])));
  const siblings = (parent: string | null) => parts.filter(part => part.parent === parent).length;

  /* Wrappers with a single child open by themselves, as does the path to the
     part being edited, so the interesting level shows without clicking. */
  function isOpen(key: string): boolean {
    void version;
    const remembered = expanded.get(`${layer.id}:${key}`);
    if (remembered != null) return remembered;
    for (const selectedKey of selected) {
      for (let parent: string | null = selectedKey; parent; parent = byKey.get(parent)?.parent ?? null) if (parent === key) return true;
    }
    const part = byKey.get(key);
    return !!part && (siblings(part.parent) === 1 || part.depth === 0 && parts.length <= 24);
  }
  function toggle(key: string, event: MouseEvent): void {
    event.stopPropagation();
    expanded.set(`${layer.id}:${key}`, !isOpen(key));
    version++;
  }
  const shown = $derived.by(() => {
    void version;
    return parts.filter(part => {
      for (let parent = part.parent; parent != null; parent = byKey.get(parent)?.parent ?? null) if (!isOpen(parent)) return false;
      return true;
    });
  });
  const dragType='application/x-powermove-model-parts';
  type PartDrop={row:string|null;edge:'inside'|'before'|'after';destination:string|null;depth:number};
  let dragging=$state<string[]>([]),dropTarget=$state<PartDrop|null>(null);
  function ancestors(key:string):string[]{
    const chain:string[]=[];
    for(let parent=byKey.get(key)?.parent;parent;parent=byKey.get(parent)?.parent)chain.push(parent);
    return chain;
  }
  function fileLevel(key:string):string|null {
    return byKey.get(key)?.group?ancestors(key).find(parent=>!byKey.get(parent)?.group) ?? null
      :key.includes('_')?key.slice(0,key.lastIndexOf('_')):null;
  }
  function canMove(keys:readonly string[],destination:string|null):boolean {
    if(!keys.length || destination && !byKey.get(destination)?.group)return false;
    if(destination && keys.some(key=>key===destination || ancestors(destination).includes(key)))return false;
    const level=destination?fileLevel(destination):fileLevel(keys[0]!);
    return keys.every(key=>byKey.has(key) && fileLevel(key)===level)
      && keys.some(key=>byKey.get(key)?.parent!==(destination || level));
  }
  function move(keys:readonly string[],destination:string|null):void {
    const result=api.scene3d.moveParts(layer.id,keys,destination);
    if(!result.ok){api.ui.toast(result.message,{error:true});return;}
    if(destination)expanded.set(`${layer.id}:${destination}`,true);
    version++;
  }
  function dragStart(event:DragEvent,key:string):void {
    if(!event.dataTransfer || (event.target as HTMLElement)?.closest('button')){event.preventDefault();return;}
    if(!selected.includes(key))choose(key);
    dragging=[...api.scene3d.getSelectedParts(layer.id)];
    event.stopPropagation();event.dataTransfer.effectAllowed='move';
    event.dataTransfer.setData(dragType,JSON.stringify({layerId:layer.id,keys:dragging}));
  }
  function outsideDrop():PartDrop {return {row:null,edge:'after',destination:null,depth:(byKey.get(fileLevel(dragging[0] || '') || '')?.depth ?? -1)+2};}
  function rowDrop(event:DragEvent,key:string):PartDrop|null {
    if(!dragging.length)return null;
    if(key==='')return {row:key,edge:'inside',destination:null,depth:0};
    const part=byKey.get(key);if(!part)return null;
    const rect=(event.currentTarget as HTMLElement).getBoundingClientRect(),y=event.clientY-rect.top;
    let edge:PartDrop['edge']=y<6?'before':y>rect.height-6?'after':'inside';
    let parent=edge==='inside' && part.group?key:edge==='inside' && dragging.every(key=>fileLevel(key)===part.key)?part.key:part.parent;
    // Moving left of the child indentation lifts the drop out of custom groups.
    while(parent && byKey.get(parent)?.group && event.clientX<rect.left+6+(byKey.get(parent)!.depth+2)*14-7){
      parent=byKey.get(parent)!.parent;edge=edge==='before'?'before':'after';
    }
    const destination=parent && byKey.get(parent)?.group?parent:null;
    if(!destination && dragging.some(key=>fileLevel(key)!==parent))return null;
    if(edge==='inside' && parent!==part.key)edge='after';
    return {row:key,edge,destination,depth:(parent?byKey.get(parent)!.depth:-1)+2};
  }
  function dragOver(event:DragEvent,target:PartDrop|null):void {
    event.stopPropagation();dropTarget=null;
    if(!target || !canMove(dragging,target.destination))return;
    event.preventDefault();
    if(event.dataTransfer)event.dataTransfer.dropEffect='move';
    dropTarget=target;
  }
  function dragLeave(event:DragEvent):void {
    if(!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node|null))dropTarget=null;
  }
  function drop(event:DragEvent,target:PartDrop|null):void {
    event.preventDefault();event.stopPropagation();dropTarget=null;
    try{
      const payload=JSON.parse(event.dataTransfer?.getData(dragType) || 'null');
      if(target && payload?.layerId===layer.id && Array.isArray(payload.keys) && canMove(payload.keys,target.destination))move(payload.keys,target.destination);
    }catch {/* Ignore other applications' drag data. */}
    dragging=[];
  }
  let rangeAnchor: string | null = null;
  function choose(key: string | null, event?: MouseEvent | KeyboardEvent): void {
    let keys = key ? [key] : [];
    if (key && event?.shiftKey && rangeAnchor) {
      const from = shown.findIndex(part => part.key === rangeAnchor), to = shown.findIndex(part => part.key === key);
      if (from >= 0 && to >= 0) keys = shown.slice(Math.min(from,to), Math.max(from,to)+1).map(part => part.key);
    } else if (key && (event?.metaKey || event?.ctrlKey)) keys = selected.includes(key) ? selected.filter(item => item !== key) : [...selected, key];
    if (!event?.shiftKey) rangeAnchor = key;
    api.scene3d.setSelectedParts(layer.id, keys);
    api.transport.invalidate();
  }
  function group(): void {
    const result = api.scene3d.groupParts(layer.id, selected);
    if (!result.ok) api.ui.toast(result.message, { error: true });
  }
  function renameCommand(value: unknown): EditCommand {
    const current=api.model.layer(layer.id)!, data=(current.d as any).data, key=activePart!.key;
    return {type:'set_content',target:layer.id,patch:{data:{...data,object:{...data.object,parts:{...data.object.parts,[key]:{...data.object.parts[key],name:String(value).trim().slice(0,160)||'Part Group'}}}}}};
  }
  const canGroup = $derived(selected.length > 1 && selected.every(key => byKey.get(key)?.parent === byKey.get(selected[0]!)?.parent));
  const animatedGroup = $derived(activePart?.group && Object.values<any>((layer.d as any)?.data?.object?.parts?.[activePart.key]?.p || {}).some(p => p.kf?.length || p.expr));
  function ungroup(): void {
    if (!activePart?.group) return;
    const result = api.scene3d.ungroupParts(layer.id, activePart.key);
    if (!result.ok) api.ui.toast(result.message, { error: true });
  }
  function onKey(event: KeyboardEvent, key: string | null): void {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'g') {
      event.preventDefault(); event.stopPropagation();
      if (event.shiftKey) ungroup(); else if (canGroup) group();
    } else if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(key, event); }
  }
  function menu(event: MouseEvent, key: string): void {
    event.preventDefault();
    if (!selected.includes(key)) choose(key);
    api.ui.menu({x:event.clientX,y:event.clientY}, [
      {label:'Group Parts',kb:'⌘G',disabled:api.scene3d.getSelectedParts(layer.id).length < 2,run:group},
      {label:'Move to Group',submenu:parts.filter(part=>part.group && canMove(selected,part.key)).map(part=>({label:part.name,run:()=>move(selected,part.key)})),disabled:!parts.some(part=>part.group && canMove(selected,part.key))},
      {label:'Move out of Group',disabled:!canMove(selected,null),run:()=>move(selected,null)},
      {label:'Ungroup Parts',kb:'⌘⇧G',disabled:!byKey.get(key)?.group,run:()=>{const result=api.scene3d.ungroupParts(layer.id,key);if(!result.ok)api.ui.toast(result.message,{error:true});}}
    ]);
  }
</script>

{#if parts.length}
  <Section {api} title="Parts">
    <button type="button" class="part-action" disabled={!canGroup} title={selected.length > 1 && !canGroup ? 'Select parts at the same level' : 'Group selected parts (⌘G)'} aria-label="Group selected parts" onclick={group}>Group</button>
  </Section>
  <div class="parts" data-parts-section>
    <div class="part-tree" class:drop-outside={dropTarget?.row===null} style={`--drop-depth:${dropTarget?.depth ?? 0}`}
      ondragover={event=>{if(event.target===event.currentTarget)dragOver(event,outsideDrop());}} ondragleave={dragLeave}
      ondrop={event=>{if(event.target===event.currentTarget)drop(event,outsideDrop());}} role="tree" tabindex="-1" aria-label="Model parts" aria-multiselectable="true">
      <div class="part" role="treeitem" tabindex="0" aria-selected={!active} class:on={!active} class:drop-target={dropTarget?.row===''} data-part-key=""
        ondragover={event=>dragOver(event,rowDrop(event,''))} ondragleave={dragLeave} ondrop={event=>drop(event,rowDrop(event,''))}
        onclick={(event) => choose(null, event)} onkeydown={(event) => onKey(event, null)}>
        <span class="twirl-gap" aria-hidden="true"></span>
        <span class="part-icon" aria-hidden="true">{@html api.ui.icon('cube')}</span>
        <span class="part-name">Whole Model</span>
      </div>
      {#each shown as part (part.key)}
        {@const open = isOpen(part.key)}
        {@const visible = (ui.version, ui.time, api.scene3d.getPart(layer.id, part.key, ui.time).values.visible)}
        {@const edited = (ui.version, partEdited(layer, part.key))}
        <div class="part" role="treeitem" tabindex="0" aria-selected={selected.includes(part.key)} aria-expanded={part.children ? open : undefined}
          draggable="true" ondragstart={event=>dragStart(event,part.key)} ondragend={()=>{dragging=[];dropTarget=null;}}
          ondragover={event=>dragOver(event,rowDrop(event,part.key))} ondragleave={dragLeave} ondrop={event=>drop(event,rowDrop(event,part.key))}
          class:drop-target={dropTarget?.row===part.key && dropTarget.edge==='inside'}
          class:drop-before={dropTarget?.row===part.key && dropTarget.edge==='before'} class:drop-after={dropTarget?.row===part.key && dropTarget.edge==='after'} class:dragging={dragging.includes(part.key)} class:on={selected.includes(part.key)} class:hidden-part={!visible} data-part-key={part.key} style={`--depth:${part.depth + 1};--drop-depth:${dropTarget?.depth ?? 0}`}
          title={part.name} onclick={(event) => choose(part.key, event)} oncontextmenu={(event) => menu(event, part.key)} onkeydown={(event) => onKey(event, part.key)}>
          {#if part.children}
            <button type="button" class="twirl" aria-label={`${open ? 'Collapse' : 'Expand'} ${part.name}`} onclick={(event) => toggle(part.key, event)}>{@html api.ui.icon(open ? 'chevD' : 'chev')}</button>
          {:else}<span class="twirl-gap" aria-hidden="true"></span>{/if}
          {#if part.group}<span class="part-icon" aria-hidden="true">{@html api.ui.icon('layers')}</span>{/if}
          <span class="part-name">{part.name}</span>
          {#if edited}<span class="part-dot" title="Moved or animated" aria-label="Changed"></span>{/if}
          <button type="button" class="part-eye" class:off={!visible} aria-label={`${visible ? 'Hide' : 'Show'} ${part.name}`} aria-pressed={!visible}
            title={visible ? 'Hide this part' : 'Show this part'} onclick={(event) => { event.stopPropagation(); togglePartVisible(api, layer.id, part.key, part.name); }}>
            {@html api.ui.icon(visible ? 'eye' : 'eyeoff')}
          </button>
        </div>
      {/each}
    </div>
    {#if activePart}
      <div class="part-edit" data-part-editor={activePart.key}>
        <div class="row part-head">
          <span class="part-title">{activePart.name}</span>
          {#if activePart.group}<button type="button" class="part-reset" disabled={!!animatedGroup} title={animatedGroup ? 'Remove group animation before ungrouping' : 'Ungroup parts and keep their placement'} onclick={ungroup}>Ungroup</button>{/if}
          {#if partEdited(layer, activePart.key)}<button type="button" class="part-reset" title={`Put ${activePart.name} back where the model file has it`} onclick={() => resetPart(api, layer.id, activePart.key, activePart.name)}>Reset</button>{/if}
        </div>
        {#if activePart.group}
          <Row {api} label="Name"><TextField {api} get={() => activePart?.name || ''} edit={{mode:'command',label:'Rename part group',origin:'inspector',command:renameCommand}} label="Part group name" /></Row>
          <div data-part-anchor={activePart.key}>
            <Row {api} label="Anchor Point" pair={true}>
              {#each ['x','y','z'] as axis,index}
                <div class="well" data-axis={axis.toUpperCase()} data-part-anchor-axis={axis}>
                  <NumField {api} get={()=>((api.model.layer(layer.id)?.d as any)?.data?.object?.parts?.[activePart!.key]?.pivot || [0,0,0])[index]}
                    edit={{mode:'command',label:`Group Anchor Point ${axis.toUpperCase()}`,origin:'inspector',command:(next:unknown)=>api.scene3d.partGroupAnchorCommands(layer.id,activePart!.key,{[axis]:Number(next)})}}
                    label={`Group Anchor Point ${axis.toUpperCase()}`} ariaLabel={`Group Anchor Point ${axis.toUpperCase()}`} step={0.01} />
                </div>
              {/each}
            </Row>
          </div>
        {/if}
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Position" fields={['x', 'y', 'z']} step={0.01} />
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Rotation" fields={['rx', 'ry', 'rz']} unit="°" />
        <PartRow {api} {ui} {layer} {version} part={activePart.key} label="Scale" fields={['scale']} unit="%" min={0} />
      </div>
    {:else if selected.length > 1}
      <p class="part-hint">{selected.length} parts selected</p>
    {:else}
      <p class="part-hint">Double-click the model in the viewer to pick a part, then drag it with the handles.</p>
    {/if}
  </div>
{/if}

<style>
  /* The tree reads like the Timeline's layer rows: 26px rows, indented by
     depth, the edited part tinted with the accent. */
  .parts { display: flex; flex-direction: column; margin: 0 calc(8px - var(--pad)) 6px; }
  .part-tree { display: flex; flex-direction: column; max-height: 232px; overflow-y: auto; padding: 2px 0 16px; position: relative; }
  .part { position: relative; display: flex; align-items: center; gap: 6px; height: 26px; flex: none; padding: 0 4px 0 calc(6px + (var(--depth, 0) * 14px)); border-radius: 6px; color: var(--tx); cursor: default; }
  .part:hover { background: var(--bg-row); }
  .part:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .part.on { background: var(--accent-dim); color: var(--accent-tx); }
  .part.drop-target { background: var(--accent-dim); box-shadow: inset 0 0 0 1px var(--accent); }
  .part.drop-before::before, .part.drop-after::after, .part-tree.drop-outside::after { content: ''; position: absolute; left: calc(6px + var(--drop-depth) * 14px); right: 4px; height: 2px; background: var(--accent); pointer-events: none; z-index: 1; }
  .part.drop-before::before { top: 0; }
  .part.drop-after::after { bottom: 0; }
  .part-tree.drop-outside::after { bottom: 6px; }
  .part.dragging { opacity: .5; }
  .part.hidden-part .part-name { color: var(--tx-3); }
  .twirl, .twirl-gap { display: grid; place-items: center; width: 12px; height: 12px; flex: none; padding: 0; border: 0; background: none; color: var(--tx-2); }
  .twirl :global(svg) { width: 10px; height: 10px; }
  .part-icon { display: grid; place-items: center; width: 14px; height: 14px; margin-left: -4px; flex: none; color: var(--tx-2); }
  .part-icon :global(svg) { width: 13px; height: 13px; }
  .part.on .part-icon { color: inherit; }
  .part-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; line-height: 16px; }
  .part-dot { width: 6px; height: 6px; flex: none; border-radius: 50%; background: var(--accent); }
  .part-eye { display: grid; place-items: center; width: 20px; height: 20px; flex: none; padding: 0; border: 0; border-radius: 5px; background: none; color: var(--tx-3); opacity: 0; }
  .part-eye :global(svg) { width: 13px; height: 13px; }
  .part:hover .part-eye, .part-eye:focus-visible, .part-eye.off { opacity: 1; }
  .part-eye:hover { color: var(--tx); }
  .part-edit { margin-top: 6px; padding-top: 4px; border-top: 1px solid var(--line); }
  .part-head { height: 28px; margin: 0; padding: 0 6px; gap: 8px; }
  .part-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: var(--fw-semibold); color: var(--tx); }
  .part-reset { flex: none; padding: 0; border: 0; background: transparent; color: var(--tx-3); font-size: 12px; line-height: 16px; }
  .part-reset:hover { color: var(--tx); }
  .part-action { margin-left: auto; padding: 0 4px; color: var(--tx-2); background: transparent; border: 0; font: inherit; }
  .part-action:disabled, .part-reset:disabled { opacity: .4; cursor: default; }
  .part-hint { margin: 4px 6px 2px; color: var(--tx-3); font-size: 12px; line-height: 16px; }
</style>
