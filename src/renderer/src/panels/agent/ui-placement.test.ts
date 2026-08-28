import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../layout/model';
import { isUIPlacementMessage, parseUIPlacement, uiPlacementInstructions, UI_PLACEMENT_PREFIX } from './ui-placement';
import { ghostRect } from './ui-placement-geometry';

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
  it('insets a panel without changing its layout', () => {
    expect(ghostRect(panel, target)).toEqual({ left: 205, top: 55, width: 290, height: 490 });
  });
  it('shows new panels at the dock end or just before the announced panel', () => {
    expect(ghostRect(dock, target)).toEqual({ left: 205, top: 397, width: 290, height: 148 });
    expect(ghostRect(dock, target, { ...target, top: 230 })).toEqual({ left: 205, top: 230, width: 290, height: 148 });
  });
  it('fits small areas and rejects hidden/non-finite geometry', () => {
    expect(ghostRect(dock, { ...target, height: 45 })?.height).toBe(35);
    expect(ghostRect(panel, { ...target, width: 0 })).toBeNull();
    expect(ghostRect(panel, { ...target, top: NaN })).toBeNull();
  });
});
