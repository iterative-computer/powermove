import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../layout/model';
import { isUIPlacementMessage, parseUIPlacement, uiPlacementInstructions, UI_PLACEMENT_PREFIX } from './ui-placement';
import { ghostRect, ghostRowCount, ghostSlotIndex } from './ui-placement-geometry';

const workspace: Workspace = { layout: { docks: [
  { id: 'center', panels: [{ id: 'viewer' }, { id: 'timeline' }] },
  { id: 'right', panels: [{ id: 'inspector' }] },
  { id: 'hidden', hidden: true, panels: [{ id: 'secret' }] }
] } };
const panel = { kind: 'panel', id: 'timeline', label: 'Timeline controls' } as const;
const dock = { kind: 'dock', id: 'right', beforePanelId: null, label: 'Easing controls' } as const;
const message = (value: unknown) => UI_PLACEMENT_PREFIX + JSON.stringify(value);

describe('early UI placement', () => {
  it('accepts existing panels and exact new-panel insertion points', () => {
    expect(parseUIPlacement(message(panel), workspace)).toEqual(panel);
    expect(parseUIPlacement(message(dock), workspace)).toEqual(dock);
    expect(parseUIPlacement(message({ ...dock, beforePanelId: 'inspector' }), workspace))
      .toEqual({ ...dock, beforePanelId: 'inspector' });
  });

  it.each([
    { ...panel, id: 'missing' }, { ...panel, id: 'secret' },
    { ...panel, id: 'timeline"] div' }, { ...panel, label: '' },
    { ...panel, label: 'x'.repeat(65) }, { ...panel, kind: 'window' },
    { ...dock, id: 'hidden' }, { ...dock, beforePanelId: 'viewer' },
    { ...dock, beforePanelId: {} }, null, []
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
    expect(prompt).toContain('scene/media-only');
    expect(prompt).toContain('Never restart or reopen');
    expect(prompt).toContain('"id":"timeline"');
    expect(prompt).not.toContain('"id":"secret"');
  });
});

describe('ghost placement geometry', () => {
  const target = { left: 200, top: 50, width: 300, height: 500 };
  it('insets a live panel without changing its layout', () => {
    expect(ghostRect(target)).toEqual({ left: 205, top: 55, width: 290, height: 490 });
  });
  it('fits small areas and rejects hidden/non-finite geometry', () => {
    expect(ghostRect({ ...target, height: 45 })?.height).toBe(35);
    expect(ghostRect({ ...target, width: 0 })).toBeNull();
    expect(ghostRect({ ...target, top: NaN })).toBeNull();
  });
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
