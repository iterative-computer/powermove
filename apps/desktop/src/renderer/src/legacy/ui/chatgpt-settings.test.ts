// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { ChatGPTAccountStatus, PowermoveBridge } from '../../../../shared/ipc';
import { createChatGPTSettingsControl, formatChatGPTPlan } from './chatgpt-settings';

const disconnected: ChatGPTAccountStatus = {
  state: 'disconnected', email: null, planType: null, detail: null
};

describe('ChatGPT settings control', () => {
  it('shows subscription identity and cleans up its status listener', async () => {
    const listeners: Array<(status: ChatGPTAccountStatus) => void> = [];
    const stop = vi.fn();
    const api = {
      status: vi.fn(async () => disconnected),
      models: vi.fn(async () => []),
      connect: vi.fn(async () => ({
        state: 'connecting' as const,
        email: null,
        planType: null,
        detail: 'Finish signing in in your browser.'
      })),
      disconnect: vi.fn(async () => disconnected),
      onChanged: vi.fn((listener: (status: ChatGPTAccountStatus) => void) => {
        listeners.push(listener);
        return stop;
      })
    } satisfies PowermoveBridge['chatgpt'];

    const control = createChatGPTSettingsControl(api);
    await vi.waitFor(() => expect(control.element.querySelector('button')?.textContent).toBe('Connect'));
    control.element.querySelector('button')?.click();
    await vi.waitFor(() => expect(api.connect).toHaveBeenCalledOnce());
    expect(control.element.querySelector('button')?.textContent).toBe('Waiting…');

    listeners[0]?.({ state: 'connected', email: 'editor@example.com', planType: 'pro', detail: null });
    expect(control.element.textContent).toContain('editor@example.com · Pro plan');
    expect(control.element.querySelector('button')?.textContent).toBe('Disconnect');
    control.element.querySelector('button')?.click();
    await vi.waitFor(() => expect(api.disconnect).toHaveBeenCalledOnce());
    expect(control.element.querySelector('button')?.textContent).toBe('Connect');

    control.destroy();
    expect(stop).toHaveBeenCalledOnce();
  });

  it('formats provider plan identifiers for display', () => {
    expect(formatChatGPTPlan('team_workspace')).toBe('Team Workspace plan');
    expect(formatChatGPTPlan(null)).toBeNull();
  });

  it('keeps an already-open renderer usable until its native bridge restarts', () => {
    const control = createChatGPTSettingsControl(undefined);
    expect(control.element.textContent).toContain('Restart Powermove once');
    expect(control.element.querySelector('button')?.textContent).toBe('Restart needed');
    expect(control.element.querySelector('button')?.disabled).toBe(true);
    control.destroy();
  });
});
