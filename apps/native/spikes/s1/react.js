import React from 'react';
import Reconciler from 'react-reconciler';
import Refresh from 'react-refresh/runtime';
import {node,ops,walk,check,summary} from './probe-common.js';
let priority=32;const noop=()=>{};const context={};
const config={
  detachDeletedInstance:noop,supportsMutation:true,isPrimaryRenderer:true,supportsPersistence:false,supportsHydration:false,
  getRootHostContext:()=>context,getChildHostContext:()=>context,getPublicInstance:x=>x,
  prepareForCommit:()=>null,resetAfterCommit:noop,createInstance:(type,props)=>{const n=node(type);n.props=props;ops.push(['setProps',n.id,Object.fromEntries(Object.entries(props).filter(([k])=>k!=='children'))]);return n},
  appendInitialChild:(p,c)=>p.appendChild(c),finalizeInitialChildren:()=>false,
  shouldSetTextContent:()=>false,createTextInstance:text=>node('#text',text),
  appendChild:(p,c)=>p.appendChild(c),appendChildToContainer:(p,c)=>p.appendChild(c),
  insertBefore:(p,c,b)=>p.insertBefore(c,b),insertInContainerBefore:(p,c,b)=>p.insertBefore(c,b),
  removeChild:(p,c)=>p.removeChild(c),removeChildFromContainer:(p,c)=>p.removeChild(c),
  commitTextUpdate:(n,_old,text)=>{n.data=text},commitUpdate:(n,type,oldProps,props)=>{n.props=props;ops.push(['setProps',n.id,Object.fromEntries(Object.entries(props).filter(([k])=>k!=='children'))]);ops.push(['setListeners',n.id,{click:!!props.onClick}])},
  clearContainer:c=>{for(const n of [...c.children])c.removeChild(n)},resetTextContent:n=>{n.textContent=''},
  scheduleTimeout:setTimeout,cancelTimeout:clearTimeout,noTimeout:-1,
  getCurrentUpdatePriority:()=>priority,setCurrentUpdatePriority:p=>{priority=p},resolveUpdatePriority:()=>priority||32,
  maySuspendCommit:()=>false,preloadInstance:()=>true,startSuspendingCommit:noop,suspendInstance:noop,waitForCommitToBeReady:()=>null,
  NotPendingTransition:null,HostTransitionContext:React.createContext(null),resetFormInstance:noop,
  requestPostPaintCallback:noop,shouldAttemptEagerTransition:()=>false,trackSchedulerEvent:noop,resolveEventType:()=>null,resolveEventTimeStamp:()=>-1,
  bindToConsole:(_method,args)=>()=>{},supportsMicrotasks:true,scheduleMicrotask:fn=>Promise.resolve().then(fn),
};
Refresh.injectIntoGlobalHook(globalThis);
const r=Reconciler(config);r.injectIntoDevTools({bundleType:1,version:'19.3.1',rendererPackageName:'pm-s1-fixture'});
let setter;function Counter(){const [v,set]=React.useState(0);setter=set;return React.createElement('text',null,`count:${v}`)}
function Updated(){const [v,set]=React.useState(0);setter=set;return React.createElement('text',null,`updated:${v}`)}
Refresh.register(Counter,'Counter');
const root=node('root');let failure=null;const onError=e=>{failure=e};
const container=r.createContainer(root,0,null,false,null,'',onError,onError,onError,null);
const commit=el=>{r.updateContainerSync(el,container,null,null);r.flushSyncWork();if(failure)throw failure};
const tree=(C,rev)=>React.createElement('div',null,React.createElement(C),...Array.from({length:999},(_,i)=>React.createElement('button',{key:rev?999-i:i,'data-value':rev+i,onClick:()=>{}},`${rev+i}`)));
commit(tree(Counter,0));r.flushSyncFromReconciler(()=>setter(17));
check(root.textContent.includes('count:17'),'react state before refresh');Refresh.register(Updated,'Counter');Refresh.performReactRefresh();r.flushSyncWork();
const refresh=root.textContent.includes('updated:17');check(refresh,'react refresh');
const times=[];for(let n=1;n<=40;n++){const t=hostNow();commit(tree(Updated,n));if(n>10)times.push(hostNow()-t)}
check(walk(root).filter(n=>n.type==='button').length===999,'react tree size');
globalThis.probeResult=summary('react',times,refresh,{native_nodes:walk(root).length,bundle_semantics:'real react-reconciler + react-refresh; synthetic host adapter'});
