// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { installAgentShell } from './agent-shell';

type Shell = ReturnType<typeof installAgentShell>;
let shell: Shell | null = null;

function fixture() {
  const events: Array<[string, unknown]> = [];
  const subscriptions = new Map<string, Set<(value?: unknown) => void>>();
  const panel = document.createElement('div');
  panel.id = 'panel-agent';
  const body = document.createElement('div');
  body.className = 'body';
  const prompt = document.createElement('div');
  prompt.className = 'agent-inline-prompt';
  prompt.tabIndex = 0;
  body.append(prompt);
  panel.append(document.createElement('header'), body);
  const park = document.createElement('div');
  park.id = 'pm-panel-pool';
  const host = document.createElement('div');
  host.dataset.panelHost = 'agent';
  host.append(panel);
  park.append(host);
  const launcher = document.createElement('button');
  launcher.id = 'agent-launcher';
  launcher.getBoundingClientRect = () => ({ left: 300, top: 7, width: 240, height: 30, right: 540, bottom: 37, x: 300, y: 7, toJSON: () => ({}) });
  const outside = document.createElement('button');
  document.body.append(park, launcher, outside);
  const PM: Record<string, any> = {
    panelInst: { agent: { el: panel } },
    bus: {
      on: (name: string, listener: (value?: unknown) => void) => {
        const group = subscriptions.get(name) ?? new Set();
        group.add(listener);
        subscriptions.set(name, group);
        return () => group.delete(listener);
      },
      emit: (name: string, value?: unknown) => {
        events.push([name, value]);
        subscriptions.get(name)?.forEach((listener) => listener(value));
      }
    }
  };
  return { PM, panel, prompt, launcher, outside, events };
}

beforeEach(() => {
  document.body.innerHTML = '';
  vi.stubGlobal('innerWidth', 1200);
  vi.stubGlobal('innerHeight', 800);
});

afterEach(() => {
  shell?.destroy();
  shell = null;
  vi.unstubAllGlobals();
});

describe('agent popover', () => {
  it('hosts the one live agent panel and reclaims it after layout passes', async () => {
    const { PM, panel } = fixture();
    shell = installAgentShell(PM);
    await Promise.resolve();
    expect(document.querySelector('#agent-popover .agent-popover-panel-host > #panel-agent')).toBe(panel);
    document.querySelector<HTMLElement>('[data-panel-host="agent"]')!.append(panel);
    PM.bus.emit('layout:applied');
    expect(panel.closest('#agent-popover')).not.toBeNull();
    shell.destroy();
    shell = null;
    expect(panel.closest('#pm-panel-pool')).not.toBeNull();
  });

  it('opens centred under the launcher, focuses the composer and reports its state', async () => {
    const { PM, prompt, launcher, events } = fixture();
    shell = installAgentShell(PM);
    await Promise.resolve();
    const surface = document.getElementById('agent-popover')!;
    expect(surface.inert).toBe(true);
    shell.open();
    await Promise.resolve();
    expect(shell.isOpen()).toBe(true);
    expect(surface.inert).toBe(false);
    expect(surface.style.width).toBe('440px');
    expect(surface.style.left).toBe(`${420 - 220}px`);
    expect(surface.style.top).toBe('43px');
    expect(launcher.getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement).toBe(prompt);
    expect(events).toContainEqual(['agent:popover', true]);
  });

  it('keeps the popover inside narrow windows', async () => {
    vi.stubGlobal('innerWidth', 400);
    vi.stubGlobal('innerHeight', 300);
    const { PM } = fixture();
    shell = installAgentShell(PM);
    shell.open();
    const surface = document.getElementById('agent-popover')!;
    expect(surface.style.width).toBe('376px');
    expect(surface.style.left).toBe('12px');
    expect(surface.style.height).toBe(`${300 - 43 - 12}px`);
  });

  it('collapses into the launcher on send and pulses it when already closed', async () => {
    const { PM, events } = fixture();
    shell = installAgentShell(PM);
    shell.open();
    shell.collapse();
    expect(shell.isOpen()).toBe(false);
    expect(document.getElementById('agent-popover-root')!.dataset.collapsing).toBe('true');
    expect(events.filter(([name]) => name === 'agent:collapse')).toHaveLength(1);
    shell.collapse();
    expect(events.filter(([name]) => name === 'agent:collapse')).toHaveLength(2);
  });

  it('closes on Escape and on outside presses, but not on the launcher or its menus', async () => {
    const { PM, launcher, outside } = fixture();
    shell = installAgentShell(PM);
    shell.open();
    await Promise.resolve();
    launcher.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    expect(shell.isOpen()).toBe(true);
    const menu = document.createElement('div');
    menu.className = 'pm-menu';
    document.body.append(menu);
    menu.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    expect(shell.isOpen()).toBe(true);
    outside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    expect(shell.isOpen()).toBe(false);

    shell.open();
    await Promise.resolve();
    document.querySelector('.agent-inline-prompt')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(shell.isOpen()).toBe(false);
    expect(document.activeElement).toBe(launcher);
  });

  it('stands aside for a docked agent and reveals it instead of opening', async () => {
    const { PM, panel, prompt } = fixture();
    let docked = false;
    const dock = document.createElement('div');
    dock.className = 'dock';
    document.body.append(dock);
    Object.assign(PM, {
      WS: { current: { layout: { docks: [] } } },
      Layout: { findPanel: () => docked ? { dock: { id: 'right' }, spec: { id: 'agent' } } : null }
    });
    shell = installAgentShell(PM);
    await Promise.resolve();
    shell.open();
    expect(shell.isOpen()).toBe(true);
    docked = true;
    dock.append(panel);
    PM.bus.emit('layout:applied');
    expect(shell.isOpen()).toBe(false);
    expect(panel.parentElement).toBe(dock);
    expect(shell.isDocked()).toBe(true);
    shell.toggle();
    expect(shell.isOpen()).toBe(false);
    expect(document.activeElement).toBe(prompt);
    docked = false;
    document.querySelector<HTMLElement>('[data-panel-host="agent"]')!.append(panel);
    PM.bus.emit('layout:applied');
    expect(panel.closest('#agent-popover')).not.toBeNull();
  });
});
