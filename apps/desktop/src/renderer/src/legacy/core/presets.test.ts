import { describe, expect, it, vi } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { install } from './presets';
import { parseAgentEditCommand, parseEditCommand, ValidationError } from '../../core/validate/commands';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing', 'core/presets');
  PM.proj = PM.mkProject({fps: 30}); PM.time = 5;
  const source = PM.mkLayer('solid'), target = PM.mkLayer('solid'); source.from = 2; target.from = 7;
  PM.proj.layers = [source, target]; PM.selectLayers(source.id);
  const definitions: Record<string, any> = { blur: {params: [{k: 'amount', def: 0}, {k: 'quality', def: 'high'}]}, tint: {params: [{k: 'color', def: '#ffffff'}]} };
  PM.effectDef = (fx: any) => definitions[fx.type];
  PM.mkEffect = (type: string) => definitions[type] ? {id: PM.uid('f'), type, version: 1, on: true,
    p: Object.fromEntries(definitions[type].params.map((p: any) => [p.k, PM.P(p.def)]))} : null;
  const blur = PM.mkEffect('blur'), tint = PM.mkEffect('tint');
  blur.p.amount = PM.P(24); tint.p.color = PM.P('#ff6020'); source.fx = [blur, tint];
  return {PM, source, target, blur, tint, definitions};
}
describe('user animation presets', () => {
  it('persists selected effects in stack order, snapshots evaluated values, and supports list/rename/delete', () => {
    const {PM, source, blur, tint} = editor(), events = vi.fn(); PM.bus.on('presets', events);
    PM.animate(source, 'rotation', [{t: 0, v: 0}, {t: 6, v: 60}]);
    const preset = PM.Presets.save({name: 'Look', effectIds: [tint.id, blur.id], propertyPaths: ['rotation']});
    expect(preset.effects.map((fx: any) => fx.type)).toEqual(['blur', 'tint']);
    expect(preset.effects[0].parameters.amount).toEqual({v: 24, kf: []});
    expect(preset.properties[0].channel.v).toBeCloseTo(PM.ev(source, 'rotation', 5));
    blur.p.amount.v = 99; preset.effects[0].parameters.amount.v = 300;
    install(PM); expect(PM.Presets.get(preset.id).effects[0].parameters.amount.v).toBe(24);
    expect(PM.Presets.rename(preset.id, ' New name ')).toBe(true);
    expect(PM.Presets.list()[0].name).toBe('New name');
    PM.Presets.list()[0].name = 'Detached'; expect(PM.Presets.get(preset.id).name).toBe('New name');
    expect(PM.Presets.remove(preset.id)).toBe(true); expect(PM.Presets.list()).toEqual([]);
    expect(PM.Presets.remove(preset.id)).toBe(false); expect(PM.Presets.rename('missing', 'No')).toBe(false);
    expect(events).toHaveBeenCalledTimes(3);
  });
  it('restores relative keys, native easing, enabled animation, and fresh identities in one undo step', () => {
    const {PM, source, target, blur} = editor();
    blur.p.amount.kf = [PM.KF(2, 4, 'glide'), PM.KF(4, 40, 'glide')];
    blur.on = PM.P(true, {kf: [PM.KF(2, false), PM.KF(4, true)]});
    PM.animate(source, 'rotation', [{t: 2, v: 10}, {t: 4, v: 50}]);
    const preset = PM.Presets.save({name: 'Animated', effectIds: [blur.id], propertyPaths: ['rotation'], includeAnimation: true});
    expect(preset.effects[0].parameters.amount.kf.map((key: any) => key.t)).toEqual([-1, 1]);
    expect(preset.effects[0].parameters.amount.kf[0].i).toBeUndefined();
    PM.time = 10;
    const result = PM.Presets.apply(preset.id, target.id); expect(result.ok).toBe(true);
    const fx = target.fx[0];
    expect(fx.p.amount.kf.map((key: any) => key.t)).toEqual([2, 4]);
    expect(fx.p.amount.kf[0].outEase).toEqual(blur.p.amount.kf[0].outEase);
    expect(fx.p.amount.kf[0].i).not.toBe(blur.p.amount.kf[0].i);
    expect(fx.on.kf.map((key: any) => [key.t,key.v])).toEqual([[2,false],[4,true]]);
    expect(target.p.rotation.kf.map((key: any) => key.t)).toEqual([2,4]);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(target.id).fx).toEqual([]);
    expect(PM.L(target.id).p.rotation.kf).toEqual([]); expect(PM.hist.redo()).toBe(true);
    expect(PM.L(target.id).fx).toHaveLength(1);
  });
  it('keeps defaults for newly added params and reports missing effects/removed params', () => {
    const {PM, target, definitions} = editor(), preset = PM.Presets.save({name: 'Evolved'});
    definitions.blur.params = [{k: 'amount', def: 0}, {k: 'newParam', def: 123}]; delete definitions.tint;
    const result = PM.Presets.apply(preset.id, target.id);
    expect(result.ok).toBe(true); expect(target.fx).toHaveLength(1);
    expect(target.fx[0].p.amount.v).toBe(24); expect(target.fx[0].p.newParam.v).toBe(123);
    expect(result.data.results[0].data.skipped).toEqual(['Parameter unavailable: blur.quality', 'Effect unavailable: tint']);
    delete definitions.blur; expect(PM.Presets.apply(preset.id, target.id).ok).toBe(false);
    expect(target.fx).toHaveLength(1);
  });
  it('merges animated properties, retains keys before layer start, and applies static-only captures', () => {
    const {PM, source, target} = editor();
    PM.animate(source, 'rotation', [{t: 1, v: 10}, {t: 4, v: 40}]);
    target.p.rotation.kf = [PM.KF(20, 80)];
    const preset = PM.Presets.save({name: 'Motion', effectIds: [], propertyPaths: ['rotation'], includeAnimation: true});
    expect(PM.Presets.apply(preset.id, target.id, 7).ok).toBe(true);
    expect(target.p.rotation.kf.map((key: any) => key.t)).toEqual([-2, 1, 20]);
    expect(PM.Presets.save({name: 'Static', effectIds: [], propertyPaths: ['rotation']}).properties[0].channel.kf).toEqual([]);
  });
  it('applies static presets to already animated properties at the playhead', () => {
    const {PM, target} = editor();
    const preset = PM.Presets.save({name:'Static rotation', effectIds:[], propertyPaths:['rotation']});
    target.p.rotation.kf = [PM.KF(0,10), PM.KF(10,90)];
    expect(PM.Presets.apply(preset.id,target.id,10).ok).toBe(true);
    expect(PM.ev(target,'rotation',10)).toBe(0);
    expect(target.p.rotation.kf.map((key: any) => key.t)).toEqual([0,3,10]);
  });
  it('honors layer/group locks and rolls back earlier effects if a property is protected', () => {
    const {PM, target} = editor(), preset = PM.Presets.save({name: 'Protected', propertyPaths: ['rotation']});
    target.lock = true; expect(PM.Presets.apply(preset.id, target.id).ok).toBe(false); expect(target.fx).toEqual([]);
    target.lock = false; target.locked_intent = {rotation: {by: 'human'}};
    expect(PM.Presets.apply(preset.id, target.id).ok).toBe(false); expect(PM.L(target.id).fx).toEqual([]);
  });
  it('skips properties missing on the target and rejects empty/missing source captures', () => {
    const {PM, source, target} = editor(); source.type = 'text'; source.d.size = PM.P(12);
    expect(() => PM.Presets.save({name: ''})).toThrow();
    expect(() => PM.Presets.save({name: 'Empty', effectIds: []})).toThrow();
    expect(() => PM.Presets.save({name: 'Missing', effectIds: ['bad']})).toThrow();
    expect(() => PM.Presets.save({name: 'Bad path', propertyPaths: ['__proto__.x']})).toThrow();
    expect(PM.Presets.apply('missing', target.id).ok).toBe(false);
    const preset = PM.Presets.save({name: 'Portable', propertyPaths: ['c.size']});
    const result = PM.Presets.apply(preset.id, target.id);
    expect(result.ok).toBe(true); expect(result.data.results[0].data.skipped).toContain('Property unavailable: c.size');
  });
  it('validates the apply_commands entry point and never exposes a lock override', () => {
    expect(parseAgentEditCommand({type:'apply_preset',preset:'preset-1',target:'layer-1',time:10})).toEqual({type:'apply_preset',preset:'preset-1',target:'layer-1',time:10});
    expect(parseEditCommand({type:'apply_preset',preset:0})).toBeInstanceOf(ValidationError);
    expect(parseEditCommand({type:'apply_preset',preset:'p',time:NaN})).toBeInstanceOf(ValidationError);
    expect(parseAgentEditCommand({type:'apply_preset',preset:'p',overrideLock:true})).not.toHaveProperty('overrideLock');
  });
});
