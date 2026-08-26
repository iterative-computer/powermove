import { describe, expect, it } from 'vitest';

import { makePM } from './make-pm';

function workspacePM() {
  return makePM('core/capabilities', 'core/workspace');
}

function runtimePM() {
  const PM = workspacePM();
  PM.proj = {
    name: 'Test', w: 1920, h: 1080, fps: 30, dur: 10, shutter: .5, work: [0, 10],
    bg: '#111111',
    backgroundFill: { type: 'linear', angle: 15, stops: [
      { id: 'start', color: '#112233', position: 0 },
      { id: 'end', color: '#445566', position: 100 },
    ] },
  };
  PM.normalizeFill = (value: unknown) => JSON.parse(JSON.stringify(value));
  return PM;
}

const manifest = (custom: any[], panelId: string) => ({
  id: 'custom', name: 'Custom',
  layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: panelId }] }] },
  custom,
});

describe('workspace safety oracle survivors', () => {
  it('recognizes only the exact malformed app-only Gradient layout', () => {
    const WS = workspacePM().WS;
    const legacy = {
      id: 'saved-gradient', name: 'Gradient', builtin: false,
      layout: { docks: [
        { id: 'left', panels: [{ id: 'gradient-controls' }, { id: 'assets' }] },
        { id: 'right', panels: [{ id: 'viewer' }, { id: 'timeline' }, { id: 'layers' }, { id: 'inspector' }] },
      ] },
    };

    expect(WS.isLegacyGradient(legacy)).toBe(true);
    expect(WS.isLegacyGradient({ ...legacy, name: 'My Gradient' })).toBe(false);
    expect(WS.isLegacyGradient({
      ...legacy, layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
    })).toBe(false);
  });

  it('binds generated gradient controls to composition background source', () => {
    const PM = runtimePM();
    const binding = PM.WS.sourceBinding({
      type: 'color', label: 'End color', target: '$composition',
      path: 'composition.background.endColor', def: '#000000',
    });

    expect(binding.get()).toBe('#445566');
    const command = binding.command('#AABBCC');
    expect(command.type).toBe('set_composition');
    expect(command.patch.backgroundFill.stops[1].color).toBe('#AABBCC');
    expect(PM.proj.backgroundFill.stops[1].color).toBe('#445566');
  });

  it('binds composition dimensions and work-area controls to real source', () => {
    const PM = runtimePM();
    const width = PM.WS.sourceBinding({ type: 'slider', target: '$composition', path: 'composition.width', def: 16 });
    const height = PM.WS.sourceBinding({ type: 'slider', target: '$composition', path: 'composition.height', def: 16 });
    const start = PM.WS.sourceBinding({ type: 'slider', target: '$composition', path: 'composition.workArea.start', def: 0 });

    expect(width.get()).toBe(1920);
    expect(height.get()).toBe(1080);
    expect(width.command(2560).patch.width).toBe(2560);
    expect(height.command(1440).patch.height).toBe(1440);
    expect(start.command(2).patch.workArea).toEqual([2, 10]);
  });

  it('migrates the exact broken shortcut-only Layer Stagger panel', () => {
    const WS = workspacePM().WS;
    const controls = ['selectAll', 'deselect', 'prevEdge', 'nextEdge', 'prevFrame', 'nextFrame', 'split']
      .map((cmd, index) => ({ type: 'button', label: `Legacy ${index}`, cmd }));
    const workspace = WS.normalize(manifest([
      { id: 'layer-stagger-tools', title: 'Layer Stagger', controls },
    ], 'layer-stagger-tools'));
    const panel = workspace.custom[0];

    expect(panel.id).toBe('layer-stagger-tools');
    expect(panel.tool).toBe('layer-stagger');
    expect(panel.controls.map((control: any) => control.label)).toEqual([
      'Selection', 'Offset', 'Order', 'Anchor', 'Preview', 'Apply Stagger', 'Undo last edit',
    ]);
    expect(panel.controls.some((control: any) => control.cmd === 'split')).toBe(false);
    expect(panel.controls.find((control: any) => control.label === 'Apply Stagger').action.type).toBe('transform');
    expect(panel.note).not.toMatch(/split|frame navigation/i);
  });

  it('never rewrites unrelated custom command panels', () => {
    const workspace = workspacePM().WS.normalize(manifest([
      { id: 'my-tools', title: 'My Tools', controls: [{ type: 'button', label: 'Split', cmd: 'split' }] },
    ], 'my-tools'));

    expect(workspace.custom[0].tool).toBeUndefined();
    expect(workspace.custom[0].controls[0].cmd).toBe('split');
  });

  it('migrates the exact broken Text Splitter panel to Decompose Text', () => {
    const workspace = workspacePM().WS.normalize(manifest([
      { id: 'text-splitter', title: 'Text Splitter', controls: [
        { type: 'text', label: 'text', def: 'Powermove' },
        { type: 'button', label: 'Duplicate source layer', cmd: 'duplicate' },
        { type: 'button', label: 'Add text layer', cmd: 'addText' },
        { type: 'button', label: 'Undo last split step', cmd: 'undo' },
      ] },
    ], 'text-splitter'));
    const panel = workspace.custom[0];

    expect({ id: panel.id, tool: panel.tool, title: panel.title }).toEqual({
      id: 'text-splitter', tool: 'decompose-text', title: 'Text Splitter',
    });
    expect(panel.controls.map((control: any) => control.label)).toEqual([
      'Selection', 'Split into', 'Original', 'Preview', 'Decompose', 'Undo last edit',
    ]);
    expect(panel.controls.find((control: any) => control.label === 'Decompose').action.type).toBe('script');
    expect(panel.controls.some((control: any) => control.cmd === 'duplicate')).toBe(false);
  });

  it('clamps and limits Timeline interface manifests to visual configuration', () => {
    const WS = workspacePM().WS;
    const edit = WS.sanitizeInterfaceEdit(JSON.stringify({ target: 'timeline', patch: {
      rowHeight: 12, gutterWidth: 900, rulerHeight: 30, clipRadius: 5, keyframeSize: 20,
      showLayerNumbers: false, showTypeBadges: false, toolbarDensity: 'compact', projectLayers: [],
    } }));

    expect(edit.patch).toEqual({
      rowHeight: 22, gutterWidth: 360, rulerHeight: 30, clipRadius: 5, keyframeSize: 12,
      showLayerNumbers: false, showTypeBadges: false, toolbarDensity: 'compact',
    });
    const workspace = WS.normalize({
      id: 'w', name: 'W', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
    });
    expect(WS.applyInterfaceEdit(workspace, edit)).toBe(true);
    expect(workspace.chrome.timeline.rowHeight).toBe(22);
    expect(workspace.chrome.timeline.gutterWidth).toBe(360);
    expect(workspace.chrome.timeline).not.toHaveProperty('projectLayers');
  });
});
