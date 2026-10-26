import { describe, expect, it } from 'vitest';
import { canonicalProject, canonicalStringify, jsonClone } from '../../../../shared/canonical-json';
import { projectTexts } from './project-text';

const now = async () => {};
const layer = (id: string, extra: object = {}) => ({ id, type: 'text', from: 0, dur: 2, anim: { x: [{ t: 0, v: 1 }, { t: 1, v: [2, -0] }] }, ...extra });

async function collect(pieces: AsyncIterable<string>) {
  const out: string[] = [];
  for await (const piece of pieces) out.push(piece);
  return out.join('');
}

async function check(project: any) {
  const texts = projectTexts('1.2.3', project, now);
  expect(texts).not.toBeNull();
  const [canonicalText, slot] = await Promise.all([collect(texts!.canonical), texts!.slot]);
  // The slot is JSON.stringify's own text, byte for byte.
  expect(slot).toBe(JSON.stringify({ v: '1.2.3', proj: project }));
  // The engine's copy is the canonical project form's JSON value.
  const canonical = JSON.parse(canonicalText);
  expect(canonicalStringify(canonical)).toBe(canonicalStringify(jsonClone(canonicalProject(project))));
  return { slot, canonical };
}

describe('projectTexts', () => {
  it('writes the slot and the canonical form of a project with an open composition and others', async () => {
    const project = {
      id: 'p', name: 'Ünïcode \u{1f600} "quoted"', w: 1920, h: 1080, fps: 30, dur: 4, bg: '#000', compId: 'main', compName: 'Main',
      layers: [layer('a'), layer('b', { text: 'lone \ud800 surrogate' }), undefined, null],
      comps: { pre: { id: 'pre', name: 'Pre', w: 100, h: 100, fps: 24, dur: 1, layers: [layer('c')] }, 10: { id: '10', layers: [] }, 2: { id: '2' } },
      assets: { m: { id: 'm', storageKey: 'media:v2:m', size: 12 } }, markers: [{ t: 1 }], work: [0, 4],
      skipped: undefined, fn: () => 1, nan: NaN, inf: -Infinity, nested: { deep: { deeper: [1, { x: new Number(3) }] } },
      revision: 7,
    };
    const { canonical } = await check(project);
    // The open composition moved into comps; every other member is shared text.
    expect(Object.keys(canonical.comps).sort()).toEqual(['10', '2', 'main', 'pre']);
    expect(canonical.comps.main.layers.map((l: any) => l?.id ?? l)).toEqual(['a', 'b', null, null]);
    expect(canonical.layers).toBeUndefined();
  });

  it('keeps a project without an open composition, or with comps that are not a record, as it is', async () => {
    await check({ id: 'p', w: 10, h: 10, layers: [layer('a')] });
    const odd = await check({ id: 'p', compId: 'main', comps: [1, 2], layers: [] });
    expect(odd.canonical).toEqual({ id: 'p', compId: 'main', comps: [1, 2], layers: [] });
    // A stale entry for the open composition is replaced, as canonicalProject does.
    await check({ id: 'p', compId: 'main', compName: 'M', layers: [layer('a')], comps: { main: { id: 'main', layers: [layer('old')] }, other: { id: 'other' } } });
    await check({ id: 'p', compId: 'main', compName: 'M', w: 5, layers: [], fpsRational: [30000, 1001], shutter: { angle: 180 } });
    await check({ id: 'p', compId: '', comps: {}, layers: [] });
  });

  it('keeps an own __proto__ member and integer-like keys in JSON.stringify order', async () => {
    const project = JSON.parse('{"id":"p","__proto__":{"a":1},"b":2,"1":3,"compId":"c","comps":{"__proto__":{"layers":[]},"x":{}},"layers":[]}');
    await check(project);
  });

  it('stays exact for members with boxed primitives and arrays with holes', async () => {
    // eslint-disable-next-line no-sparse-arrays
    await check({ id: 'p', compId: 'c', layers: [layer('a'), , layer('b')], name: new String('boxed'), flag: new Boolean(false), n: new Number(-0) });
  });

  it('refuses a toJSON near the root, where serializing a member alone would change its key', async () => {
    expect(projectTexts('1', { id: 'p', toJSON: () => ({}) }, now)).toBeNull();
    expect(projectTexts('1', { id: 'p', layers: [{ id: 'a', toJSON: (key: string) => key }] }, now)).toBeNull();
    expect(projectTexts('1', { id: 'p', comps: { toJSON: () => 1 } }, now)).toBeNull();
    expect(projectTexts('1', { id: 'p', comps: { c: { layers: [{ toJSON: () => 1 }] } } }, now)).toBeNull();
    // Deeper hooks are the native serializer's, with their own keys.
    await check({ id: 'p', layers: [{ id: 'a', when: { toJSON: (key: string) => `at ${key}` } }] });
  });

  it('yields between members on a large project, and hands out canonical text before the slot is made', async () => {
    let pauses = 0, release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const big = { id: 'p', compId: 'c', name: 'big', layers: Array.from({ length: 400 }, (_, i) => layer(`l${i}`, { keys: Array.from({ length: 400 }, (_, t) => ({ t, v: [t, t * 2] })) })) };
    // The first pause holds the serialization until the first piece was read.
    const texts = projectTexts('1', big, async () => { if (pauses++ === 0) await gate; });
    const reader = texts!.canonical[Symbol.asyncIterator]();
    let slotMade = false;
    void texts!.slot.then(() => { slotMade = true; });
    // Held at its first pause, in the middle of the layers: what is made so
    // far, layers included, is already handed out.
    const early: string[] = [];
    let pending = reader.next();
    for (;;) {
      const next = await Promise.race([pending, new Promise<null>(resolve => setTimeout(() => resolve(null), 20))]);
      if (!next) break;
      early.push(next.value as string);
      pending = reader.next();
    }
    expect(early[0]).toBe('{');
    expect(early.some(piece => piece.startsWith('{"id":"l1"'))).toBe(true);
    expect(slotMade).toBe(false);
    release();
    const rest: string[] = [...early];
    for (let piece = await pending; !piece.done; piece = await reader.next()) rest.push(piece.value);
    expect(await texts!.slot).toBe(JSON.stringify({ v: '1', proj: big }));
    expect(canonicalStringify(JSON.parse(rest.join('')))).toBe(canonicalStringify(canonicalProject(big)));
    expect(pauses).toBeGreaterThan(0);
  });

  it('ends the canonical text with the serialization\'s own error', async () => {
    const texts = projectTexts('1', { id: 'p', compId: 'c', big: 1n as any }, now);
    await expect(texts!.slot).rejects.toThrow(/BigInt/);
    await expect(collect(texts!.canonical)).rejects.toThrow(/BigInt/);
  });
});
