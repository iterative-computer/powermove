import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './util';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/capture-bridge';

function utilRegistry() {
  const listeners: Record<string, any[]> = {};
  const listenerOptions: Record<string, any[]> = {};
  const windowStub: any = {
    addEventListener: (type: any, fn: any, options: any) => {
      (listeners[type] = listeners[type] || []).push(fn);
      (listenerOptions[type] = listenerOptions[type] || []).push(options);
    },
    removeEventListener: (type: any, fn: any) => {
      const index = (listeners[type] || []).indexOf(fn);
      if (index >= 0) {
        listeners[type]!.splice(index, 1);
        listenerOptions[type]!.splice(index, 1);
      }
    },
    requestAnimationFrame: () => 0,
    setTimeout,
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    innerWidth: 1024,
    innerHeight: 768,
  };
  const documentStub: any = { body: { style: {} }, addEventListener: windowStub.addEventListener, removeEventListener: windowStub.removeEventListener };
  const consoleStub: any = { error: vi.fn(), warn: vi.fn() };
  windowStub.document = documentStub;
  windowStub.console = consoleStub;
  vi.stubGlobal('window', windowStub);
  vi.stubGlobal('document', documentStub);
  vi.stubGlobal('console', consoleStub);
  const PM: PMRegistry = {};
  install(PM);
  return { PM, listeners, listenerOptions };
}

function fire(listeners: Record<string, any[]>, type: string, event: any) {
  (listeners[type] || []).forEach(fn => fn(event));
}

afterEach(() => {
  resetBridgeForTests();
  vi.unstubAllGlobals();
});

describe('legacy util install', () => {
  it('waits for every bounded upload chunk before finishing a download', async () => {
    const { PM } = utilRegistry();
    const bytes = new Uint8Array(1024 * 1024 + 3); bytes[bytes.length - 1] = 91;
    const chunks: Uint8Array[] = [];
    const upload = { begin: vi.fn(async () => 'upload'), chunk: vi.fn(async (_token: string, data: Uint8Array) => { chunks.push(data); }),
      finish: vi.fn(async () => ({ ok: true as const, path: '/tmp/export.bin' })), abort: vi.fn(async () => undefined) };
    installBridgeForTests({ fileUpload: upload } as any);
    expect(await PM.download(new Blob([bytes]), 'export.bin')).toEqual({ ok: true, path: '/tmp/export.bin' });
    expect(chunks.map(chunk => chunk.length)).toEqual([1024 * 1024, 3]);
    expect(chunks[1]![2]).toBe(91);
    expect(upload.finish).toHaveBeenCalledWith('upload', { name: 'export.bin' });
    expect(upload.abort).toHaveBeenCalledWith('upload');
  });

  it('drains a speculative read before aborting a failed download and preserves the write error', async () => {
    const { PM } = utilRegistry();
    const failure = new Error('upload failed');
    let rejectRead!: (error: Error) => void;
    const pendingRead = new Promise<ArrayBuffer>((_resolve, reject) => { rejectRead = reject; });
    const source = { size: 3 * 1024 * 1024, slice: vi.fn((start: number) => ({ arrayBuffer: () => start === 0 ? Promise.resolve(new ArrayBuffer(1024 * 1024)) : pendingRead })) };
    const upload = { begin: vi.fn(async () => 'upload'), chunk: vi.fn(async () => { throw failure; }),
      finish: vi.fn(), abort: vi.fn(async () => undefined) };
    installBridgeForTests({ fileUpload: upload } as any);
    const downloading = PM.download(source, 'export.bin');
    const rejected = expect(downloading).rejects.toBe(failure);
    await vi.waitFor(() => expect(upload.chunk).toHaveBeenCalledTimes(1));
    expect(source.slice).toHaveBeenCalledTimes(2);
    expect(upload.abort).not.toHaveBeenCalled();
    rejectRead(new Error('later read failed'));
    await rejected;
    expect(upload.finish).not.toHaveBeenCalled();
    expect(upload.abort).toHaveBeenCalledWith('upload');
    expect(source.slice).toHaveBeenCalledTimes(2);
  });

  it('signals rendering immediately while leaving UI work coalesced', () => {
    const { PM } = utilRegistry();
    const draw = vi.fn();
    const ui = vi.fn();
    PM.bus.on('draw', draw);
    PM.bus.on('draw:ui', ui);

    PM.invalidate('render');
    PM.invalidate('render');

    expect(draw).toHaveBeenCalledTimes(2);
    expect(ui).not.toHaveBeenCalled();
  });

  it('routes pointerup once and ignores later drag events', () => {
    const { PM, listeners } = utilRegistry();
    let ups = 0;
    let moves = 0;
    PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, {
      move: () => moves++,
      up: () => ups++,
    });

    fire(listeners, 'pointermove', { clientX: 20, clientY: 20 });
    fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });
    fire(listeners, 'pointermove', { clientX: 40, clientY: 40 });
    fire(listeners, 'pointerup', { clientX: 50, clientY: 50 });

    expect(moves).toBe(1);
    expect(ups).toBe(1);
    expect((listeners.pointermove || []).length).toBe(0);
  });

  it('cancels idempotently without committing the drag', () => {
    const { PM, listeners } = utilRegistry();
    let ups = 0;
    let cancels = 0;
    const control = PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, {
      up: () => ups++,
      cancel: () => cancels++,
    });

    control.cancel();
    control.cancel();
    fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });

    expect(cancels).toBe(1);
    expect(ups).toBe(0);
    expect((listeners.pointerup || []).length).toBe(0);
  });
});

it('continues scrubbing with a stationary cursor and releases pointer lock on mouseup', async () => {
  const { PM, listeners } = utilRegistry();
  const doc = window.document as any;
  doc.body.requestPointerLock = vi.fn(() => { doc.pointerLockElement = doc.body; fire(listeners, 'pointerlockchange', {}); return Promise.resolve(); });
  doc.exitPointerLock = vi.fn(() => { doc.pointerLockElement = null; });
  const move = vi.fn(), up = vi.fn(), cancel = vi.fn();
  PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, { infinite: true, move, up, cancel });
  fire(listeners, 'pointermove', { clientX: 15, clientY: 5 });
  fire(listeners, 'mousemove', { movementX: 2000, movementY: 0 });
  expect(move).toHaveBeenLastCalledWith(2010, 0, expect.anything());
  fire(listeners, 'mousemove', { movementX: -4000, movementY: 0 });
  expect(move).toHaveBeenLastCalledWith(-1990, 0, expect.anything());
  fire(listeners, 'mouseup', {});
  expect(up).toHaveBeenCalledTimes(1);
  expect(cancel).not.toHaveBeenCalled();
  expect(doc.exitPointerLock).toHaveBeenCalledTimes(1);
});
