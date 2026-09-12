import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { splitTextLayers } from './split-text';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Split text', w: 1000, h: 600, dur: 5 });
  PM.time = 0;
  const source = PM.mkLayer('text', { name: 'Headline', d: { text: 'Hello world', size: 50 } }, PM.proj);
  PM.proj.layers = [source];
  PM.ProjectIndex.invalidate();
  PM.selectLayers(source.id);
  PM.hist.clear();
  PM.textLayout = () => ({
    characters: [
      { text: 'H', x: 0, y: 0, sourceStart: 0, index: 0, word: 0, line: 0 },
      { text: 'w', x: 120, y: 0, sourceStart: 6, index: 1, word: 1, line: 0 },
    ],
    words: [
      { text: 'Hello', x: 0, y: 0, sourceStart: 0, index: 0, line: 0 },
      { text: 'world', x: 120, y: 0, sourceStart: 6, index: 1, line: 0 },
    ],
    lines: [{ text: 'Hello world', x: 0, y: 0, sourceStart: 0, index: 0, line: 0 }],
  });
  PM.raster = () => ({ fontOffset: { x: 0, y: 0 } });
  return { PM, source };
}

describe('split text layers', () => {
  it('puts the split layers and recoverable source in one selected group, with one-step undo and redo', () => {
    const { PM, source } = editor();
    const before = JSON.stringify(PM.proj.layers);

    expect(splitTextLayers(PM, [source.id], 'words')).toBeTruthy();

    const group = PM.firstSel();
    expect(group).toMatchObject({ type: 'group', name: 'Headline', collapsed: false });
    const members = PM.proj.layers.filter((layer: any) => layer.group === group.id);
    expect(members.map((layer: any) => layer.d.text)).toEqual(['Hello', 'world', 'Hello world']);
    expect(members.at(-1)).toBe(source);
    expect(source.on.v).toBe(false);

    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.proj.layers.find((layer: any) => layer.type === 'group')).toMatchObject({ name: 'Headline' });
  });
});
