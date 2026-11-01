import {describe,test,expect} from 'bun:test';
import {Recorder} from '../core';
import {validateTrace} from '../validate';

describe('observational recorder',()=>{
  test('preserves return/error identities, nesting, event order and snapshots',()=>{
    const state={name:'before',depth:0};const recorder=new Recorder(()=>state,{synthetic:true});
    const sentinel={answer:42};const error=new Error('synthetic');
    expect(recorder.call('outer',[sentinel],()=>{
      state.depth++;
      recorder.point('event',{sentinel});
      let caught;try{recorder.call('throw',[],()=>{throw error;});}catch(actual){caught=actual;}expect(caught).toBe(error);
      state.name='after';return sentinel;
    })).toBe(sentinel);
    const text=recorder.finish();expect(validateTrace(text).calls).toBe(2);
    const rows=text.trim().split('\n').map(x=>JSON.parse(x));
    expect(rows.map(x=>x.type)).toEqual(['header','call','event','call','throw','return','state','end']);
    expect(rows[1].state.props.name).toBe('before');expect(rows[5].state.props.name).toBe('after');
    expect(rows[1].args.values[0].$id).toBe(rows[5].value.$id);
  });
  test('does not invoke accessors or thenables; records cyclic aliases and undefined distinctly',()=>{
    let reads=0;const input:any={undefined, get value(){reads++;throw new Error('getter');}};input.self=input;
    const recorder=new Recorder(()=>null,{});const promise=Promise.resolve(false);
    expect(recorder.call('promise',[input],()=>promise)).toBe(promise);expect(reads).toBe(0);
    const rows=recorder.finish().trim().split('\n').map(x=>JSON.parse(x));
    expect(rows[1].args.values[0].props.self.$ref).toBe(rows[1].args.values[0].$id);
    expect(rows[2].value.$type).toBe('promise');
  });
  test('serialization failure is visible and never replaces the actual result or throw',()=>{
    const recorder=new Recorder(()=>{throw new Error('observer');},{});const result={};
    expect(recorder.call('actual',[],()=>result)).toBe(result);
    expect(()=>validateTrace(recorder.finish())).toThrow('invalid');
  });
  test('serialized snapshots have data tokens, not invented live object identities',()=>{
    const recorder=new Recorder(()=>null,{});recorder.token('random');
    const encoded=recorder.value('{"id":"random","layers":[],"at":1700000000000}');
    expect(encoded).toEqual({$type:'json-string',value:{id:{$token:'id:1'},layers:[],at:{$volatile:'wall-clock-ms'}}});
  });
  test('golden event scope retains nested emissions and full mode retains background emissions',()=>{
    const capture=(eventScope:string)=>{const recorder=new Recorder(()=>null,{eventScope});recorder.point('event.kernel',{event:'outside'});recorder.call('mutation',[],()=>recorder.point('event.kernel',{event:'inside'}));return recorder.finish().trim().split('\n').map(x=>JSON.parse(x));};
    expect(capture('synchronous-calls').filter(x=>x.type==='event.kernel').length).toBe(1);
    expect(capture('all').filter(x=>x.type==='event.kernel').length).toBe(2);
  });
  test('deterministic logical ids, explicit special values and version rejection',()=>{
    const capture=()=>{const shared:any={};shared.cycle=shared;const r=new Recorder(()=>shared,{});r.token('random-id');r.call('sample',['random-id',undefined,-0,NaN,1n,Symbol('s')],()=>shared);return r.finish();};
    expect(capture()).toBe(capture());
    expect(()=>validateTrace(capture().replace('"version":1','"version":2'))).toThrow('Unsupported');
    expect(()=>validateTrace(capture().replace('"call":1','"call":5'))).toThrow('Broken');
  });
});
