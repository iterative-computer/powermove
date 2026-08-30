// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { canPopoutPanel, installPanelPopouts } from './popout';

afterEach(() => {
  document.body.replaceChildren();
  document.head.querySelectorAll('[data-test-style]').forEach((node) => node.remove());
  vi.restoreAllMocks();
});

describe('panel pop-outs', () => {
  it('keeps live canvas surfaces docked', () => {
    expect(canPopoutPanel('notes')).toBe(true);
    expect(canPopoutPanel('viewer')).toBe(false);
    expect(canPopoutPanel('timeline')).toBe(false);
    expect(canPopoutPanel('toolbar')).toBe(false);
  });

  it('moves the authoritative panel node into a child and returns the same node', () => {
    const pool = document.createElement('div');
    pool.id = 'pm-panel-pool';
    const poolHost = document.createElement('div');
    poolHost.dataset.panelHost = 'notes';
    const panel = document.createElement('section');
    panel.id = 'panel-notes';
    panel.className = 'panel';
    poolHost.append(panel);
    pool.append(poolHost);
    document.body.append(pool);
    const style = document.createElement('style');
    style.dataset.testStyle = '1';
    style.textContent = '.panel{display:flex}';
    document.head.append(style);

    const childDocument = document.implementation.createHTMLDocument('');
    const listeners = new Map<string, EventListener>();
    const child: Record<string, any> = {
      document: childDocument,
      closed: false,
      focus: vi.fn(),
      addEventListener: vi.fn((type: string, listener: EventListener) => listeners.set(type, listener)),
      close: vi.fn(() => {
        child.closed = true;
        listeners.get('beforeunload')?.(new Event('beforeunload'));
      })
    };
    vi.spyOn(window, 'open').mockReturnValue(child as Window);
    const apply = vi.fn();
    const PM: any = {
      PANELS: { notes: { title: 'Notes' } },
      panelInst: { notes: { el: panel } },
      Layout: { ws: { layout: { docks: [] } }, apply },
      WS: { current: { layout: { docks: [] } } },
      closeMenus: vi.fn(),
      toast: vi.fn()
    };

    installPanelPopouts(PM);
    expect(PM.Popout.open('notes')).toBe(true);
    expect(childDocument.querySelector('.pop-mirror')?.firstElementChild).toBe(panel);
    expect(childDocument.title).toBe('Notes — Powermove');
    expect(PM.Popout.isOpen('notes')).toBe(true);
    expect(apply).toHaveBeenCalledOnce();

    childDocument.querySelector<HTMLButtonElement>('.pop-bar button')!.click();
    expect(poolHost.firstElementChild).toBe(panel);
    expect(panel.classList.contains('popped')).toBe(false);
    expect(PM.Popout.isOpen('notes')).toBe(false);
    expect(child.close).toHaveBeenCalledOnce();
    expect(apply).toHaveBeenCalledTimes(2);
  });
});
