import { describe, expect, it } from 'vitest';
import { documentPending, waitForDocumentReady } from './readiness';

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
