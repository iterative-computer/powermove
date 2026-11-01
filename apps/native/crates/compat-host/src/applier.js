/* Minimal adapter fixture, not the complete pm-remote-dom protocol. No eval. */
(() => {
  'use strict';
  const {base, generation} = PM_HOST_CONFIG;
  const panels = new Map();
  let after = 0, stopped = false;
  function call(value) {
    const request = new XMLHttpRequest();
    request.open('GET', base + 'call?q=' + encodeURIComponent(JSON.stringify({...value, generation})), false);
    request.send();
    const reply = JSON.parse(request.responseText);
    if (!reply.ok) throw Error(reply.error);
    return reply.value;
  }
  Object.defineProperty(window, 'pmCompatHost', {value:Object.freeze({
    call:(panel,method,payload)=>call({kind:'call',panel,method,payload}),
    generation,
  })});
  function emit(kind, panel, payload) { return call({kind, panel, payload}); }
  function panel(id) { const p = panels.get(id); if (!p) throw Error('unknown panel'); return p; }
  function node(p, id) { const n = p.nodes.get(id); if (!n) throw Error('unknown node'); return n; }
  const tags = new Set(['section','div','span','p','h1','h2','h3','input','textarea','button','output','label','select','option','canvas','img','ul','li']);
  function attr(n, name, value) {
    if (name==='data-pm-panel' || name==='data-pm-node') throw Error('reserved host attribute');
    if (!/^(aria-[\w-]+|data-[\w-]+|class|style|id|title|type|placeholder|role|tabindex|disabled|width|height|value|src|spellcheck|autocomplete|autocorrect|autocapitalize)$/.test(name)) throw Error('unsupported attribute');
    if (value === null) n.removeAttribute(name); else n.setAttribute(name, String(value));
  }
  function batch(p, ops) {
    if (!Array.isArray(ops) || ops.length > 10000) throw Error('bad batch');
    for (const [op,id,...args] of ops) {
      if (op === 'create') {
        if (p.nodes.has(id) || !tags.has(args[0])) throw Error('bad create');
        const n = document.createElement(args[0]); n.dataset.pmNode = id; p.nodes.set(id,n);
      } else if (op === 'append') node(p,id).append(node(p,args[0]));
      else if (op === 'text') node(p,id).textContent = args[0];
      else if (op === 'attr') attr(node(p,id),args[0],args[1]);
      else if (op === 'value') node(p,id).value = args[0];
      else if (op === 'remove') {
        const n=node(p,id); if (id===0) throw Error('root removal');
        for (const child of [n,...n.querySelectorAll('[data-pm-node]')]) p.nodes.delete(Number(child.dataset.pmNode));
        n.remove();
      } else throw Error('unsupported mutation');
    }
    // A remote `style` on the root must not erase the host-owned geometry.
    place(p);
  }
  function place(p) {
    if (!p.slot) return;
    const b = p.slot.bounds;
    Object.assign(p.root.style,{left:b.x+'px',top:b.y+'px',width:b.w+'px',height:b.h+'px'});
  }
  function layout(p, slot) {
    p.slot = slot; p.root.hidden = !slot;
    for (const q of [...panels.values()].sort((a,b)=>(a.slot?.reading_order??1e9)-(b.slot?.reading_order??1e9) || Number(a.root.dataset.pmPanel)-Number(b.root.dataset.pmPanel))) document.body.append(q.root);
    place(p);
    // Composition clips the shared page; do not clip this root: fixed overlays
    // must escape its panel rectangle. The compositor owns native occlusion.
  }
  let pendingGeometry = false;
  function reportGeometry() {
    if (pendingGeometry || stopped) return;
    pendingGeometry = true;
    queueMicrotask(() => {
      pendingGeometry = false;
      for (const [id,p] of panels) if (p.slot) {
        const rects = [...p.root.querySelectorAll('[data-pm-overlay]')].filter(n=>n.getClientRects().length).map(n=>{
          const r=n.getBoundingClientRect();return {x:r.x,y:r.y,w:r.width,h:r.height};
        });
        emit('overlays',id,rects);
      }
    });
  }
  function apply(e) {
    if (e.kind === 'create') {
      if (panels.has(e.panel)) throw Error('duplicate panel');
      const root=document.createElement('section');root.dataset.pmPanel=e.panel;
      root.setAttribute('aria-label',e.label);root.hidden=true;root.tabIndex=-1;
      document.body.append(root);panels.set(e.panel,{root,nodes:new Map([[0,root]]),slot:null});
      observer.observe(root);
    } else if (e.kind === 'destroy') { const p=panel(e.panel);observer.unobserve(p.root);p.root.remove();panels.delete(e.panel); }
    else if (e.kind === 'slot') layout(panel(e.panel),e.slot);
    else if (e.kind === 'batch') batch(panel(e.panel),e.batch);
    else if (e.kind === 'focus') { const p=panel(e.panel);(e.node===null?p.root:node(p,e.node)).focus(); }
    else if (e.kind === 'event') panel(e.panel).root.dispatchEvent(new CustomEvent('pm-host-event',{detail:e.event}));
    else throw Error('unknown host event');
  }
  const observer=new ResizeObserver(reportGeometry);
  const mutations=new MutationObserver(reportGeometry);
  document.addEventListener('DOMContentLoaded',()=>mutations.observe(document.body,{subtree:true,attributes:true,childList:true,characterData:true}),{once:true});
  const owner = target => target.closest?.('[data-pm-panel]');
  function forward(e) {
    const root=owner(e.target); if (!root) return;
    emit('dom',Number(root.dataset.pmPanel),{type:e.type,node:Number(e.target.dataset.pmNode||0),value:e.target.value??null});
  }
  for (const type of ['input','change','click','focusin','focusout']) document.addEventListener(type,forward);
  // The bubble listener schedules its decision after every panel listener and
  // the remote keymap. preventDefault means consumed, independent of propagation.
  for (const type of ['keydown','keyup']) window.addEventListener(type,e=>queueMicrotask(()=>{
    if (e.defaultPrevented || e.isComposing) return;
    const root=owner(e.target);if (!root) return;
    const field=!!e.target.closest('input,textarea,select,[contenteditable="true"]');
    if (field && e.key.length===1 && !e.metaKey && !e.ctrlKey && !e.altKey) return;
    if ((e.metaKey||e.ctrlKey) && e.key.toLowerCase()==='c' && String(getSelection())) return;
    if (e.key==='Enter' && e.target.closest('button')) return;
    emit('key',Number(root.dataset.pmPanel),{type,key:e.key,code:e.code,meta:e.metaKey,ctrl:e.ctrlKey,alt:e.altKey,shift:e.shiftKey,repeat:e.repeat,field});
  }));
  document.addEventListener('click',e=>{
    if (!e.target.closest('[data-pm-popout]')) return;
    const child=window.open('about:blank','_blank','width=440,height=260');
    if (child) {
      child.document.write('<!doctype html><title>Compat popout</title><style>body{font:18px -apple-system;padding:24px}</style><h2>Compat popout</h2><input aria-label="Popout input" placeholder="Separate DOM document">');
      child.document.close();
      // The opener retains this object. This is a closure/reference smoke only;
      // the cloud bridge owns production child-document IDs and event plumbing.
      e.target.pmPopout=child;
    }
  });
  window.addEventListener('resize',reportGeometry);
  window.addEventListener('scroll',reportGeometry,true);
  async function poll() {
    try {
      while (!stopped) {
        const r=await fetch(base+'events?q='+after,{cache:'no-store'});
        const m=await r.json();
        if (m.generation!==generation) throw Error('stale generation');
        for (const e of m.events) apply(e);
        after=m.seq;reportGeometry();
      }
    } catch (_) { stopped=true;try { call({kind:'failure',panel:0,payload:'applier_stopped'}); } catch (_) {} }
  }
  document.addEventListener('DOMContentLoaded',()=>{
    call({kind:'ready',panel:0,payload:{origin:location.origin}});poll();
  },{once:true});
})();
