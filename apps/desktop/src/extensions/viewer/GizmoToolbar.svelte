<script lang="ts">
  /* The 3D viewer's toolbar, left to right:
     - what you look through: the shot's camera, or a free view that never changes the shot;
     - one row of tools, each saying what a drag does: move, rotate or scale the selection;
       orbit, pan or zoom the view; or point light at a model;
     - for move, rotate and scale: which way the handles face (the light tool's options are in its panel);
     - frame the selection;
     - how the viewer draws 3D: fast (real time) or quality (Studio's path tracer). */
  import {onDestroy,untrack} from 'svelte';
  import type {PowermoveAPI} from 'powermove';
  /* `transform`: the 3D Move / Rotate / Scale tools and their axes, in the
     viewer header. `view`: which view you look through, in the viewer footer
     beside zoom, as After Effects puts its 3D View menu. */
  let {api,part='transform'}:{api:PowermoveAPI;part?:'transform'|'view'}=$props();
  const {Segmented,SelectField}=untrack(()=>api.ui.controls);
  let mode=$state(untrack(()=>api.scene3d.getMode())),space=$state(untrack(()=>api.scene3d.getSpace()));
  const subscription=untrack(()=>api.scene3d.onGizmoChange(next=>{mode=next.mode;space=next.space;}));
  let navigation=$state(untrack(()=>api.scene3d.getNavigationMode()));
  const navigationSubscription=untrack(()=>api.scene3d.onNavigationChange(next=>navigation=next));
  let view=$state(untrack(()=>api.scene3d.getView()));
  let layout=$state(untrack(()=>api.scene3d.views.get().layout));
  const layoutSubscription=untrack(()=>api.scene3d.views.onChange(()=>layout=api.scene3d.views.get().layout));
  onDestroy(()=>layoutSubscription.dispose());
  const viewSubscription=untrack(()=>api.scene3d.onViewChange(next=>view=next));
  /* Two views a newcomer can tell apart. The internal ids stay camera/editor. */
  let light=$state(untrack(()=>api.scene3d.lights.getTool()));
  let preview=$state(untrack(()=>api.scene3d.studio.getPreview()));
  const lightSubscription=untrack(()=>api.scene3d.lights.onToolChange(next=>light=next));
  const previewSubscription=untrack(()=>api.scene3d.studio.onPreviewChange(next=>preview=next));
  onDestroy(()=>{lightSubscription.dispose();previewSubscription.dispose();});
  const views=[
    {id:'camera',label:'Active Camera',title:'Active Camera — See the shot exactly as it renders and exports'},
    {id:'editor',label:'Free View',title:'Free View — Look around the 3D scene without moving the camera. Middle-drag or Option-drag to orbit; your render doesn’t change'},
  ];
  const viewOptions=[{v:'select',label:'Select'},{v:'orbit',label:'Orbit'},{v:'pan',label:'Pan'},{v:'dolly',label:'Dolly'},
    {v:'frame-selection',label:'Frame Selection'},{v:'frame-all',label:'Frame All'},
    /* After Effects' 3D View angles, for the Free View. */
    {v:'look-front',label:'Front'},{v:'look-left',label:'Left'},{v:'look-top',label:'Top'},
    {v:'look-back',label:'Back'},{v:'look-right',label:'Right'},{v:'look-bottom',label:'Bottom'}];
  /* Navigating always happens in the Free View. Picking a navigation tool
     switches to it visibly, rather than a gesture doing it by surprise. */
  const navigate=(next:unknown)=>{
    if(next==='frame-selection'||next==='frame-all'){api.scene3d.setView('editor');api.scene3d.frame(next==='frame-selection');return false;}
    if(typeof next==='string'&&next.startsWith('look-')){api.scene3d.lookFrom(next.slice(5) as Parameters<typeof api.scene3d.lookFrom>[0]);return false;}
    if(next!=='select'&&api.scene3d.getView()!=='editor')api.scene3d.setView('editor');
    return true;
  };
  onDestroy(()=>subscription.dispose());
  onDestroy(()=>navigationSubscription.dispose());
  onDestroy(()=>viewSubscription.dispose());
</script>
{#if part==='transform'}
<Segmented label="3D transform" options={[{id:'translate',label:'Move'},{id:'rotate',label:'Rotate'},{id:'scale',label:'Scale'},{id:'light',label:'Light',title:'Click a surface to place a light'}]} value={light.on?'light':mode} onChange={next=>{if(next==='light'){api.scene3d.lights.setTool({on:!light.on});return;}api.scene3d.lights.setTool({on:false});api.scene3d.setNavigationMode('select');api.scene3d.setMode(next as typeof mode);}} />
<Segmented label="3D axes" options={[{id:'world',label:'Global'},{id:'local',label:'Local'}]} value={space} onChange={next=>api.scene3d.setSpace(next as typeof space)} />
{:else}
<Segmented label="3D preview" options={[{id:"fast",label:"Fast"},{id:"quality",label:"Quality"}]} value={preview} onChange={next=>api.scene3d.studio.setPreview(next as typeof preview)} />
<Segmented label="3D view" options={views} value={view} onChange={next=>api.scene3d.setView(next as typeof view)} />
<div style="width:84px" title="View layout — 1 View, or 2 or 4 views side by side, each looking through the Active Camera, Front, Left, Top… Click a view to work in it">
  <SelectField {api} label="View layout" get={()=>layout} edit={{mode:'local',label:'View layout',set:(next:unknown)=>api.scene3d.views.setLayout(Number(next) as 1|2|4)}} options={[{v:1,label:'1 View'},{v:2,label:'2 Views'},{v:4,label:'4 Views'}]} />
</div>
<div style="width:96px" title="3D navigation — Orbit, pan or dolly the Free View, frame the scene, or look from the Front, Left, Top, Back, Right or Bottom. F frames the selection, Home frames everything. Your camera and render don’t change">
  <SelectField {api} label="3D navigation" get={()=>navigation} edit={{mode:'local',label:'3D navigation',set:(next:unknown)=>api.scene3d.setNavigationMode(next as typeof navigation)}} options={viewOptions}
    onChange={navigate} />
</div>
{/if}
