// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { editCanvasText } from './canvas-text';
import { createKernel } from '../../renderer/src/kernel/registries';

afterEach(() => {
  document.body.replaceChildren();
  window.getSelection()?.removeAllRanges();
});

describe('canvas text keyboard handling', () => {
  it('lets field-aware keyboard shortcuts reach the global dispatcher', () => {
    const layer = {
      id: 'text-1',
      type: 'text',
      d: {
        text: 'Hello', font: 'Arial', weight: 500, size: 48,
        leading: 1.15, tracking: 0, color: '#ffffff', align: 'left'
      }
    };
    const PM: any = {
      time: 0,
      raster: () => null,
      worldMatrix: () => [1, 0, 0, 1, 0, 0],
      selectLayers: vi.fn(),
      pause: vi.fn(),
      invalidate: vi.fn(),
      Edit: { begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn() }
    };
    const V = { shown: 1, inner: document.body };
    const kernel = createKernel();
    const commands = vi.fn();
    const listener = kernel.installKeyListener((command) => void commands(command));
    kernel.bind('built-in', { key: 'cmd+v', command: 'contextPaste', inFields: true });
    kernel.bind('built-in', { key: 'v', command: 'toolSelect' });

    editCanvasText(PM, V, layer);
    const editor = document.querySelector<HTMLElement>('.canvas-text-editor')!;
    const paste = new KeyboardEvent('keydown', {
      key: 'v', metaKey: true, bubbles: true, cancelable: true
    });
    editor.dispatchEvent(paste);
    editor.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'v', bubbles: true, cancelable: true
    }));

    expect(commands).toHaveBeenCalledExactlyOnceWith('contextPaste');
    expect(paste.defaultPrevented).toBe(true);
    listener.dispose();
    PM.finishCanvasText?.();
  });
});
