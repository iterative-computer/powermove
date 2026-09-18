// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PowermoveAPI, TextCaretLayout } from 'powermove';

import { beginTextEdit } from './text-editor';
import { createKernel } from '../../renderer/src/kernel/registries';

afterEach(() => {
  document.body.replaceChildren();
});

/* 10px per character, one line, 48px type: geometry the tests can predict. */
function fixture(text = 'Hello', extra: Partial<{ name: string; lock: boolean }> = {}) {
  const layer: any = {
    id: 'text-1', type: 'text', name: extra.name ?? 'Title', lock: extra.lock ?? false,
    d: { text, font: 'Arial', weight: 500, size: 48, leading: 1.15, tracking: 0, color: '#ffffff', align: 'left', boxWidth: 0, boxHeight: 0 },
  };
  const layers = new Map<string, any>([[layer.id, layer]]);
  const dispatched: any[] = [];
  const api = {
    selection: { select: vi.fn() },
    transport: { time: () => 0, pause: vi.fn(), invalidate: vi.fn() },
    anim: { resolveContent: (target: any) => target.d, worldMatrix: () => [1, 0, 0, 1, 100, 200] },
    model: { layer: (id: string) => layers.get(id) ?? null },
    render: {
      raster: () => null,
      textLayout: (target: any): TextCaretLayout => {
        const value = String(target.d.text ?? '');
        return {
          size: 48, lineHeight: 55.2, length: value.length, align: 'left', boxWidth: 0, boxHeight: 0,
          lines: [{ text: value, start: 0, x: 0, y: 0, baseline: 39.36, width: value.length * 10, boundaries: Array.from({ length: value.length + 1 }, (_, k) => ({ index: k, x: k * 10 })) }],
        };
      },
    },
    edit: {
      begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(),
      dispatch: vi.fn((command: any) => {
        dispatched.push(command);
        if (command.type === 'set_property' && command.path === 'c.text') layer.d.text = command.value;
        if (command.type === 'set_layer') Object.assign(layer, command.patch);
        if (command.type === 'delete_layers') for (const id of command.targets) layers.delete(id);
        return { ok: true };
      }),
    },
    services: { get: vi.fn(() => null) },
  } as unknown as PowermoveAPI;
  const V: any = { shown: 1, inner: document.body, requestOverlay: vi.fn() };
  return { layer, api, V, dispatched };
}

const field = () => document.querySelector<HTMLTextAreaElement>('textarea.canvas-text-input')!;
const type = (value: string) => { const el = field(); el.value = value; el.dispatchEvent(new InputEvent('input', { bubbles: true })); };
const key = (init: KeyboardEventInit) => { const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }); field().dispatchEvent(event); return event; };

