/* Powermove — recursive workspace renderer. */
(() => {
const PM=window.PM,h=PM.h,$=PM.$,Schema=window.PMWorkspaceSchema;
PM.PANELS={}; PM.panelInst={};
PM.registerPanel=(id,def)=>{PM.PANELS[id]=Object.assign({id,title:id},def);};
const L={ws:null,root:null}; PM.Layout=L;
const MIRRORABLE=new Set(['viewer','timeline']);

function sectionTitle(node){
  if(!node)return 'Section';
  if(node.type==='panel')return node.title||PM.PANELS[node.panel]?.title||node.panel;
  return sectionTitle(node.children?.[0]);
}

function makeGrip(){return h('span.grip',{title:'Drag to place · right-click for options'},PM.icon('grip'));}
function ensureGrip(header){if(header&&!header.querySelector('.grip'))header.insertBefore(makeGrip(),header.firstChild);}
L.ensureGrip=ensureGrip;

function primaryInstance(panelId){return Object.values(PM.panelInst).find((inst)=>inst.panelId===panelId&&inst.real&&inst.built&&(!L.requestedInstances||L.requestedInstances.has(inst.key)));}

function buildPanel(spec){
  const def=PM.PANELS[spec.panel]; if(!def)return missingPanel(spec);
  const key=spec.instance||spec.panel;
  let inst=PM.panelInst[key];
  const primary=primaryInstance(spec.panel);
  const mirror=!!(MIRRORABLE.has(spec.panel)&&primary&&primary.key!==key);
  if(inst?.el&&inst.built&&inst.mirror!==mirror){inst.el.remove();inst.built=false;}
  if(inst?.el&&inst.built&&(def.persist||inst.mirror)){
    inst.spec=spec; applyPanelSize(inst.el,spec,def); return inst.el;
  }
  inst=PM.panelInst[key]||(PM.panelInst[key]={key,panelId:spec.panel,def,cache:null});
  inst.def=def; inst.spec=spec; inst.mirror=mirror; inst.real=!mirror;
  const cls='.panel'+(def.flush?'.flush':'')+(def.noscroll?'.noscroll':'')+(spec.headless||def.headless?'.headless':'')+(mirror?'.linked-mirror-panel':'');
  const el=h('div'+cls,{id:key===spec.panel||!document.getElementById('panel-'+spec.panel)?'panel-'+spec.panel:'panel-'+key});
  el.dataset.panel=spec.panel;el.dataset.instance=key;
  const header=h('header'),title=h('span.ptitle',spec.title||def.title);header.append(title,h('span.sp'));el.appendChild(header);ensureGrip(header);
  const body=h('div.body');el.appendChild(body);
  inst.el=el;inst.body=body;inst.header=header;
  applyPanelSize(el,spec,def);
  el.style.minWidth=(spec.minWidth||def.minWidth||80)+'px';el.style.minHeight=(spec.minHeight||def.minHeight||56)+'px';
  try{
    if(mirror)buildMirror(body,spec.panel,primary);
    else def.build&&def.build(body,inst);
    inst.built=true;
  }catch(error){console.error('[panel] '+spec.panel,error);body.appendChild(h('div.empty','Panel error: '+error.message));}
  if(!mirror){try{def.header&&def.header(header,inst);}catch(error){}}
  ensureGrip(header);
  header.addEventListener('pointerdown',(event)=>{if(event.target.closest('button')||event.button!==0)return;startPanelDrag(event,spec,el);});
  header.addEventListener('dblclick',(event)=>{if(event.target.closest('button')||el.classList.contains('headless'))return;toggleCollapse(inst);});
  header.addEventListener('contextmenu',(event)=>{event.preventDefault();panelMenu(event,spec);});
  return el;
}

function missingPanel(spec){const el=h('div.panel.missing-section',h('header',h('span.ptitle','Missing section')),h('div.body.empty','“'+(spec.panel||spec.id)+'” is unavailable.'));el.dataset.instance=spec.instance||spec.panel;return el;}
function applyPanelSize(el,spec,def){el.style.flex=spec.flex||(!spec.size&&!def.size)?'1 1 auto':'0 0 '+(spec.size||def.size)+'px';}
function toggleCollapse(inst){const collapsed=inst.el.dataset.collapsed==='1';inst.el.dataset.collapsed=collapsed?'0':'1';inst.body.style.display=collapsed?'':'none';inst.el.classList.toggle('collapsed',!collapsed);PM.bus.emit('layout:applied');}

function buildMirror(body,panelId,primary){
  const canvas=h('canvas.linked-mirror'); body.appendChild(canvas);
  const paint=()=>requestAnimationFrame(()=>{
    const source=panelId==='viewer'?PM.GL?.canvas:PM.TL?.cv;if(!source||!source.width||!source.height||!canvas.isConnected)return;
    const rect=body.getBoundingClientRect(),dpr=Math.min(devicePixelRatio||1,2);canvas.width=Math.max(2,Math.round(rect.width*dpr));canvas.height=Math.max(2,Math.round(rect.height*dpr));
    const ctx=canvas.getContext('2d');ctx.clearRect(0,0,canvas.width,canvas.height);ctx.drawImage(source,0,0,canvas.width,canvas.height);
  });
  PM.bus.on(panelId==='viewer'?'overlay':'draw:timeline',paint);PM.bus.on('layout:applied',paint);paint();
  canvas.addEventListener('pointerdown',(event)=>{
    const rect=canvas.getBoundingClientRect(),u=(event.clientX-rect.left)/rect.width,v=(event.clientY-rect.top)/rect.height;
    if(panelId==='timeline'&&PM.TL){const x=u*PM.TL.w;if(x>=PM.TL.gut)PM.setTime((x-PM.TL.gut)/PM.TL.pps+PM.TL.scrollT);return;}
    if(panelId==='viewer'&&PM.GL){const layer=PM.GL.pick(u*PM.proj.w,v*PM.proj.h,PM.time);if(layer)PM.selectLayers(layer.id);}
  });
  body.appendChild(h('div.linked-badge','Linked '+(panelId==='viewer'?'viewer':'timeline')));
}

function buildNode(node,path=[]){
  if(!node)return h('div.workspace-empty','No section');
  if(node.type==='panel')return buildPanel(node);
  if(node.type==='tabs')return buildTabs(node,path);
  const direction=node.type==='stack'||node.direction==='column'?'column':'row';
  const container=h('div.workspace-split.is-'+direction);container.dataset.nodePath=path.join('.');
  const sizes=Schema.normalizeSizes(node.sizes,node.children.length);node.sizes=sizes;
  node.children.forEach((child,index)=>{
    const pane=h('div.workspace-pane');pane.style.flex=`${sizes[index]} 1 0`;pane.appendChild(buildNode(child,path.concat(index)));container.appendChild(pane);
    if(index<node.children.length-1)container.appendChild(buildSplitter(node,index,container,direction));
  });
  return container;
}

function buildTabs(node,path){
  if(!node.children.some((child)=>child.instance===node.active))node.active=node.children[0]?.instance;
  const wrap=h('div.workspace-tabs'),bar=h('div.workspace-tabbar'),body=h('div.workspace-tabbody');
  node.children.forEach((child)=>{
    const instance=child.instance||firstInstance(child),button=h('button.workspace-tab'+(instance===node.active?'.on':''),sectionTitle(child));
    button.onclick=()=>PM.WS.mutate((workspace)=>{const target=Schema.atPath(workspace.layout.root,path);if(target?.type==='tabs')target.active=instance;},{label:'Switch section tab',inPlace:true});
    bar.appendChild(button);
    if(instance===node.active)body.appendChild(buildNode(child,path.concat(node.children.indexOf(child))));
  });
  wrap.append(bar,body);return wrap;
}
function firstInstance(node){let result=null;Schema.walk(node,(child)=>{if(!result&&child.type==='panel')result=child.instance;});return result;}

function buildSplitter(node,index,container,direction){
  const splitter=h('div.workspace-splitter.'+(direction==='row'?'vertical':'horizontal'));
  splitter.addEventListener('pointerdown',(event)=>{
    const panes=[...container.children].filter((child)=>child.classList.contains('workspace-pane')),a=panes[index],b=panes[index+1],ra=a.getBoundingClientRect(),rb=b.getBoundingClientRect();
    const total=direction==='row'?ra.width+rb.width:ra.height+rb.height,startA=direction==='row'?ra.width:ra.height,start=node.sizes.slice();splitter.classList.add('drag');
    PM.drag(event,{cursor:direction==='row'?'col-resize':'row-resize',move:(dx,dy)=>{const delta=direction==='row'?dx:dy,minimum=70,nextA=PM.clamp(startA+delta,minimum,total-minimum),combined=start[index]+start[index+1];node.sizes[index]=combined*(nextA/total);node.sizes[index+1]=combined-node.sizes[index];a.style.flex=`${node.sizes[index]} 1 0`;b.style.flex=`${node.sizes[index+1]} 1 0`;PM.bus.emit('layout:applied');},up:()=>{splitter.classList.remove('drag');PM.WS.save();PM.bus.emit('layout:applied');}});
  });return splitter;
}

function parkPersistent(){
  const park=document.createElement('div');park.id='pm-persist-park';park.style.cssText='position:absolute;left:-9999px;top:0;width:1px;height:1px;overflow:hidden;';document.body.appendChild(park);
  Object.values(PM.panelInst).forEach((inst)=>{if((inst.def?.persist||inst.mirror)&&inst.el?.parentNode)park.appendChild(inst.el);});return park;
}

L.apply=(workspace)=>{
  L.ws=workspace;L.requestedInstances=new Set(Schema.listPanels(workspace).map((entry)=>entry.instance));const root=$('#body'),park=parkPersistent();root.textContent='';root.classList.add('workspace-root');
  const tree=buildNode(workspace.layout.root,[]);tree.classList.add('workspace-main');root.appendChild(tree);
  (workspace.layout.overlays||[]).forEach((item)=>root.appendChild(buildFloating(item,'overlay')));
  (workspace.layout.floating||[]).forEach((item)=>root.appendChild(buildFloating(item,'floating')));
  applyTheme(workspace.theme||{});document.documentElement.dataset.density=workspace.density||'normal';PM.bus.emit('layout');
  if(!park.childNodes.length)park.remove();
  requestAnimationFrame(()=>{PM.bus.emit('layout:applied');requestAnimationFrame(()=>{PM.bus.emit('layout:applied');PM.invalidate();});});
};

function buildFloating(item,mode){
  const shell=h('div.workspace-floating.'+mode);shell.dataset.floatId=item.id;shell.style.width=item.width+'px';shell.style.height=item.height+'px';positionFloating(shell,item);
  const content=buildNode(item.node,['@'+mode,item.id]);shell.appendChild(content);
  const handle=h('span.float-resize');shell.appendChild(handle);
  const header=content.matches?.('.panel')?content.querySelector(':scope > header'):content.querySelector?.('.panel > header');
  if(header)header.addEventListener('pointerdown',(event)=>{if(event.target.closest('button')||event.button!==0)return;const startX=item.x,startY=item.y;PM.drag(event,{cursor:'move',move:(dx,dy)=>{item.anchor='free';item.x=startX+dx;item.y=startY+dy;positionFloating(shell,item);},up:()=>PM.WS.save()});});
  handle.addEventListener('pointerdown',(event)=>{event.stopPropagation();const w=item.width,hgt=item.height;PM.drag(event,{cursor:'nwse-resize',move:(dx,dy)=>{item.width=PM.clamp(w+dx,140,innerWidth-20);item.height=PM.clamp(hgt+dy,90,innerHeight-20);shell.style.width=item.width+'px';shell.style.height=item.height+'px';PM.bus.emit('layout:applied');},up:()=>PM.WS.save()});});
  return shell;
}
function positionFloating(shell,item){
  shell.style.left=shell.style.right=shell.style.top=shell.style.bottom='auto';const pad=14;
  if(item.anchor==='top-right'){shell.style.right=(item.x??pad)+'px';shell.style.top=(item.y??pad)+'px';}
  else if(item.anchor==='bottom-left'){shell.style.left=(item.x??pad)+'px';shell.style.bottom=(item.y??pad)+'px';}
  else if(item.anchor==='bottom-right'){shell.style.right=(item.x??pad)+'px';shell.style.bottom=(item.y??pad)+'px';}
  else if(item.anchor==='center'){shell.style.left='50%';shell.style.top='50%';shell.style.transform='translate(-50%,-50%)';}
  else{shell.style.left=(item.x??pad)+'px';shell.style.top=(item.y??pad)+'px';shell.style.transform='';}
}

function applyTheme(theme){
  const style=document.documentElement.style,map={accent:'--accent',bg:'--bg-window',panel:'--bg-panel',text:'--tx',line:'--line',font:'--f-ui',mono:'--f-mono'};
  Object.keys(map).forEach((key)=>theme[key]?style.setProperty(map[key],theme[key]):style.removeProperty(map[key]));
  if(theme.accent){style.setProperty('--accent-dim',hexA(theme.accent,.16));style.setProperty('--accent-tx',theme.accent);}else{style.removeProperty('--accent-dim');style.removeProperty('--accent-tx');}
  if(theme.radius!=null){style.setProperty('--r-lg',theme.radius+'px');style.setProperty('--r-md',Math.max(2,theme.radius-3)+'px');style.setProperty('--r-sm',Math.max(2,theme.radius-5)+'px');}
}
function hexA(hex,alpha){const [r,g,b]=PM.hex2rgb(hex);return `rgba(${Math.round(r*255)},${Math.round(g*255)},${Math.round(b*255)},${alpha})`;}
L.applyTheme=applyTheme;

/* ── direct placement ─────────────────────────────────── */
function moveRelative(spec,target,where,duplicate=false){PM.WS.mutate((workspace)=>{workspace.layout=Schema.placePanel(workspace,{...spec,instance:spec.instance},{target,where,duplicate}).layout;},{label:(duplicate?'Duplicate ':'Move ')+(PM.PANELS[spec.panel]?.title||spec.panel)});}
function moveToRegion(spec,where){const target=Schema.listPanels(L.ws).find((entry)=>entry.region==='root'&&entry.instance!==spec.instance)?.instance;moveRelative(spec,target,where);}
function floatSection(spec,mode){PM.WS.mutate((workspace)=>{workspace.layout=Schema.placePanel(workspace,spec,{mode,width:mode==='overlay'?320:440,height:mode==='overlay'?220:360,anchor:mode==='overlay'?'top-right':'free',viewerOverlay:mode==='overlay'}).layout;},{label:(mode==='overlay'?'Overlay ':'Float ')+sectionTitle(spec)});}
function removeSection(instance){PM.WS.mutate((workspace)=>{workspace.layout=Schema.removeInstance(workspace,instance).workspace.layout;},{label:'Hide section'});}
function duplicateSection(spec,where='right'){const target=spec.instance;PM.WS.mutate((workspace)=>{workspace.layout=Schema.placePanel(workspace,{...spec,instance:spec.instance},{target,where,duplicate:true}).layout;},{label:'Duplicate '+sectionTitle(spec)});}

L.hasPanel=(workspace,id)=>Schema.listPanels(Schema.normalizeWorkspace(workspace)).some((entry)=>entry.panel===id);
L.findPanel=(workspace,id)=>Schema.listPanels(Schema.normalizeWorkspace(workspace)).find((entry)=>entry.panel===id||entry.instance===id)||null;
L.removePanel=(workspace,id)=>{const found=L.findPanel(workspace,id);if(found)workspace.layout=Schema.removeInstance(Schema.normalizeWorkspace(workspace),found.instance).workspace.layout;};
L.addPanel=(workspace,id,where='right')=>{const normalized=Schema.normalizeWorkspace(workspace),target=Schema.listPanels(normalized)[0]?.instance;workspace.layout=Schema.placePanel(normalized,Schema.panel(id,{instance:Schema.uniqueInstance(normalized,id)}),{target,where,duplicate:true}).layout;};

function panelMenu(event,spec){
  const others=Schema.listPanels(L.ws).filter((entry)=>entry.region==='root'&&entry.instance!==spec.instance);
  PM.menu(document.body,[
    {header:sectionTitle(spec)},
    {label:'Duplicate beside',run:()=>duplicateSection(spec,'right')},
    others.length?{label:'Add as tab with '+sectionTitle(others[0]),run:()=>moveRelative(spec,others[0].instance,'tab')}:null,
    {label:'Overlay workspace',run:()=>floatSection(spec,'overlay')},
    {label:'Float in workspace',run:()=>floatSection(spec,'floating')},
    !MIRRORABLE.has(spec.panel)?{label:'Pop out to native window',run:()=>PM.Popout.open(spec.instance)}:null,
    '-',{header:'Place at edge'},
    {label:'Left',run:()=>moveToRegion(spec,'left')},{label:'Right',run:()=>moveToRegion(spec,'right')},{label:'Top',run:()=>moveToRegion(spec,'top')},{label:'Bottom',run:()=>moveToRegion(spec,'bottom')},
    '-',{label:'Hide section',run:()=>removeSection(spec.instance)},
    '-',{header:'Add section'},
    ...Object.values(PM.PANELS).filter((def)=>def.id!=='toolbar').map((def)=>({label:def.title,run:()=>PM.WS.mutate((workspace)=>{const normalized=Schema.normalizeWorkspace(workspace),target=spec.instance;workspace.layout=Schema.placePanel(normalized,Schema.panel(def.id,{instance:Schema.uniqueInstance(normalized,def.id)}),{target,where:'right',duplicate:true}).layout;},{label:'Add '+def.title})})),
  ].filter(Boolean),{x:event.clientX,y:event.clientY});
}

let dragState=null;
function startPanelDrag(event,spec,el){
  const rect=el.getBoundingClientRect(),ghost=h('div.panel-ghost',sectionTitle(spec));ghost.style.width=Math.min(rect.width,360)+'px';ghost.style.height='44px';document.body.appendChild(ghost);dragState={spec,ghost,moved:false,target:null};
  PM.drag(event,{cursor:'grabbing',move:(dx,dy,ev)=>{if(!dragState.moved&&Math.hypot(dx,dy)<5)return;dragState.moved=true;ghost.classList.add('on');ghost.style.left=ev.clientX+12+'px';ghost.style.top=ev.clientY+12+'px';updateDropTarget(ev);},up:()=>{const state=dragState;dragState=null;ghost.remove();clearDropHints();if(state?.moved&&state.target)moveRelative(spec,state.target.instance,state.target.where);}});
}
function updateDropTarget(event){clearDropHints();dragState.target=null;const targetEl=document.elementFromPoint(event.clientX,event.clientY)?.closest?.('.panel');if(!targetEl||targetEl.dataset.instance===dragState.spec.instance)return;const rect=targetEl.getBoundingClientRect(),x=(event.clientX-rect.left)/rect.width,y=(event.clientY-rect.top)/rect.height;let where='tab';if(x<.24)where='left';else if(x>.76)where='right';else if(y<.24)where='top';else if(y>.76)where='bottom';targetEl.classList.add('drop-'+where);dragState.target={instance:targetEl.dataset.instance,where};}
function clearDropHints(){PM.$$('.drop-left,.drop-right,.drop-top,.drop-bottom,.drop-tab').forEach((el)=>el.classList.remove('drop-left','drop-right','drop-top','drop-bottom','drop-tab'));}

L.refresh=(id)=>{Object.values(PM.panelInst).filter((inst)=>inst.panelId===id&&inst.real&&inst.el?.isConnected).forEach((inst)=>{inst.body.textContent='';try{inst.def.build&&inst.def.build(inst.body,inst);}catch(error){console.error(error);}inst.def.header&&inst.def.header(inst.header,inst);ensureGrip(inst.header);});};

/* Native pop-outs remain available for ordinary panels. */
PM.Popout={wins:{},open(instance){const inst=PM.panelInst[instance];if(!inst||inst.mirror)return PM.toast('This linked section stays in the workspace');if(this.wins[instance]&&!this.wins[instance].closed){this.wins[instance].focus();return;}const win=window.open('','pm-popout-'+instance,'width=480,height=620,left=160,top=120');if(!win)return PM.toast('Pop-out blocked');this.wins[instance]=win;const setup=()=>{if(win.closed)return;const doc=win.document;if(!doc?.body){setTimeout(setup,30);return;}doc.title=sectionTitle(inst.spec)+' — Powermove';let css='html,body{height:100%;margin:0;overflow:hidden;background:var(--bg-panel)}';for(const sheet of document.styleSheets)try{for(const rule of sheet.cssRules)css+=rule.cssText+'\n';}catch(error){}const style=doc.createElement('style');style.textContent=css;doc.head.appendChild(style);doc.body.style.cssText='display:flex;flex-direction:column';const bar=h('div.pop-bar',h('span.pop-title',sectionTitle(inst.spec)),h('span.sp'),h('button.iconbtn',{onclick:()=>PM.Popout.dock(instance)},PM.icon('panelL')));doc.body.append(bar,inst.el);inst.el.classList.add('popped');const timer=setInterval(()=>{if(win.closed){clearInterval(timer);PM.Popout.reclaim(instance);}},350);};setup();},reclaim(instance){const inst=PM.panelInst[instance];delete this.wins[instance];inst?.el?.classList.remove('popped');PM.Layout.apply(PM.WS.current);},dock(instance){const win=this.wins[instance];delete this.wins[instance];PM.panelInst[instance]?.el?.classList.remove('popped');PM.Layout.apply(PM.WS.current);if(win&&!win.closed)win.close();}};
})();
