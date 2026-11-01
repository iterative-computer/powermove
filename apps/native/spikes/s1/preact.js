import {h,render,options} from 'preact';
import {useState} from 'preact/hooks';
import '@prefresh/core';
import {node,ops,walk,check,summary} from './probe-common.js';
globalThis.document={createElement:type=>node(type),createElementNS:(_ns,type)=>node(type),createTextNode:text=>node('#text',text)};
options.debounceRendering=fn=>{globalThis.flushPreact=fn};
const root=node('root');let setter;
function Counter(){const [v,set]=useState(0);setter=set;return h('text',null,`count:${v}`)}
function Updated(){const [v,set]=useState(0);setter=set;return h('text',null,`updated:${v}`)}
globalThis.__PREFRESH__.register(Counter,'Counter');
const tree=()=>h('div',null,h(Counter),...Array.from({length:999},(_,i)=>h('button',{key:i,'data-value':i,onClick:()=>{}},`${i}`)));
render(tree(),root);setter(17);globalThis.flushPreact?.();
check(root.textContent.includes('count:17'),'preact state before refresh');
globalThis.__PREFRESH__.register(Updated,'Counter');
const runtime=globalThis.__PREFRESH__;
const pending=runtime.getPendingUpdates();runtime.flush();
for(const [prev,next] of pending){runtime.replaceComponent(prev,next,false);}
globalThis.flushPreact?.();
const refresh=root.textContent.includes('updated:17');check(refresh,'preact refresh');
const times=[];
for(let n=0;n<40;n++){const t=hostNow();render(h('div',null,h(Updated),...Array.from({length:999},(_,i)=>h('button',{key:999-i,'data-value':n+i,onClick:()=>{}},`${n+i}`))),root);times.push(hostNow()-t)}
check(walk(root).filter(n=>n.type==='button').length===999,'preact tree size '+walk(root).filter(n=>n.type==='button').length);
globalThis.probeResult=summary('preact',times,refresh,{bundle_semantics:'real preact + prefresh; synthetic host adapter'});