describe('text editing session', () => {
  it('opens one transaction, writes live and commits on Escape without losing text', () => {
    const { api, V, layer } = fixture();
    const session = beginTextEdit(api, V, layer)!;
    expect(api.edit.begin).toHaveBeenCalledWith('Edit text', { origin: 'canvas' });
    expect(V.canvasTextEditing).toBe('text-1');
    expect(document.activeElement).toBe(field());
    type('Hello world');
    expect(layer.d.text).toBe('Hello world');
    expect(api.edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ type: 'set_property', path: 'c.text', value: 'Hello world' }));
    const escape = key({ key: 'Escape' });
    expect(escape.defaultPrevented).toBe(true);
    expect(api.edit.commit).toHaveBeenCalledWith('Edit text');
    expect(api.edit.cancel).not.toHaveBeenCalled();
    expect(layer.d.text).toBe('Hello world');
    expect(V.canvasTextEditing).toBeNull();
    expect(V.finishCanvasText).toBeNull();
    expect(document.querySelector('.canvas-text-input')).toBeNull();
    session.finish(); // idempotent
    expect(api.edit.commit).toHaveBeenCalledTimes(1);
  });

  it('commits on Command+Enter and lets other shortcuts bubble to the field-aware keymap', () => {
    const { api, V, layer } = fixture();
    const kernel = createKernel();
    const commands = vi.fn();
    const listener = kernel.installKeyListener((command) => void commands(command));
    kernel.bind('built-in', { key: 'cmd+v', command: 'contextPaste', inFields: true });
    kernel.bind('built-in', { key: 'v', command: 'toolSelect' });
    beginTextEdit(api, V, layer);
    const paste = key({ key: 'v', metaKey: true });
    key({ key: 'v' });
    expect(commands).toHaveBeenCalledExactlyOnceWith('contextPaste');
    expect(paste.defaultPrevented).toBe(true);
    key({ key: 'Enter', metaKey: true });
    expect(api.edit.commit).toHaveBeenCalledWith('Edit text');
    listener.dispose();
  });

  it('removes a fresh layer with no history when it ends empty, and names a kept one after its text', () => {
    const empty = fixture('', { name: 'Text' });
    beginTextEdit(empty.api, empty.V, empty.layer, { fresh: true });
    expect(empty.api.edit.begin).not.toHaveBeenCalled();
    type('   ');
    empty.V.finishCanvasText();
    expect(empty.api.edit.cancel).toHaveBeenCalledTimes(1);
    expect(empty.api.edit.commit).not.toHaveBeenCalled();

    const kept = fixture('', { name: 'Text' });
    beginTextEdit(kept.api, kept.V, kept.layer, { fresh: true });
    type('  Launch day \nsecond line');
    kept.V.finishCanvasText();
    expect(kept.dispatched.at(-1)).toEqual({ type: 'set_layer', target: 'text-1', patch: { name: 'Launch day' } });
    expect(kept.api.edit.commit).toHaveBeenCalledWith('Add text');
  });

  it('deletes an existing layer whose text was emptied', () => {
    const { api, V, layer, dispatched } = fixture('Hello');
    beginTextEdit(api, V, layer);
    type('');
    V.finishCanvasText();
    expect(dispatched.at(-1)).toEqual({ type: 'delete_layers', targets: ['text-1'] });
    expect(api.edit.commit).toHaveBeenCalledWith('Delete empty text');
  });

  it('returns the Text tool to Select when the session ends', () => {
    const { api, V, layer } = fixture();
    const tools = { tool: 'text', setTool: vi.fn() };
    (api.services.get as any).mockImplementation((name: string) => name === 'tool' ? tools : null);
    beginTextEdit(api, V, layer);
    V.finishCanvasText();
    expect(tools.setTool).toHaveBeenCalledWith('select');
  });

  it('places the caret from a click, selects words on double click and drags a range', () => {
    const { api, V, layer } = fixture('Hello brave world');
    const drags: any[] = [];
    const session = beginTextEdit(api, V, layer, { caretAt: { x: 100 + 31, y: 200 + 20 }, drag: (_e, o) => drags.push(o) })!;
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 3, end: 3 });
    expect(session.contains({ x: 100 + 50, y: 200 + 10 })).toBe(true);
    expect(session.contains({ x: 100 + 400, y: 200 + 10 })).toBe(false);
    const down = (x: number, detail = 1, shiftKey = false) => session.pointerDown(new MouseEvent('pointerdown', { detail, shiftKey, cancelable: true }) as PointerEvent, { x: 100 + x, y: 200 + 20 });
    expect(down(75, 2)).toBe(true);
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 6, end: 11 });
    expect(down(20)).toBe(true);
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 2, end: 2 });
    drags.at(-1).move(0, 0, { clientX: 100 + 90, clientY: 220 });
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 2, end: 9 });
    expect(down(140, 1, true)).toBe(true);
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 2, end: 14 });
    expect(down(600)).toBe(false);
    expect(V.canvasTextEditing).toBe('text-1');
  });

  it('selects everything on entry when asked and mirrors external text changes', () => {
    const { api, V, layer } = fixture('Hello');
    const session = beginTextEdit(api, V, layer, { selectAll: true })!;
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 0, end: 5 });
    // The overlay draw is the sync point for changes made elsewhere (undo, inspector).
    layer.d.text = 'Changed elsewhere';
    const c = { save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {}, closePath() {}, stroke() {}, fill() {}, fillRect() {} } as unknown as CanvasRenderingContext2D;
    session.draw(c, 1, '#fff');
    expect(field().value).toBe('Changed elsewhere');
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 0, end: 5 });
  });

  it('commits on clicks outside the viewer but keeps editing through the inspector', () => {
    const { api, V, layer } = fixture();
    const viewer = document.createElement('div'); viewer.id = 'panel-viewer';
    const inspector = document.createElement('div'); inspector.id = 'panel-inspector';
    const font = document.createElement('button'); inspector.append(font);
    const toolbar = document.createElement('button');
    document.body.append(viewer, inspector, toolbar);
    V.inner = viewer;
    beginTextEdit(api, V, layer);
    font.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(V.canvasTextEditing).toBe('text-1');
    viewer.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(V.canvasTextEditing).toBe('text-1');
    toolbar.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(V.canvasTextEditing).toBeNull();
    expect(api.edit.commit).toHaveBeenCalledWith('Edit text');
  });

  it('commits before an inspector value field starts its own transaction', () => {
    const { api, V, layer } = fixture();
    const inspector = document.createElement('div'); inspector.id = 'panel-inspector';
    const number = document.createElement('input'); number.className = 'num'; inspector.append(number);
    document.body.append(inspector);
    beginTextEdit(api, V, layer);
    number.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(V.canvasTextEditing).toBeNull();
    expect(api.edit.commit).toHaveBeenCalledWith('Edit text');
  });

  it('closes a transaction left open elsewhere instead of failing to start', () => {
    const { api, V, layer } = fixture();
    (api.edit.begin as any).mockImplementationOnce(() => { throw new Error('A source edit is already active'); });
    expect(beginTextEdit(api, V, layer)).not.toBeNull();
    expect(api.edit.commit).toHaveBeenCalledTimes(1);
    expect(api.edit.begin).toHaveBeenCalledTimes(2);
  });

  it('handles Command+A itself and offers style shortcuts', () => {
    const { api, V, layer, dispatched } = fixture('Hello world');
    beginTextEdit(api, V, layer);
    const all = key({ key: 'a', metaKey: true });
    expect(all.defaultPrevented).toBe(true);
    expect(V.textSelection).toEqual({ layer: 'text-1', start: 0, end: 11 });
    key({ key: 'b', metaKey: true });
    expect(dispatched.at(-1)).toMatchObject({ type: 'set_property', path: 'c.weight', value: 700 });
    layer.d.weight = 700;
    key({ key: 'b', metaKey: true });
    expect(dispatched.at(-1)).toMatchObject({ path: 'c.weight', value: 400 });
    key({ key: 'i', metaKey: true });
    expect(dispatched.at(-1)).toEqual({ type: 'set_content', target: 'text-1', patch: { italic: true } });
    key({ key: '>', code: 'Period', metaKey: true, shiftKey: true });
    expect(dispatched.at(-1)).toMatchObject({ path: 'c.size', value: 49 });
    key({ key: '<', code: 'Comma', metaKey: true, shiftKey: true, altKey: true });
    expect(dispatched.at(-1)).toMatchObject({ path: 'c.size', value: 38 });
    key({ key: 't', code: 'KeyT', metaKey: true, altKey: true });
    expect(dispatched.at(-1)).toEqual({ type: 'set_content', target: 'text-1', patch: { align: 'center' } });
    expect(V.canvasTextEditing).toBe('text-1');
  });

  it('takes focus back when nothing else holds it', () => {
    const { api, V, layer } = fixture();
    const session = beginTextEdit(api, V, layer)!;
    const other = document.createElement('input'); document.body.append(other);
    const c = { save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {}, closePath() {}, stroke() {}, fill() {}, fillRect() {} } as unknown as CanvasRenderingContext2D;
    other.focus();
    session.draw(c, 1, '#fff');
    expect(document.activeElement).toBe(other);
    other.blur();
    session.draw(c, 1, '#fff');
    expect(document.activeElement).toBe(field());
  });

  it('refuses locked layers', () => {
    const { api, V, layer } = fixture('Hello', { lock: true });
    expect(beginTextEdit(api, V, layer)).toBeNull();
    expect(api.edit.begin).not.toHaveBeenCalled();
  });
});
