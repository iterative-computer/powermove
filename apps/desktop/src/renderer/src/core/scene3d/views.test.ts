import { afterEach, expect, it } from 'vitest';
import { makePM } from '../../legacy/__tests__/make-pm';
import { viewsState,setViewLayout,setActivePane,setPaneView } from './views';
import { getViewportMode } from './viewport';
import { disposeSceneRuntimes } from './service';
const editors:any[]=[];
function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Views'});PM.time=0;editors.push(PM);return PM;
}
afterEach(()=>{for(const PM of editors.splice(0))disposeSceneRuntimes(PM);});
const snapshot=(PM:any)=>JSON.stringify(viewsState(PM));
it('undoes/redoes layout, pane selection and view choices and reopens the same views',()=>{
  const PM=editor();
  for(const action of [()=>setViewLayout(PM,4),()=>setPaneView(PM,0,'left'),()=>setActivePane(PM,0),()=>setPaneView(PM,0,'back'),()=>setViewLayout(PM,2)]){
    const before=snapshot(PM);action();const after=snapshot(PM);
    expect(after).not.toBe(before);
    expect(PM.hist.undo()).toBe(true);expect(snapshot(PM)).toBe(before);
    expect(PM.hist.redo()).toBe(true);expect(snapshot(PM)).toBe(after);
    PM.proj=PM.deserialize(PM.serialize());PM.touch();expect(snapshot(PM)).toBe(after);
  }
});
it('keeps saved layouts per composition and defaults older projects to one camera view',()=>{
  const PM=editor(),first=PM.proj.compId;
  setViewLayout(PM,4);const saved=snapshot(PM);
  const next=PM.Comps.create({name:'Second'});expect(viewsState(PM).layout).toBe(1);
  setViewLayout(PM,2);PM.Comps.open(first);expect(snapshot(PM)).toBe(saved);
  PM.Comps.open(next);expect(viewsState(PM).layout).toBe(2);
  PM.proj=PM.mkProject();expect(viewsState(PM)).toEqual({layout:1,active:0,panes:[{view:'camera',pose:null}]});
  expect(getViewportMode(PM)).toBe('camera');
});
it('rejects invalid persisted view edits atomically',()=>{
  const PM=editor(),before=PM.serialize();
  for(const views3d of [{layout:4,active:7,panes:[]},{layout:1,active:0,panes:[{view:'custom',pose:{position:[0,0,0],target:[0,0,0],zoom:NaN}}]}]){
    expect(PM.Edit.apply({type:'set_composition',patch:{views3d}}).ok).toBe(false);
    expect(PM.serialize()).toBe(before);
  }
});
