// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest';

import { clampPanelHeight, visibleDockPlan } from '../../layout/geometry';
import { hidePanel, movePanel, movePanelBy, restorePanel } from '../../layout/model';
import { makePM } from './make-pm';

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe('panel drag and layout behavior survivors', () => {
  it('creates missing side docks in stable order while preserving panel metadata', () => {
    const workspace: any = {
      layout: { docks: [{
        id: 'center',
        flex: true,
        panels: [{ id: 'viewer', flex: true }, { id: 'assets', size: 240 }],
      }] },
      hiddenPanels: [],
    };

    expect(movePanel(workspace, 'assets', 'left')).toBe(true);
    expect(workspace.layout.docks.map((dock: any) => dock.id)).toEqual(['left', 'center']);
    expect(workspace.layout.docks[0]!.panels[0]!.size).toBe(240);

    expect(movePanel(workspace, 'assets', 'right')).toBe(true);
    expect(workspace.layout.docks.map((dock: any) => dock.id)).toEqual(['left', 'center', 'right']);
    expect(workspace.layout.docks[2]!.panels[0]!.size).toBe(240);
  });

  it('omits empty side docks from the visible plan and fills the destination column', () => {
    const workspace: any = {
      layout: { docks: [{
        id: 'center',
        flex: true,
        panels: [{ id: 'viewer', flex: true }, { id: 'assets', size: 180 }],
      }] },
      hiddenPanels: [],
    };

    movePanel(workspace, 'assets', 'left');
    expect(visibleDockPlan(workspace).map((item: any) => item.dock.id)).toEqual([
      'left', 'center',
    ]);
    const left = workspace.layout.docks.find((dock: any) => dock.id === 'left');
    expect(left!.panels[0]!.flex).toBe(true);
    expect(left!.panels[0]!.size).toBe(180);

    movePanel(workspace, 'assets', 'right');
    expect(visibleDockPlan(workspace).map((item: any) => item.dock.id)).toEqual([
      'center', 'right',
    ]);
    const right = workspace.layout.docks.find((dock: any) => dock.id === 'right');
    expect(right!.panels[0]!.flex).toBe(true);
    expect(right!.panels[0]!.size).toBe(180);
  });

  it('reorders panels without changing their metadata and treats edges as no-ops', () => {
    const workspace: any = { layout: { docks: [{ id: 'center', panels: [
      { id: 'viewer', flex: true },
      { id: 'assets', size: 190 },
      { id: 'inspector', size: 260 },
    ] }] } };

    expect(movePanelBy(workspace, 'inspector', -1)).toBe(true);
    expect(workspace.layout.docks[0]!.panels.map((panel: any) => panel.id)).toEqual([
      'viewer', 'inspector', 'assets',
    ]);
    expect(workspace.layout.docks[0]!.panels[1]!.size).toBe(260);
    expect(movePanelBy(workspace, 'viewer', -1)).toBe(false);
  });

  it('clamps vertical panel resize geometry independently of sampling rate', () => {
    expect(clampPanelHeight(220, 40, 1, 600, 88, 100, 8)).toBe(260);
    expect(clampPanelHeight(220, -500, 1, 600, 88, 100, 8)).toBe(88);
    expect(clampPanelHeight(220, 800, 1, 600, 88, 100, 8)).toBe(492);
    expect(clampPanelHeight(300, 40, -1, 600, 88, 88, 8)).toBe(260);
  });

  it('hides and restores the last panel in a dock with its original slot metadata', () => {
    const workspace: any = { layout: { docks: [
      { id: 'left', size: 250, panels: [{ id: 'assets', size: 180 }] },
      { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
    ] }, hiddenPanels: [] as any[] };

    expect(hidePanel(workspace, 'assets')).toBe(true);
    expect(workspace.layout.docks[0]!.panels).toHaveLength(0);
    expect(workspace.hiddenPanels[0]!.dockId).toBe('left');
    expect(workspace.hiddenPanels[0]!.spec.size).toBe(180);

    expect(restorePanel(workspace, 'assets')).toBe(true);
    expect(workspace.layout.docks[0]!.panels[0]).toEqual({ id: 'assets', size: 180, flex: true });
    expect(workspace.hiddenPanels).toHaveLength(0);
  });

  it('keeps Composition visible and makes invalid hide or restore requests safe no-ops', () => {
    const workspace: any = {
      layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
      hiddenPanels: [],
    };

    expect(hidePanel(workspace, 'viewer')).toBe(false);
    expect(hidePanel(workspace, 'missing')).toBe(false);
    expect(restorePanel(workspace, 'missing')).toBe(false);
    expect(workspace.layout.docks[0]!.panels[0]!.id).toBe('viewer');
  });

  it('aborts a drag on native pointer cancellation without committing', () => {
    const PM = makePM('core/util');
    let commits = 0;
    let cancellations = 0;
    PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, {
      up: () => { commits++; },
      cancel: () => { cancellations++; },
    });

    window.dispatchEvent(new PointerEvent('pointercancel'));
    window.dispatchEvent(new PointerEvent('pointerup', { clientX: 30, clientY: 30 }));

    expect(cancellations).toBe(1);
    expect(commits).toBe(0);
  });

  it('keeps tracking a drag after capture loss and commits on release', () => {
    const addListener = vi.spyOn(window, 'addEventListener');
    const PM = makePM('core/util');
    const target = document.createElement('div') as HTMLElement & {
      setPointerCapture: (pointerId: number) => void;
      hasPointerCapture: (pointerId: number) => boolean;
      releasePointerCapture: (pointerId: number) => void;
    };
    target.setPointerCapture = vi.fn();
    target.hasPointerCapture = vi.fn(() => true);
    target.releasePointerCapture = vi.fn();
    let cancellations = 0;
    const move = vi.fn(), up = vi.fn();

    PM.drag({
      clientX: 5,
      clientY: 5,
      pointerId: 17,
      currentTarget: target,
      preventDefault() {},
    }, { move, up, cancel: () => { cancellations++; } });

    expect(target.setPointerCapture).toHaveBeenCalledWith(17);
    expect(addListener).toHaveBeenCalledWith('pointermove', expect.any(Function), true);
    target.dispatchEvent(new Event('lostpointercapture'));
    window.dispatchEvent(new PointerEvent('pointermove', { pointerId: 17, clientX: 25, clientY: 35 }));
    window.dispatchEvent(new PointerEvent('pointerup', { pointerId: 17, clientX: 25, clientY: 35 }));
    expect(cancellations).toBe(0);
    expect(move).toHaveBeenCalledWith(20, 30, expect.any(PointerEvent));
    expect(up).toHaveBeenCalledWith(20, 30, expect.any(PointerEvent));
    expect(up).toHaveBeenCalledOnce();
    expect(target.hasPointerCapture).toHaveBeenCalledWith(17);
    expect(target.releasePointerCapture).toHaveBeenCalledWith(17);
  });

  it('cancels a regular canvas drag when the window loses focus', () => {
    const PM = makePM('core/util');
    const cancel = vi.fn(), up = vi.fn();
    PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, { cancel, up, cursor: 'move' });
    window.dispatchEvent(new Event('blur'));
    window.dispatchEvent(new PointerEvent('pointerup', { clientX: 30, clientY: 30 }));
    expect(cancel).toHaveBeenCalledOnce();
    expect(up).not.toHaveBeenCalled();
    expect(document.body.style.cursor).toBe('');
  });
});
