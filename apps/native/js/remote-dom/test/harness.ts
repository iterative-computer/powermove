import { JSDOM } from 'jsdom';
import { RemoteDOM } from '../src/shim';
import { PageApplier } from '../src/applier';
import type { Batch, PageMessage } from '../src/protocol';
export function harness() {
  const page = new JSDOM('<!doctype html><html><head></head><body></body></html>', {
    pretendToBeVisual: true
  });
  const batches: Batch[] = [],
    messages: PageMessage[] = [],
    errors: unknown[] = [],
    late: string[] = [];
  const shim = new RemoteDOM({
    session: { panel: 'fixture', generation: 1 },
    send(batch) {
      batches.push(structuredClone(batch));
      queueMicrotask(() => applier.apply(structuredClone(batch)));
      return true;
    },
    reportError: (error) => errors.push(error),
    lateCancellation: (event) => late.push(event.type)
  });
  const applier = new PageApplier({
    document: page.window.document,
    session: shim.session,
    send(message) {
      messages.push(structuredClone(message));
      queueMicrotask(() => shim.receive(structuredClone(message)));
    }
  });
  return {
    page,
    shim,
    applier,
    batches,
    messages,
    errors,
    late,
    async settle() {
      for (let i = 0; i < 12; i++) {
        await Promise.resolve();
        shim.flush();
      }
    },
    close() {
      shim.dispose();
      applier.dispose();
      page.window.close();
    }
  };
}
