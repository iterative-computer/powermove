// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { editCanvasText } from './canvas-text';
import { createKernel } from '../../renderer/src/kernel/registries';
import type { PowermoveAPI } from 'powermove';

afterEach(() => {
  document.body.replaceChildren();
  window.getSelection()?.removeAllRanges();
});

function textFixture() {
  const layer = {
    id: 'text-1',
    type: 'text',
    d: {
      text: 'Hello', font: 'Arial', weight: 500, size: 48,
      leading: 1.15, tracking: 0, color: '#ffffff', align: 'left'
    }
  };
  const api = {
    selection: { select: vi.fn() },
    transport: { time: () => 0, pause: vi.fn(), invalidate: vi.fn() },
    anim: { resolveContent: () => layer.d, worldMatrix: () => [1, 0, 0, 1, 0, 0] },
    render: { raster: () => null },
    edit: { begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn() },
    services: { get: () => null },
  } as unknown as PowermoveAPI;
  const V: any = { shown: 1, inner: document.body };
  return { layer, api, V };
}

describe('canvas text editing', () => {
  it('lets field-aware keyboard shortcuts reach the global dispatcher', () => {
    const { layer, api, V } = textFixture();
    const kernel = createKernel();
    const commands = vi.fn();
    const listener = kernel.installKeyListener((command) => void commands(command));
    kernel.bind('built-in', { key: 'cmd+v', command: 'contextPaste', inFields: true });
    kernel.bind('built-in', { key: 'v', command: 'toolSelect' });

    editCanvasText(api, V, layer);
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
    V.finishCanvasText?.();
  });

  it('saves an empty string for the browser placeholder after cutting all text', () => {
    const { layer, api, V } = textFixture();
    editCanvasText(api, V, layer);
    const editor = document.querySelector<HTMLElement>('.canvas-text-editor')!;
    // Chromium leaves a single BR in an emptied contenteditable.
    editor.innerHTML = '<br>';
    editor.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteByCut' }));
    expect(api.edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ path: 'c.text', value: '' }));
    // An intentional blank line is represented by two BRs and must survive.
    editor.innerHTML = '<br><br>';
    const lineBreaks = editor.innerText;
    editor.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertParagraph' }));
    expect(api.edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ path: 'c.text', value: lineBreaks }));
    V.finishCanvasText?.();
  });
});
