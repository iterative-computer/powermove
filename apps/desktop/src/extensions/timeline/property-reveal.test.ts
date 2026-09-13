import { describe, it, expect, vi } from 'vitest';
import { createPropertyReveal, revealedProperties } from './property-reveal';
import { fakePowermoveAPI } from './fake-api.test-helper';

describe('property disclosure shortcuts', () => {
  function setup() {
    const props = ['position.x','position.y','opacity','m.mask.x','m.mask.feather','fx.amount','c.gain','c.sourceTime'].map(key => ({ key, label: key, group: '', prop: { v: 1, kf: [], expr: null } }));
    const layer: any = { id: 'l', type: 'solid', fx: [{ id: 'fx', type: 'missing' }], collapsed: true, reveal: null };
    const harness = fakePowermoveAPI(vi);
    harness.state.project.layers = [layer];
    harness.state.selection = { layers: [layer.id], keys: [], chan: 'opacity' };
    harness.api.anim.allProps = (() => props) as unknown as typeof harness.api.anim.allProps;
    harness.api.model.mkLayer = (() => ({})) as unknown as typeof harness.api.model.mkLayer;
    return { api: harness.api, state: harness.state, layer, props, press: createPropertyReveal(harness.api) };
  }

  it('isolates static properties and Shift adds and removes rows', () => {
    const { api, layer, props, press } = setup();
    press('p'); expect(revealedProperties(api, layer, props).map(p => p.key)).toEqual(['position.x','position.y']);
    press('t', true); expect(revealedProperties(api, layer, props).map(p => p.key)).toEqual(['position.x','position.y','opacity']);
    press('p', true); expect(revealedProperties(api, layer, props).map(p => p.key)).toEqual(['opacity']);
  });

  for (const [single, doubled, expected] of [
    ['m','mm',['m.mask.x','m.mask.feather']], ['e','ee',[]], ['r','rr',['c.sourceTime']],
    ['s','ss',['opacity']], ['l','ll',[]],
  ] as const) it(`double ${single} selects ${doubled}`, () => {
    const { layer, press } = setup(); press(single); press(single); expect(layer.reveal).toEqual(expected);
  });

  it('M with no selection opens and closes every strip on consecutive taps', () => {
    const { state, layer, press } = setup(); state.selection.layers = [];
    press('m'); expect(layer.collapsed).toBe(false); expect(layer.reveal).toEqual(['*']);
    press('m'); expect(layer.collapsed).toBe(true); expect(layer.reveal).toEqual(['*']);
  });

  it('single shortcut toggles closed when its rows are already shown', () => {
    const { layer, press } = setup(); layer.collapsed = false; layer.reveal = ['position.x','position.y']; press('p'); expect(layer.collapsed).toBe(true);
  });
});
