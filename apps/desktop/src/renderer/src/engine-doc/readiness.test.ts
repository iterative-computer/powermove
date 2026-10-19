import { describe, expect, it } from 'vitest';
import { waitForDocumentReady } from './readiness';

describe('document readiness for async callers', () => {
  it('holds a request until the native mirror is ready', async () => {
    let release!: (ready: boolean) => void;
    const doc = { enabled: true, ready: false, whenReady: new Promise<boolean>(resolve => { release = resolve; }) };
    let ran = false;
    const request = waitForDocumentReady({ EngineDoc: doc } as never).then(() => { ran = true; });
    await Promise.resolve();
    expect(ran).toBe(false);
    doc.ready = true;
    release(true);
    await request;
    expect(ran).toBe(true);
  });

  it('keeps a failed module load named and never runs the request', async () => {
    const doc = { enabled: true, ready: false, whenReady: Promise.resolve(false) };
    await expect(waitForDocumentReady({ EngineDoc: doc } as never)).rejects.toThrow('pm-wasm-unavailable');
  });

  it('does not delay an already ready or disabled evaluator', async () => {
    await expect(waitForDocumentReady({ EngineDoc: { enabled: true, ready: true } } as never)).resolves.toBeUndefined();
    await expect(waitForDocumentReady({ EngineDoc: { enabled: false } } as never)).resolves.toBeUndefined();
  });
});
