// Synthetic Linux CPU/queue diagnostic. No display/presentation claim.
import { JSDOM } from 'jsdom';
import { RemoteDOM } from './src/shim';
import { PageApplier } from './src/applier';
import type { PageMessage } from './src/protocol';
const page = new JSDOM('<html><body></body></html>');
let completion: (() => void) | undefined,
  started = 0;
const samples: number[] = [];
const shim = new RemoteDOM({
  session: { panel: 'latency-fixture', generation: 1 },
  send(batch) {
    const wire = JSON.stringify(batch);
    queueMicrotask(() => {
      if (!applier.apply(JSON.parse(wire))) throw new Error('Page fault');
      if (batch.caused_by && completion) {
        if (
          page.window.document.querySelector('output')!.textContent !==
          (page.window.document.querySelector('input') as HTMLInputElement).value
        )
          throw new Error('Output mismatch');
        samples.push(performance.now() - started);
        const done = completion;
        completion = undefined;
        done();
      }
    });
    return true;
  },
  reportError(error) {
    throw error;
  }
});
const applier = new PageApplier({
  document: page.window.document,
  session: shim.session,
  send(message: PageMessage) {
    const wire = JSON.stringify(message);
    queueMicrotask(() => {
      if (!shim.receive(JSON.parse(wire))) throw new Error('Rejected feedback');
    });
  }
});
const input = shim.document.createElement('input'),
  output = shim.document.createElement('output');
output.textContent = 'Ready';
input.addEventListener('input', () => (output.firstChild.data = input.value));
shim.document.body.append(input, output);
for (let i = 0; i < 5000; i++) {
  const row = shim.document.createElement('div');
  row.textContent = `Static ${i}`;
  shim.document.body.append(row);
}
shim.flush();
for (let i = 0; i < 500; i++) {
  await Promise.resolve();
  shim.flush();
  if (!shim.inspect().pending && shim.inspect().sent === shim.inspect().applied) break;
}
if (shim.inspect().pending || shim.inspect().sent !== shim.inspect().applied)
  throw new Error('Initial mount did not drain');
const real = applier.node(shim.id(input)) as HTMLInputElement;
for (let i = 0; i < 1100; i++) {
  real.value = `Key ${i}`;
  await new Promise<void>((resolve) => {
    completion = resolve;
    started = performance.now();
    real.dispatchEvent(new page.window.Event('input', { bubbles: true }));
  });
}
const sorted = samples.slice(100).sort((a, b) => a - b);
console.log(
  JSON.stringify(
    {
      diagnostic: 'jsdom event → JSON → realm → JSON → applied output; 5000 static rows',
      samples: sorted.length,
      p50_ms: sorted[Math.ceil(sorted.length * 0.5) - 1],
      p95_ms: sorted[Math.ceil(sorted.length * 0.95) - 1],
      max_ms: sorted.at(-1),
      presentation_measured: false,
      target_ms_on_macos: 21
    },
    null,
    2
  )
);
shim.dispose();
applier.dispose();
page.window.close();
