/* Powermove — promptable workspaces. */
(() => {
const PM = window.PM, h = PM.h, Schema = window.PMWorkspaceSchema;
const clone = Schema.clone;
const dock = (id, panels, size) => ({ id, size, panels });
const p = (id, o = {}) => ({ id, ...o });

/* Legacy presets are migrated by the schema at boot. Keeping them readable is
   an intentional compatibility test for every existing saved workspace. */
const PRESETS = () => ([
  { id:'design', name:'Design', builtin:true, density:'normal', theme:{accent:'#FF6B1A'}, features:{motionBlur:true,snapping:true,guides:true,autosave:true,adaptiveQuality:true}, layout:{docks:[dock('left',[p('assets',{flex:true})],250),dock('center',[p('viewer',{flex:true,instance:'viewer-main'}),p('timeline',{size:300,instance:'timeline-main'})]),dock('right',[p('inspector',{flex:true})],300)]}},
  { id:'animate', name:'Animate', builtin:true, density:'compact', theme:{accent:'#FF6B1A'}, features:{motionBlur:true,snapping:true,guides:false,autosave:true,adaptiveQuality:true,graphOnOpen:true}, layout:{docks:[dock('left',[p('layers',{flex:true}),p('takes',{size:160})],230),dock('center',[p('viewer',{size:300,instance:'viewer-main'}),p('timeline',{flex:true,instance:'timeline-main'})]),dock('right',[p('inspector',{flex:true})],320)]}},
  { id:'shaderlab', name:'Shader Lab', builtin:true, density:'compact', theme:{accent:'#4C8DFF'}, features:{motionBlur:false,snapping:true,guides:false,autosave:true,adaptiveQuality:true}, layout:{docks:[dock('left',[p('shader',{flex:true})],460),dock('center',[p('viewer',{flex:true,instance:'viewer-main'}),p('timeline',{size:200,instance:'timeline-main'})]),dock('right',[p('inspector',{flex:true}),p('perf',{size:190})],290)]}},
  { id:'edit', name:'Edit', builtin:true, density:'normal', theme:{accent:'#3FCF8E'}, features:{motionBlur:false,snapping:true,guides:true,autosave:true,adaptiveQuality:true}, layout:{docks:[dock('left',[p('assets',{flex:true}),p('layers',{size:240})],250),dock('center',[p('viewer',{flex:true,instance:'viewer-main'}),p('timeline',{size:360,instance:'timeline-main'})]),dock('right',[p('inspector',{flex:true})],280)]}},
  { id:'review', name:'Review', builtin:true, density:'comfy', theme:{accent:'#FF6B1A'}, features:{motionBlur:true,snapping:true,guides:false,autosave:true,adaptiveQuality:true}, layout:{docks:[dock('center',[p('viewer',{flex:true,instance:'viewer-main'}),p('timeline',{size:180,instance:'timeline-main'})]),dock('right',[p('chat',{flex:true}),p('notes',{size:170})],400)]}},
  { id:'focus', name:'Focus', builtin:true, density:'normal', theme:{accent:'#FF6B1A'}, features:{motionBlur:true,snapping:true,guides:false,autosave:true,adaptiveQuality:true}, layout:{docks:[dock('center',[p('viewer',{flex:true,instance:'viewer-main'})])]}},
]);

const WS = { current:null, all:[], previewing:null, list:()=>WS.all, get:(id)=>WS.all.find((w)=>w.id===id) };
PM.WS = WS;

const history = { stack:[], index:-1, max:80 };
function historyPush(label, before, after) {
  history.stack = history.stack.slice(0, history.index + 1);
  history.stack.push({ label, before:clone(before), after:clone(after) });
  if (history.stack.length > history.max) history.stack.shift();
  history.index = history.stack.length - 1; PM.bus.emit('workspace:history');
}
WS.canUndo = () => history.index >= 0;
WS.canRedo = () => history.index < history.stack.length - 1;
WS.undo = () => { if (!WS.canUndo() || WS.previewing) return false; const e=history.stack[history.index--]; installWorkspace(e.before,{save:true}); PM.toast('Workspace undo · '+e.label); PM.bus.emit('workspace:history'); return true; };
WS.redo = () => { if (!WS.canRedo() || WS.previewing) return false; const e=history.stack[++history.index]; installWorkspace(e.after,{save:true}); PM.toast('Workspace redo · '+e.label); PM.bus.emit('workspace:history'); return true; };

WS.init = () => {
  const saved=PM.store.get('workspaces',null);
  WS.all=(saved&&saved.length?saved:PRESETS()).map(Schema.normalizeWorkspace);
  PRESETS().map(Schema.normalizeWorkspace).forEach((preset)=>{ const i=WS.all.findIndex((w)=>w.id===preset.id); if(i<0)WS.all.push(preset); else if(WS.all[i].builtin)WS.all[i]=preset; });
  WS.save(false); const last=PM.store.get('workspace','design'); WS.activate(WS.get(last)?last:'design',true);
};
WS.validate = (workspace) => Schema.validate(
  Schema.normalizeWorkspace(workspace),
  [...Object.keys(PM.PANELS), ...(workspace.custom || []).map((section) => section.id)],
);
WS.activate = (id,silent) => { if(WS.previewing)WS.cancelPreview(true); const w=WS.get(id); if(!w)return false; WS.current=w; applyWorkspace(w); PM.store.set('workspace',id); PM.bus.emit('workspaces'); if(!silent)PM.toast('Workspace · '+w.name); return true; };

function applyWorkspace(w){ registerCustom(w); applyFeatures(w); PM.Layout.apply(w); }
function applyFeatures(w){ const f=w.features||{}; if(f.motionBlur!==undefined)PM.mblurOn=!!f.motionBlur; if(f.snapping!==undefined)PM.snap=!!f.snapping; if(f.guides!==undefined)PM.guides=!!f.guides; if(f.adaptiveQuality!==undefined)PM.perf.auto=!!f.adaptiveQuality; if(f.graphOnOpen!==undefined&&PM.TL)PM.TL.graph=!!f.graphOnOpen; PM.bus.emit('viewopts'); PM.invalidate(); }
WS.save = (emit=true) => { PM.store.set('workspaces',WS.all); if(emit)PM.bus.emit('workspaces'); };

function installWorkspace(input,opt={}){
  const w=Schema.normalizeWorkspace(input), validation=WS.validate(w); if(!validation.ok)throw new Error(validation.errors.join('\n'));
  const i=WS.all.findIndex((x)=>x.id===w.id); if(i<0)WS.all.push(w); else WS.all[i]=w; WS.current=w;
  if(opt.save!==false)WS.save(); applyWorkspace(w); PM.store.set('workspace',w.id); PM.bus.emit('workspaces'); return w;
}

WS.mutate = (fn,opt={}) => {
  if(WS.previewing)WS.cancelPreview(true); const before=clone(WS.current); let draft=clone(WS.current);
  if(draft.builtin&&!opt.inPlace){draft.id=PM.uid('ws');draft.name+=' (edited)';draft.builtin=false;}
  fn(draft); draft=Schema.normalizeWorkspace(draft); const valid=WS.validate(draft);
  if(!valid.ok){PM.toast('Workspace rejected · '+valid.errors[0],4200);return WS.current;}
  const after=installWorkspace(draft,{save:true}); historyPush(opt.label||'Edit interface',before,after); return after;
};
WS.create = (spec={}) => { const before=clone(WS.current), base=clone(WS.get(spec.base)||WS.get('design')); const draft=Schema.normalizeWorkspace({...base,...spec,id:PM.uid('ws'),builtin:false,name:spec.name||'Workspace'}); const made=installWorkspace(draft,{save:true}); historyPush('Create '+made.name,before,made); PM.toast('Workspace · '+made.name); return made; };
WS.replace = (manifest,opt={}) => { const draft=Schema.normalizeWorkspace({...manifest,id:opt.keepId===false?PM.uid('ws'):WS.current.id,builtin:false}); if(opt.preview)return WS.preview(draft); const before=clone(WS.current),after=installWorkspace(draft,{save:true}); historyPush(opt.label||'Generate interface',before,after); return after; };

WS.preview = (candidate) => { if(WS.previewing)WS.cancelPreview(true); const draft=Schema.normalizeWorkspace(candidate),v=WS.validate(draft); if(!v.ok)throw new Error(v.errors.join('\n')); WS.previewing={before:clone(WS.current),draft}; WS.current=draft; applyWorkspace(draft); requestAnimationFrame(showPreviewBar); PM.bus.emit('workspace:preview',true); return draft; };
WS.commitPreview = () => { if(!WS.previewing)return false; const {before,draft}=WS.previewing; WS.previewing=null; installWorkspace(draft,{save:true}); historyPush('Generate interface',before,draft); removePreviewBar(); PM.toast('Generated workspace kept'); PM.bus.emit('workspace:preview',false); return true; };
WS.cancelPreview = (silent) => { if(!WS.previewing)return false; const before=WS.previewing.before; WS.previewing=null; WS.current=WS.get(before.id)||before; applyWorkspace(WS.current); removePreviewBar(); if(!silent)PM.toast('Workspace preview reverted'); PM.bus.emit('workspace:preview',false); return true; };
function showPreviewBar(){ removePreviewBar(); if(!WS.previewing)return; const bar=h('div.workspace-preview-bar',h('span.preview-dot'),h('b','Generated workspace preview'),h('span','Try it before keeping it.'),h('span.sp'),h('button.chip',{onclick:()=>WS.cancelPreview()},'Revert'),h('button.chip.solid',{onclick:()=>WS.commitPreview()},'Keep workspace')); bar.id='workspace-preview-bar';document.body.appendChild(bar); }
function removePreviewBar(){document.getElementById('workspace-preview-bar')?.remove();}

WS.remove=(id)=>{const w=WS.get(id);if(!w||w.builtin)return PM.toast('Built-in workspaces can’t be deleted');const before=clone(WS.current);WS.all=WS.all.filter((x)=>x.id!==id);WS.save();if(WS.current.id===id)WS.activate('design');historyPush('Delete '+w.name,before,WS.current);};
WS.restoreDefault=()=>{const preset=PRESETS().map(Schema.normalizeWorkspace).find((w)=>w.id==='design'),before=clone(WS.current);installWorkspace(preset,{save:true});historyPush('Restore default workspace',before,preset);PM.toast('Default workspace restored');};
WS.saveAsNew=()=>{const input=h('input',{value:WS.current.name.replace(' (edited)','')+' copy'});PM.modal({title:'Save workspace',body:h('div.field',input),width:400,actions:[{label:'Cancel'},{label:'Save',pri:true,run:()=>{const draft=clone(WS.current),before=clone(WS.current);draft.id=PM.uid('ws');draft.name=input.value.trim()||'Workspace';draft.builtin=false;installWorkspace(draft,{save:true});historyPush('Save workspace copy',before,draft);}}]});setTimeout(()=>input.focus(),30);};
WS.editJSON=()=>{const ta=h('textarea.code',{style:{height:'460px',borderRadius:'8px'},spellcheck:'false'},JSON.stringify(WS.current,null,2));ta.addEventListener('keydown',(e)=>e.stopPropagation());PM.modal({title:'Workspace manifest',body:ta,width:760,actions:[{label:'Cancel'},{label:'Preview',run:()=>{try{const d=JSON.parse(ta.value);d.id=WS.current.id;d.builtin=false;WS.preview(d);}catch(e){PM.toast('Invalid workspace: '+e.message,4200);return false;}}},{label:'Apply',pri:true,run:()=>{try{const d=JSON.parse(ta.value);d.id=WS.current.id;d.builtin=false;WS.replace(d,{label:'Edit workspace manifest'});}catch(e){PM.toast('Invalid workspace: '+e.message,4200);return false;}}}]});};

/* Generated sections are assembled from approved components and safe bindings. */
function registerCustom(w){(w.custom||[]).forEach((section)=>PM.registerPanel(section.id,{title:section.title||'Generated section',size:section.size||220,generated:true,build(body){const wrap=h('div.generated-section');body.appendChild(wrap);if(section.note)wrap.appendChild(h('div.generated-note',section.note));(section.controls||section.components||[]).forEach((c)=>buildComponent(wrap,c));}}));}
function buildComponent(wrap,c){
  const type=c.type||'slider';
  if(type==='heading'){wrap.appendChild(h('div.generated-heading',c.label||c.text||'Controls'));return;}
  if(type==='text'||type==='note'){wrap.appendChild(h('div.generated-note',c.text||c.label||''));return;}
  if(type==='divider'){wrap.appendChild(h('div.generated-divider'));return;}
  if(type==='layerList'){buildLayerList(wrap);return;}
  if(type==='assetGrid'){buildAssetGrid(wrap);return;}
  if(type==='transport'){buildTransport(wrap);return;}
  if(type==='button'){
    /* Never render decorative or dead generated controls. */
    if(c.prompt)wrap.appendChild(h('button.chip.generated-button',{onclick:()=>PM.Agent.send(c.prompt)},c.label||'Ask assistant'));
    else if(c.cmd&&PM.commands?.[c.cmd])wrap.appendChild(h('button.chip.generated-button',{onclick:()=>PM.cmd(c.cmd)},c.label||PM.commands[c.cmd].label));
    return;
  }
  const binding=c.binding||c.bind||(c.param?`param:${c.param}`:`param:${c.label||'Control'}`),get=()=>readBinding(binding,c),set=(v)=>writeBinding(binding,v,c.label||'Generated control'); let field;
  if(type==='color')field=PM.colorField(get,set,{label:c.label}); else if(type==='toggle')field=PM.toggleField(get,set,{label:c.label}); else if(type==='select')field=PM.selectField(get,set,c.options||[],{label:c.label}); else if((type==='textField'||type==='textInput')&&PM.textField)field=PM.textField(get,set,{label:c.label}); else field=PM.numField(get,set,{label:c.label,min:c.min,max:c.max,step:c.step||.01,precision:c.precision??2,unit:c.unit});
  if(field?.sync&&PM.Inspector)PM.Inspector.syncs.push(field.sync);wrap.appendChild(PM.row(c.label||binding.split(':').pop(),field));
}
function ensureParam(name,c){const ps=PM.proj.params||(PM.proj.params={});if(!ps[name])ps[name]={name,label:c.label||name,control:c.type==='slider'?'num':c.type,value:c.def??0,min:c.min,max:c.max,options:c.options||[]};return ps[name];}
function readBinding(binding,c={}){const [scope,...rest]=String(binding||'').split(':'),key=rest.join(':');if(scope==='param')return ensureParam(key,c).value;if(scope==='project')return PM.proj[key];if(scope==='selection'){const layer=PM.firstSel();if(!layer)return c.def??0;if(key.startsWith('data.'))return layer.d[key.slice(5)];return layer.p[key]?PM.ev(layer,key,PM.time):layer[key];}return c.def??0;}
function writeBinding(binding,value,label){PM.hist.do(label,()=>{const [scope,...rest]=String(binding||'').split(':'),key=rest.join(':');if(scope==='param')ensureParam(key,{label}).value=value;else if(scope==='project'&&['name','w','h','fps','dur','bg','shutter'].includes(key))PM.proj[key]=value;else if(scope==='selection')PM.selLayers().filter((l)=>!l.lock).forEach((l)=>{if(key.startsWith('data.'))l.d[key.slice(5)]=value;else if(l.p[key])PM.setOrKey(l,key,value,PM.time);else if(['on','audio','solo','shy','blend'].includes(key))l[key]=value;});PM.touch();PM.bus.emit('project');PM.bus.emit('layers');PM.invalidate();});}
function buildLayerList(wrap){const list=h('div.generated-list');wrap.appendChild(list);const paint=()=>{list.textContent='';PM.proj.layers.forEach((l)=>list.appendChild(h('button.generated-list-row',{onclick:()=>PM.selectLayers(l.id)},h('span.generated-swatch',{style:{background:l.color}}),l.name)));};PM.bus.on('layers',paint);paint();}
function buildAssetGrid(wrap){const grid=h('div.generated-asset-grid');wrap.appendChild(grid);const paint=()=>{grid.textContent='';Object.values(PM.proj.assets||{}).forEach((a)=>grid.appendChild(h('button.generated-asset',{onclick:()=>PM.cmd('addFromAsset',a.id)},h('span',a.kind),h('b',a.name))));};PM.bus.on('assets',paint);paint();}
function buildTransport(wrap){wrap.appendChild(h('div.generated-transport',h('button.iconbtn',{onclick:()=>PM.step(-1)},PM.icon('left')),h('button.iconbtn',{onclick:()=>PM.toggle()},PM.icon(PM.playing?'pause':'play')),h('button.iconbtn',{onclick:()=>PM.step(1)},PM.icon('right'))));}
WS.readBinding=readBinding;WS.writeBinding=writeBinding;WS.registerCustom=registerCustom;
})();
