<script lang="ts">
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
  const viewSubscription=untrack(()=>api.scene3d.onViewChange(next=>view=next));
  /* Two views a newcomer can tell apart. The internal ids stay camera/editor. */
  const views=[
    {id:'camera',label:'Active Camera',title:'Active Camera — View the rendered shot. Orbit, Pan and Dolly edit the active camera'},
    {id:'editor',label:'Free View',title:'Free View — Look around the 3D scene without moving the camera. Middle-drag or Option-drag to orbit; your render doesn’t change'},
  ];
  const viewOptions=[{v:'select',label:'Select'},{v:'orbit',label:'Orbit'},{v:'pan',label:'Pan'},{v:'dolly',label:'Dolly'},
    {v:'frame-selection',label:'Frame Selection'},{v:'frame-all',label:'Frame All'}];
  /* Camera tools use the selected view. Framing stays a Free View action. */
  const navigate=(next:unknown)=>{
    if(next==='frame-selection'||next==='frame-all'){api.scene3d.setView('editor');api.scene3d.frame(next==='frame-selection');return false;}
    return true;
  };
  onDestroy(()=>subscription.dispose());
  onDestroy(()=>navigationSubscription.dispose());
  onDestroy(()=>viewSubscription.dispose());
</script>
{#if part==='transform'}
<Segmented label="3D transform" options={[{id:'translate',label:'Move'},{id:'rotate',label:'Rotate'},{id:'scale',label:'Scale'}]} value={mode} onChange={next=>{api.scene3d.setNavigationMode('select');api.scene3d.setMode(next as typeof mode);}} />
<Segmented label="3D axes" options={[{id:'world',label:'Global'},{id:'local',label:'Local'}]} value={space} onChange={next=>api.scene3d.setSpace(next as typeof space)} />
{:else}
<Segmented label="3D view" options={views} value={view} onChange={next=>api.scene3d.setView(next as typeof view)} />
<div style="width:96px" title="3D navigation — Active Camera edits your camera; Free View looks around. F frames the selection, Home frames everything">
  <SelectField {api} label="3D navigation" get={()=>navigation} edit={{mode:'local',label:'3D navigation',set:(next:unknown)=>api.scene3d.setNavigationMode(next as typeof navigation)}} options={viewOptions}
    onChange={navigate} />
</div>
{/if}
