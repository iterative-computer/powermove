// Synthetic activation host: model all revocable effect classes, with an
// explicit await. External I/O is represented by an append-only audit log.
// This is not the production timers/network/files implementation.
(()=>{
  const check=x=>{if(!x)throw Error('activation invariant')};
  const live={registrations:new Map(),events:new Map(),timers:new Map(),ui:new Map()};
  const audit=[],kernel=[],diskStorage={value:1};let active=null;
  async function activate(id,fn){
    const staged={id,storage:{...diskStorage},ops:[],effects:[],hot:{counter:active?.hot.counter??0}};
    const api={hot:staged.hot,storage:staged.storage,mutate:op=>staged.ops.push(op),
      register:(kind,key,value)=>staged.effects.push([kind,key,value]),
      externalIO:kind=>audit.push([id,kind])};
    try {await fn(api)}catch(error){return false}
    if(active)for(const [kind,key]of active.effects)live[kind].delete(key);
    for(const [kind,key,value]of staged.effects)live[kind].set(key,value);
    Object.assign(diskStorage,staged.storage);kernel.push(...staged.ops);active=staged;return true;
  }
  const effects=['registrations','events','timers','ui'];
  globalThis.effectDone=(async()=>{
    check(await activate(1,async api=>{for(const kind of effects)api.register(kind,'x',()=>1);api.hot.counter=17;api.storage.value=2;api.mutate('acknowledged');}));
    const previous=active;
    check(!await activate(2,async api=>{for(const kind of effects)api.register(kind,'x',()=>2);api.storage.value=99;api.mutate('uncommitted');api.externalIO('network');api.externalIO('file');await Promise.resolve();throw Error('failed activation')}));
    check(active===previous&&diskStorage.value===2&&kernel.join()==='acknowledged');
    check(effects.every(kind=>live[kind].get('x')()===1));check(audit.length===2);
    const externalUndo=()=>previous.hot.counter; // Retained history stays callable.
    check(await activate(3,async api=>{check(api.hot.counter===17);await Promise.resolve();for(const kind of effects)api.register(kind,'x',()=>3);api.mutate('new-generation');}));
    check(effects.every(kind=>live[kind].size===1&&live[kind].get('x')()===3));check(externalUndo()===17);
    globalThis.effectResult=JSON.stringify({probe:'activation-effects',async_staging:true,failed_activation_preserved_old_generation:true,revoked:effects,external_history_callable:true,irreversible_io_logged:2,scope:'synthetic host; actual I/O not exercised'});
  })();
  return 'pending';
})()
