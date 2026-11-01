(()=>{
  const check=x=>{if(!x)throw Error('cluster invariant')};
  const stacks=new Map(), get=k=>stacks.get(k)?.at(-1)?.value;
  function activate(factory){
    const staged=[]; const effects=[];
    const api={register:(k,value)=>staged.push({k,value}),
      storage:{...storage}, op:f=>effects.push(f)};
    try {factory(api)} catch(e){return false}
    for(const entry of staged){let list=stacks.get(entry.k);if(!list)stacks.set(entry.k,list=[]);list.push(entry)}
    storage=api.storage; for(const f of effects)f(); return staged;
  }
  const a=activate(api=>api.register('timeline',{version:1}));
  const held=get('timeline');check(held===get('timeline'));
  check(activate(api=>{api.register('timeline',{version:99});api.storage.counter=999;throw Error('bad activation')})===false);
  check(get('timeline')===held&&storage.counter===7);
  const b=activate(api=>{api.register('timeline',{version:2});api.op(()=>globalThis.committed=1)});
  check(get('timeline')!==held&&held.version===1&&committed===1);
  // Buried disposal leaves top unchanged; top disposal reveals previous provider.
  const top=get('timeline');stacks.get('timeline').splice(stacks.get('timeline').indexOf(a[0]),1);check(get('timeline')===top);
  stacks.get('timeline').pop();check(get('timeline')===undefined);
  const c=activate(api=>api.register('timeline',held));activate(api=>api.register('timeline',{version:3}));stacks.get('timeline').pop();check(get('timeline')===held);
  return true;
})()
