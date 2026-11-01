export const check=(x,message='invariant')=>{if(!x)throw Error(message)};
export const ops=[];
let id=0;
export function node(type,text='') {
  const n={id:++id,type,text,props:{},children:[],parent:null};
  ops.push(['create',n.id,type]);if(text)ops.push(['setText',n.id,text]);
  Object.assign(n,{
    get firstChild(){return n.children[0]??null},
    get nextSibling(){if(!n.parent)return null;return n.parent.children[n.parent.children.indexOf(n)+1]??null},
    appendChild(c){return n.insertBefore(c,null)},
    insertBefore(c,b){if(c.parent){const old=c.parent.children.indexOf(c);c.parent.children.splice(old,1)}c.parent=n;const i=b===null?n.children.length:n.children.indexOf(b);check(i>=0,'before id');n.children.splice(i,0,c);ops.push(['insertBefore',n.id,c.id,b?.id??null]);return c},
    remove(){if(n.parent)n.parent.removeChild(n)},
    removeChild(c){const i=n.children.indexOf(c);check(i>=0,'remove');n.children.splice(i,1);c.parent=null;ops.push(['remove',c.id]);return c},
    setAttribute(k,v){n.props[k]=v;ops.push(['setProps',n.id,{[k]:v}])},
    removeAttribute(k){delete n.props[k];ops.push(['setProps',n.id,{[k]:null}])},
    addEventListener(k){ops.push(['setListeners',n.id,{[k]:true}])},
    removeEventListener(k){ops.push(['setListeners',n.id,{[k]:false}])},
    localName:type, namespaceURI:'http://www.w3.org/1999/xhtml', nodeType:type==='#text'?3:1,
  });
  // Define accessors separately: Object.assign evaluates getters.
  Object.defineProperties(n,{
    parentNode:{get:()=>n.parent},childNodes:{get:()=>n.children},nodeName:{get:()=>type.toUpperCase()},firstChild:{get:()=>n.children[0]??null},
    nextSibling:{get:()=>n.parent?.children[n.parent.children.indexOf(n)+1]??null},
    data:{get:()=>n.text,set:v=>{n.text=v;ops.push(['setText',n.id,v])}},
    textContent:{get:()=>n.type==='#text'?n.text:n.children.map(c=>c.textContent).join(''),set:v=>{n.text=v;n.children=[];ops.push(['setText',n.id,v])}},
  });return n;
}
export const walk=(n)=>[n,...n.children.flatMap(walk)];
export function summary(kind,times,refresh,extra={}) {
  const sorted=times.toSorted((a,b)=>a-b);return {probe:'reconciler',kind,n:times.length,widgets:1000,p50_ms:sorted[Math.ceil(times.length*.5)-1],p95_ms:sorted[Math.ceil(times.length*.95)-1],provisional_budget_ms:4,fast_refresh_preserves_state:refresh,operation_names:[...new Set(ops.map(o=>o[0]))],...extra};
}
// V8 is deliberately Node-free. Fixtures run sync, with explicit flushes.
globalThis.performance={now:hostNow};
globalThis.console={log(){},warn(){},error(){},info(){}};
globalThis.setTimeout=()=>0;globalThis.clearTimeout=()=>{};
