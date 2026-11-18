import { CompatRealm } from '../../src';
import { FakeHost } from '../fake-host';
import { expect, test } from 'bun:test';
import { install } from '../../src/frozen/reference/legacy/core/anim';

test('effect channel headings prefer the instance name, then definition label, then id', () => {
  const PM: any = { Ease: {}, clamp: Math.min, effectDef: () => ({ label: 'Deep Glow' }) };
  install(PM);
  const effect: any = { id: 'fx1', type: 'deep-glow-pop.effect', on: { v: true, kf: [] }, p: { exposure: { v: 1, kf: [] } } };
  const groups = () => PM.allProps({ fx: [effect] }).map((row: any) => row.group);
  expect(groups()).toEqual(['Deep Glow', 'Deep Glow']);
  effect.name = 'My Glow';
  expect(groups()).toEqual(['My Glow', 'My Glow']);
  effect.name = '';
  PM.effectDef = () => undefined;
  expect(groups()).toEqual(['deep-glow-pop.effect', 'deep-glow-pop.effect']);
});

test('a registered extension supplies its display name through the real compatibility realm', () => {
  const host = new FakeHost();
  const realm = new CompatRealm({ id: 'effect-names', generation: 1, initial: host.publication(), host, installLegacy: install });
  try {
    realm.extension('synthetic-glow').api.effects.register({
      id: 'deep-glow-pop.effect', label: 'Deep Glow', group: 'Stylize', params: [], frag: 'o = src(v_uv);',
    });
    const rows = realm.PM.allProps({ fx: [{ id: 'instance', type: 'deep-glow-pop.effect', p: { exposure: { v: 1, kf: [] } } }] });
    expect(rows[0].group).toBe('Deep Glow');
  } finally {
    realm.dispose();
  }
});
