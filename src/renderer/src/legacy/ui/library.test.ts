import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './library';

describe('legacy library UI install', () => {
  it('exposes the modal and workspace editor contracts without opening DOM UI', () => {
    const subscriptions: string[] = [];
    const PM: PMRegistry = {
      h() {},
      bus: { on(event: string) { subscriptions.push(event); } },
    };

    install(PM);

    expect(PM.LibraryUI.isOpen).toBe(false);
    expect(typeof PM.LibraryUI.open).toBe('function');
    expect(typeof PM.LibraryUI.close).toBe('function');
    expect(typeof PM.WorkspaceEditor.show).toBe('function');
    expect(subscriptions).toEqual(['layout:applied', 'workspaces', 'project']);
  });
});
