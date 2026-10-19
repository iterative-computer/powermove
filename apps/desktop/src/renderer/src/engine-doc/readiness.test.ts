import { describe, expect, it, vi } from 'vitest';
import { documentCommand, documentPending, runCommandWhenReady, waitForDocumentReady } from './readiness';

describe('document readiness for async callers', () => {
  it('holds a request until the native mirror is ready', async () => {
    let release!: (ready: boolean) => void;
    const doc = { enabled: true, ready: false, whenReady: new Promise<boolean>(resolve => { release = resolve; }) };
    expect(documentPending({ EngineDoc: doc } as never)).toBe(true);
    let ran = false;
    const request = waitForDocumentReady({ EngineDoc: doc } as never).then(() => { ran = true; });
    await Promise.resolve();
    expect(ran).toBe(false);
    doc.ready = true;
    release(true);
    await request;
    expect(ran).toBe(true);
    expect(documentPending({ EngineDoc: doc } as never)).toBe(false);
  });

  it('keeps a failed module load named and never runs the request', async () => {
    const doc = { enabled: true, ready: false, whenReady: Promise.resolve(false) };
    await expect(waitForDocumentReady({ EngineDoc: doc } as never)).rejects.toThrow('pm-wasm-unavailable');
  });

  it('queues behind a Retry or revival in flight instead of failing', async () => {
    let release!: (ready: boolean) => void;
    const loading = new Promise<boolean>(resolve => { release = resolve; });
    // Boot already resolved false; a Retry is loading the module again.
    const doc = { enabled: true, strict: true, ready: false, whenReady: Promise.resolve(false), settled: () => loading };
    let ran = false;
    const request = waitForDocumentReady({ EngineDoc: doc } as never).then(() => { ran = true; });
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(ran).toBe(false);
    doc.ready = true;
    release(true);
    await request;
    expect(ran).toBe(true);
  });

  it('shows the named failure again when a request cannot run', async () => {
    const showFailure = vi.fn();
    const doc = { enabled: true, strict: true, ready: false, whenReady: Promise.resolve(false), settled: () => Promise.resolve(false), showFailure };
    await expect(waitForDocumentReady({ EngineDoc: doc } as never)).rejects.toThrow('pm-wasm-unavailable');
    expect(showFailure).toHaveBeenCalledOnce();
  });

  it('classifies app commands, which always run, apart from document commands, which wait', async () => {
    for (const id of ['settings', 'palette', 'agent', 'projects', 'open', 'newProject', 'closeTab', 'nextTab', 'previousTab', 'moveTabToNewWindow']) {
      expect(documentCommand(id)).toBe(false);
    }
    for (const id of ['duplicate', 'save', 'export', 'undo', 'contextPaste', 'projectSettings', 'zoomIn', 'vendor.unknown']) {
      expect(documentCommand(id)).toBe(true);
    }
    // Ready or disabled: every command runs in its own tick and returns its result.
    expect(runCommandWhenReady({ EngineDoc: { enabled: true, ready: true } } as never, 'duplicate', () => 7)).toBe(7);
    expect(runCommandWhenReady({} as never, 'duplicate', () => 8)).toBe(8);
    // Pending: queued (undefined), and run once the load settles.
    let release!: (ready: boolean) => void;
    const doc = { enabled: true, strict: true, ready: false, whenReady: new Promise<boolean>(resolve => { release = resolve; }) };
    const run = vi.fn();
    expect(runCommandWhenReady({ EngineDoc: doc } as never, 'duplicate', run)).toBeUndefined();
    expect(runCommandWhenReady({ EngineDoc: doc } as never, 'settings', () => 'opened')).toBe('opened');
    doc.ready = true;
    release(true);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(run).toHaveBeenCalledOnce();
  });

  it('lets development modes without strict failures answer through their own fallback', async () => {
    const doc = { enabled: true, strict: false, ready: false, whenReady: Promise.resolve(false), settled: () => Promise.resolve(false) };
    await expect(waitForDocumentReady({ EngineDoc: doc } as never)).resolves.toBeUndefined();
  });

  it('does not delay an already ready or disabled evaluator', async () => {
    expect(documentPending({ EngineDoc: { enabled: true, ready: true } } as never)).toBe(false);
    expect(documentPending({ EngineDoc: { enabled: false } } as never)).toBe(false);
    expect(documentPending({} as never)).toBe(false);
    await expect(waitForDocumentReady({ EngineDoc: { enabled: true, ready: true } } as never)).resolves.toBeUndefined();
    await expect(waitForDocumentReady({ EngineDoc: { enabled: false } } as never)).resolves.toBeUndefined();
  });
});
