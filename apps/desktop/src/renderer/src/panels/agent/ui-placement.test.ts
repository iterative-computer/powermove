import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../layout/model';
import { isUIPlacementMessage, parseUIPlacement, splitUIPlacementText, uiPlacementInstructions, UI_PLACEMENT_PREFIX } from './ui-placement';
import { ghostRowCount, ghostSlotIndex } from './ui-placement-geometry';

const workspace: Workspace = { layout: { docks: [
  { id: 'center', panels: [{ id: 'viewer' }, { id: 'timeline' }] },
  { id: 'right', panels: [{ id: 'inspector' }] },
  { id: 'hidden', hidden: true, panels: [{ id: 'secret' }] }
] } };
const panel = { kind: 'panel', id: 'timeline', label: 'Timeline controls' } as const;
const dock = { kind: 'dock', id: 'right', beforePanelId: null, label: 'Easing controls' } as const;
const message = (value: unknown) => UI_PLACEMENT_PREFIX + JSON.stringify(value);

describe('early UI placement', () => {
  it('withholds every streamed marker prefix and extracts the finished target without hiding prose', () => {
    const raw = message(panel);
    for (let end = 1; end <= raw.length; end++) {
      expect(splitUIPlacementText(raw.slice(0, end), true).text).toBe('');
    }
    expect(splitUIPlacementText(raw)).toEqual({ text: '', messages: [raw] });
    expect(splitUIPlacementText(`Starting.\n${raw}\nContinuing.`)).toEqual({ text: 'Starting.\n\nContinuing.', messages: [raw] });
    expect(splitUIPlacementText('Please wait.', true).text).toBe('Please wait.');
    expect(splitUIPlacementText('P', true).text).toBe('');
    expect(splitUIPlacementText('P').text).toBe('P');
    expect(splitUIPlacementText(`Example: ${raw}`).text).toBe(`Example: ${raw}`);
  });

  it('handles newlines, quoted braces and malformed or interrupted metadata', () => {
    const raw = message({ ...panel, label: 'A } and "quoted" label' });
    expect(splitUIPlacementText(raw + '\nDone.').text).toBe('\nDone.');
    expect(splitUIPlacementText(raw.replace('TARGET ', 'TARGET\n')).messages).toEqual([raw]);
    expect(splitUIPlacementText(UI_PLACEMENT_PREFIX + '{"kind":"panel"').text).toBe('');
    expect(splitUIPlacementText(UI_PLACEMENT_PREFIX + 'bad\nStill working.').text).toBe('\nStill working.');
    expect(splitUIPlacementText(`${message(panel)}\n${message(dock)}`).messages).toEqual([message(panel), message(dock)]);
  });
  it('accepts existing panels and exact new-panel insertion points', () => {
    expect(parseUIPlacement(message(panel), workspace)).toEqual(panel);
    expect(parseUIPlacement(message(dock), workspace)).toEqual(dock);
    expect(parseUIPlacement(message({ ...dock, beforePanelId: 'inspector' }), workspace))
      .toEqual({ ...dock, beforePanelId: 'inspector' });
    expect(parseUIPlacement(message({ ...panel, selector: '.tl-transport' }), workspace))
      .toEqual({ ...panel, selector: '.tl-transport' });
    expect(parseUIPlacement(message({ ...panel, selector: '[data-section="easing"]', insert: 'after' }), workspace))
      .toEqual({ ...panel, selector: '[data-section="easing"]', insert: 'after' });
  });

  it.each([
    { ...panel, id: 'missing' }, { ...panel, id: 'secret' },
    { ...panel, id: 'timeline"] div' }, { ...panel, label: '' },
    { ...panel, label: 'x'.repeat(65) }, { ...panel, kind: 'window' },
    { ...dock, id: 'hidden' }, { ...dock, beforePanelId: 'viewer' },
    { ...dock, beforePanelId: {} }, { ...panel, selector: '' },
    { ...panel, selector: 'x'.repeat(161) }, { ...panel, insert: 'after' },
    { ...panel, selector: '.row', insert: 'inside' }, null, []
  ])('ignores invalid or unreachable targets: %j', value => {
    expect(parseUIPlacement(message(value), workspace)).toBeNull();
  });

  it('does not infer location from normal prose, embedded examples, or malformed messages', () => {
    expect(parseUIPlacement('I will edit the timeline', workspace)).toBeNull();
    expect(isUIPlacementMessage('Example: ' + message(panel))).toBe(false);
    expect(parseUIPlacement(UI_PLACEMENT_PREFIX + '{', workspace)).toBeNull();
    expect(parseUIPlacement(message(panel), undefined)).toBeNull();
    expect(parseUIPlacement(message({ ...panel, extra: 'x'.repeat(600) }), workspace)).toBeNull();
  });

  it('asks for a public placement before edits and includes the current layout', () => {
    const prompt = uiPlacementInstructions(workspace);
    expect(prompt).toContain('Before editing files or building controls');
    expect(prompt).toContain('standalone public commentary');
    expect(prompt).toContain("Inspect the panel's DOM or source early");
    expect(prompt).toContain('insert=before or insert=after');
    expect(prompt).toContain('scene/media-only');
    expect(prompt).toContain('Never restart or reopen');
    expect(prompt).toContain('"id":"timeline"');
    expect(prompt).not.toContain('"id":"secret"');
  });
});

describe('ghost placement geometry', () => {
  it('holds the slot the new panel was announced for', () => {
    const panels = ['inspector', 'library'];
    expect(ghostSlotIndex(dock, 'right', panels)).toBe(2);
    expect(ghostSlotIndex({ ...dock, beforePanelId: 'inspector' }, 'right', panels)).toBe(0);
    expect(ghostSlotIndex({ ...dock, beforePanelId: 'library' }, 'right', panels)).toBe(1);
  });
  it('claims no slot in another dock, for an existing panel, or without a placement', () => {
    expect(ghostSlotIndex(dock, 'center', ['viewer'])).toBeNull();
    expect(ghostSlotIndex(panel, 'right', ['timeline'])).toBeNull();
    expect(ghostSlotIndex(null, 'right', [])).toBeNull();
    // A panel that has since been popped out or closed sends the ghost to the end.
    expect(ghostSlotIndex({ ...dock, beforePanelId: 'inspector' }, 'right', ['library'])).toBe(1);
  });

  it('fills the space it was given with skeleton rows', () => {
    // 6px rows on an 11px rhythm: three fit in 40px, a fourth needs 57px.
    expect(ghostRowCount(40)).toBe(3);
    expect(ghostRowCount(56)).toBe(3);
    expect(ghostRowCount(57)).toBe(4);
    // A tall panel fills without an unbounded row count.
    expect(ghostRowCount(4000)).toBe(24);
    expect(ghostRowCount(0)).toBe(0);
    expect(ghostRowCount(NaN)).toBe(0);
  });
});
