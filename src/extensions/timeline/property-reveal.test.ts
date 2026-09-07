import { describe, it, expect, vi } from 'vitest';
import { createPropertyReveal, revealedProperties } from './property-reveal';

describe('property disclosure shortcuts', () => {
  function setup() {
    const props = ['position.x','position.y','opacity','m.mask.x','m.mask.feather','fx.amount','c.gain','c.sourceTime'].map(key => ({ key, prop: { v: 1, kf: key === 'opacity' ? [{ i: 'k', t: 0 }] : [], expr: key === 'fx.amount' ? '1' : null } }));
    const L: any = { id:'a', fx:[{id:'fx',type:'blur'}], collapsed:true, reveal:null };
    const PM: any = { proj:{layers:[L]}, sel:{ keys:[],chan:'opacity' }, selLayers:()=>[L], allProps:()=>props,
      UIState:{getReveal:(l:any)=>l.reveal,setReveal:(l:any,k:any)=>l.reveal=k,getLayerCollapsed:(l:any)=>l.collapsed,setLayerCollapsed:(l:any,c:any)=>l.collapsed=c}, bus:{emit:vi.fn()},invalidate:vi.fn() };
    return {PM,L,props,press:createPropertyReveal(PM)};
  }
  it('isolates static properties and Shift adds and removes rows', () => {
    const {PM,L,props,press}=setup();
    press('p'); expect(revealedProperties(PM,L,props).map(p=>p.key)).toEqual(['position.x','position.y']);
    press('t',true); expect(L.reveal).toContain('opacity');
    press('p'); press('t',true); press('p',true);
    expect(L.reveal).toEqual(['opacity']);
  });
  it.each([['m','mm',['m.mask.x','m.mask.feather']],['e','ee',['fx.amount']],['r','rr',['c.sourceTime']],['s','ss',['opacity']],['l','ll',[]]])('double %s selects %s', (key,_mode,expected)=>{
    const {L,press}=setup(); press(key); press(key); expect(L.reveal).toEqual(expected);
  });
  it('M with no selection opens and closes every strip on consecutive taps',()=>{
    const {PM,L,press}=setup(); PM.selLayers=()=>[];
    press('m'); expect(L.collapsed).toBe(false); expect(L.reveal).toEqual(['*']);
    press('m'); expect(L.collapsed).toBe(true);
    press('m'); expect(L.collapsed).toBe(false);
  });
  it('single shortcut toggles closed when its rows are already shown',()=>{
    const {L,press}=setup(); L.collapsed=false;L.reveal=['position.x','position.y'];press('p');expect(L.collapsed).toBe(true);
  });
});
