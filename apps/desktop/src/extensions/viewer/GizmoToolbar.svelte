<script lang="ts">
  import {onDestroy,untrack} from 'svelte';
  import type {PowermoveAPI} from 'powermove';
  let {api}:{api:PowermoveAPI}=$props();
  const {Segmented,SelectField}=untrack(()=>api.ui.controls);
  let mode=$state(untrack(()=>api.scene3d.getMode())),space=$state(untrack(()=>api.scene3d.getSpace()));
  const subscription=untrack(()=>api.scene3d.onGizmoChange(state=>{mode=state.mode;space=state.space;}));
  let navigation=$state(untrack(()=>api.scene3d.getNavigationMode()));
  const navigationSubscription=untrack(()=>api.scene3d.onNavigationChange(next=>navigation=next));
  let view=$state(untrack(()=>api.scene3d.getView()));
  const viewSubscription=untrack(()=>api.scene3d.onViewChange(next=>view=next));
  const viewOptions=[{v:'select',label:'Select'},{v:'orbit',label:'Orbit'},{v:'pan',label:'Pan'},{v:'dolly',label:'Dolly'},
    {v:'frame-selection',label:'Frame selection'},{v:'frame-all',label:'Frame all'}];
  onDestroy(()=>subscription.dispose());
  onDestroy(()=>navigationSubscription.dispose());
  onDestroy(()=>viewSubscription.dispose());
</script>
<Segmented label="3D transform" options={[{id:'translate',label:'Move'},{id:'rotate',label:'Rotate'},{id:'scale',label:'Scale'}]} value={mode} onChange={next=>{api.scene3d.setNavigationMode('select');api.scene3d.setMode(next as typeof mode);}} />
<Segmented label="3D axes" options={[{id:'world',label:'Global'},{id:'local',label:'Local'}]} value={space} onChange={next=>api.scene3d.setSpace(next as typeof space)} />
<Segmented label="3D viewpoint" options={[{id:'camera',label:'Camera'},{id:'editor',label:'Editor'}]} value={view} onChange={next=>api.scene3d.setView(next as typeof view)} />
<div style="width:100px" title="3D view: middle-drag or Alt-drag to orbit; Shift to pan. In a navigation tool, scroll to dolly, F to frame selection, Home to frame all. Navigation changes only the editor view. Camera returns to the rendered shot.">
  <SelectField {api} label="3D view" get={()=>navigation} edit={{mode:'local',label:'3D view',set:(next:unknown)=>api.scene3d.setNavigationMode(next as typeof navigation)}} options={viewOptions}
    onChange={(next:unknown)=>{if(next==='frame-selection'||next==='frame-all'){api.scene3d.frame(next==='frame-selection');return false;}return true;}} />
</div>
