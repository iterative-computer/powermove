/** Runs inside the real Electron renderer. Only synthetic data and disposable DOM. */
export const scenarios = ['history-nesting', 'history-cancel', 'edit-cancel', 'edit-admission',
  'external-history', 'gesture-modes', 'workspace-direct', 'keymap-fields', 'keymap-order', 'menu-equivalents', 'history-async'];
export function runScenario({name, metadata}: {name:string; metadata:Record<string, unknown>}) {
  const PM = (window as any).PM, trace = (window as any).__PMTrace;
  const {history, edit, workspace} = trace.apis;
  const assert = (value: any, message: string) => { if (!value) throw new Error(`${name}: ${message}`); };
  const expectThrow = (fn: () => unknown) => { try { fn(); } catch (error) { return error; } throw new Error('Expected throw'); };
  const fresh = PM.mkProject({name:'Synthetic trace fixture'});
  fresh.created=0;
  PM.replaceProject(fresh);
  PM.hist.clear();
  PM.sel.layers = []; PM.sel.keys = [];
  if(name==='workspace-direct') {
    // Recreate a synthetic workspace fixture before each capture; never reuse a prior mutation.
    trace.workspaceFixture ??= JSON.parse(JSON.stringify(PM.WS.current));
    const current=JSON.parse(JSON.stringify(trace.workspaceFixture));
    PM.WS.current=current;
    const index=PM.WS.all.findIndex((w:any)=>w.id===current.id);
    if(index>=0)PM.WS.all[index]=current;
  }
  trace.start({...metadata, scenario:name, role:'trusted', apiVersions:[1,2,3], synthetic:true});
  try {
    if (name === 'history-async') {
      const promise=Promise.resolve(false);
      const result=history.do('sync commit of async callback',()=>{PM.proj.name='synchronous part';return promise;});
      assert(result===promise && PM.hist.canUndo(),'Promise returned unchanged, history already committed');
      trace.point('driver.after-return',{name:PM.proj.name,canUndo:PM.hist.canUndo()});
      result.then((value:any)=>{
        trace.point('callback.promise',{value});
        trace.call('PM.proj.direct',['name','asynchronous part'],()=>PM.proj.name='asynchronous part');
      });
      return null;
    } else if (name === 'menu-equivalents') {
      history.do('Menu edit',()=>{PM.proj.name='menu edit';});
      trace.point('driver.menu',{id:'contextUndo',equivalent:'CommandOrControl+Z'});
      return null; // The runner invokes the real Electron MenuItem and awaits renderer IPC.
    } else if (name === 'history-nesting') {
      const outer = history.begin('outer'), inner = history.begin('inner');
      assert(outer === inner, 'nested begin reuses snapshot');
      const sentinel = {label:'same return object'};
      assert(history.do('folded', () => { PM.proj.name='folded'; return sentinel; }) === sentinel,'exact callback result');
      assert(history.commit('inner') === false, 'inner commit');
      assert(history.commit('outer') === true, 'outer commit');
      assert(history.undo() && PM.proj.name==='Synthetic trace fixture','undo folded edit');
      assert(history.redo() && PM.proj.name==='folded','redo folded edit');
      const boom = new Error('synthetic callback failure');
      assert(expectThrow(() => history.do('throw', () => { PM.proj.name='bad'; throw boom; })) === boom,'same thrown error');
      assert(PM.proj.name==='folded','throw rollback');
      assert(history.commit()===false,'no pending commit');
    } else if (name === 'history-cancel') {
      history.begin('keep edits'); PM.proj.name='kept';
      assert(history.cancel()===undefined && PM.proj.name==='kept' && !PM.hist.canUndo(),'cancel keeps edits, no entry');
    } else if (name === 'edit-cancel') {
      const project=PM.proj;
      edit.begin('empty'); assert(edit.cancel()===true && PM.proj===project,'empty cancel retains project');
      edit.begin('changed');
      assert(edit.dispatch({type:'add_layer',id:'synthetic-layer',layerType:'shape',name:'Synthetic shape'}).ok,'live add');
      edit.cancel();
      assert(!PM.proj.layers.length && !PM.hist.canUndo(),'changed cancel reverts');
      assert(edit.cancel()===false,'no live cancel');
    } else if (name === 'edit-admission') {
      edit.begin('live');
      expectThrow(() => edit.begin('nested source'));
      let ran=false;
      const result=edit.mutate('refused', () => {ran=true;});
      assert(!result.ok && !ran,'mutate refused before callback');
      const nested=history.begin('scoped raw nesting'); assert(nested===null,'scoped history snapshot is null');
      history.commit(); edit.cancel();
      assert(!edit.mutate('invalid',null).ok,'invalid callback result');
      const sentinel={label:'callback identity'};
      const noop=edit.mutate('noop',()=>sentinel);
      assert(noop.ok && noop.data.result===sentinel,'EditResult wraps callback');
      assert(!edit.mutate('invalid non-tree',()=>{PM.proj.name='rejected';}).ok,'non-tree rollback');
      assert(PM.proj.name==='Synthetic trace fixture','rollback restored');
      assert(!edit.commit().ok,'commit without live');
      assert(!edit.apply([],{}).ok,'empty edit');
      assert(!edit.apply({type:'add_layer',layerType:'shape'}, {baseRevision:999}).ok,'stale revision');
    } else if (name === 'external-history') {
      let local=1;
      assert(history.external('invalid',null,()=>{})===null,'invalid external');
      history.begin('pending document'); PM.proj.name='pending';
      const undo=()=>{ local=0; trace.point('callback.external',{phase:'undo',local}); };
      const redo=()=>{ local=1; trace.point('callback.external',{phase:'redo',local}); };
      const cleanup=()=>trace.point('callback.external',{phase:'cleanup'});
      const id=history.external('external',undo,redo,{cleanup,group:'synthetic-group'});
      assert(typeof id==='string' && PM.hist.busy(),'external independent of pending');
      assert(history.undo() && local===0 && PM.hist.busy(),'undo while begin open');
      assert(history.redo() && local===1,'external redo');
      history.cancel(); PM.hist.clear();
      history.external('throw external',()=>{throw new Error('synthetic external failure');},()=>{});
      expectThrow(()=>history.undo()); assert(!PM.hist.canUndo() && PM.hist.canRedo(),'cursor moved before throw');
      PM.hist.clear();
    } else if (name === 'gesture-modes') {
      const Gesture=trace.gesture;
      const setter=(value:any)=>{PM.proj.name=value;return value;};
      const set=new Gesture({edit,history},{mode:'set',label:'set gesture',set:setter});
      set.begin();set.write('kept by set cancel');set.cancel();
      assert(PM.proj.name==='kept by set cancel','set cancel keeps write');
      assert(set.once('set once')==='set once','set once callback return');
      const command=new Gesture({edit,history},{mode:'command',label:'command gesture',command:{type:'add_layer',id:'gesture-layer',layerType:'shape'}});
      command.begin();command.write(1);command.cancel();
      assert(!PM.proj.layers.length,'command cancel reverts');
      command.begin();command.write(1);command.commit();
      assert(PM.proj.layers.length===1,'command commit');
      let value:any;
      const local=new Gesture({edit,history},{mode:'local',label:'local',set:(x:any)=>{value=x;}});
      local.begin();local.write(2);local.commit();local.cancel();
      assert(local.once(3)===undefined && value===3,'local once');
    } else if (name === 'workspace-direct') {
      workspace.mutate((w:any)=>{w.name='Synthetic workspace';},{inPlace:true});
      const boom=new Error('synthetic workspace failure');
      assert(expectThrow(()=>workspace.mutate((w:any)=>{w.name='Partial synthetic workspace';throw boom;},{inPlace:true}))===boom,'workspace rethrows');
      assert(PM.WS.current.name==='Partial synthetic workspace','workspace retains partial writes on throw');
      const held=PM.proj;
      // This explicit operation boundary observes the assignment without proxying PM.proj.
      const result=trace.call('PM.proj.direct',['name','raw synthetic'],()=>PM.proj.name='raw synthetic');
      assert(result==='raw synthetic' && PM.proj===held && !PM.hist.canUndo(),'raw assignment keeps reference and creates no history');
      PM.touch(); trace.checkpoint('after direct touch');
    } else if (name.startsWith('keymap-')) {
      const kernel=PM.Kernel, disposals:any[]=[];
      const owner='synthetic.trace';
      const root=document.createElement('div'), input=document.createElement('input');
      root.id='trace-panel';input.id='trace-field';root.append(input);document.body.append(root);
      const bind=(key:string,id:string,result:any,options:any={})=>{
        disposals.push(kernel.commands.register(owner,{id,title:id,run:()=>{trace.point('callback.command',{id});return result;}}));
        disposals.push(kernel.bind(owner,{key,command:id,...options}));
      };
      const key=(target:EventTarget,k:string,options:any={})=>{
        const down=new KeyboardEvent('keydown',{key:k,code:k==='Escape'?'Escape':k==='.'?'Period':`Key${k.toUpperCase()}`,bubbles:true,cancelable:true,...options});
        target.dispatchEvent(down);trace.point('dom.after',{event:down});
        const up=new KeyboardEvent('keyup',{key:k,bubbles:true,cancelable:true,...options});
        target.dispatchEvent(up);trace.point('dom.after',{event:up});return down;
      };
      const panel=(event:Event)=>trace.point('dom.panel',event);
      const later=(event:Event)=>trace.point('dom.window-later',event);
      root.addEventListener('keydown',panel);root.addEventListener('keyup',panel);
      window.addEventListener('keydown',later);window.addEventListener('keyup',later);
      try {
        if (name==='keymap-fields') {
          bind('q','trace.field-suppressed',undefined,{priority:-300});
          bind('q','trace.field-decline',false,{priority:-200,inFields:true});
          bind('q','trace.field-handle',undefined,{priority:-100,inFields:true});
          input.focus(); assert(key(input,'q').defaultPrevented,'field handled');
          kernel.unbind(owner,'q',false);
          bind('q','trace.field-only-decline',false,{priority:-200,inFields:true});
          assert(!key(input,'q').defaultPrevented,'field decline preserves default');
          bind('escape','trace.escape',false,{priority:-1000,inFields:true});
          key(input,'Escape');key(input,'.',{metaKey:true});
          root.tabIndex=0;root.focus();
          bind('q','trace.ordinary-decline',false,{priority:-300});
          assert(key(root,'q').defaultPrevented,'ordinary decline pre-cancels');
        } else {
          root.tabIndex=0;root.focus();
          bind('q','trace.first-decline',false,{priority:-300});
          bind('q','trace.next-handle',undefined,{priority:-200});
          key(root,'q');
          assert(!key(root,'q',{repeat:true}).defaultPrevented,'nonrepeat filtered');
          bind('q','trace.repeat-decline',false,{priority:-100,repeat:true});
          assert(!key(root,'q',{repeat:true}).defaultPrevented,'repeat decline does not cancel');
          bind('q','trace.repeat-handle',0,{priority:0,repeat:true});
          assert(key(root,'q',{repeat:true}).defaultPrevented,'zero handles repeat');
          bind('w','trace.promise-false',Promise.resolve(false),{priority:-300});
          bind('w','trace.after-promise',undefined,{priority:-200});key(root,'w');
          bind('r','trace.pending-command',new Promise(()=>{}),{priority:-300});
          key(root,'r');key(root,'Escape');key(root,'.',{metaKey:true});
          bind('e','trace.undefined',undefined,{priority:-300});
          const stop=(event:Event)=>{event.stopPropagation();trace.point('dom.panel-stop',event);};
          root.addEventListener('keydown',stop);key(root,'e');root.removeEventListener('keydown',stop);
          const cancel=(event:Event)=>{event.preventDefault();trace.point('dom.panel-cancel',event);};
          root.addEventListener('keydown',cancel);key(root,'e');root.removeEventListener('keydown',cancel);
        }
      } finally {
        window.removeEventListener('keydown',later);window.removeEventListener('keyup',later);
        for(const d of disposals) d.dispose();root.remove();
      }
    } else throw new Error(`Unknown scenario ${name}`);
    return trace.finish();
  } catch(error) { trace.recorder=null; throw error; }
}
