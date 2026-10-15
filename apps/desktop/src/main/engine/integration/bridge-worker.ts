/*
 * The presentation worker of the frame bridge's Electron integration test
 * in worker mode (`PM_IT_PRESENT=worker`): the viewer's production worker
 * (present-worker.ts) with every drawn frame read back in the worker, as
 * bridge-page.ts does on the main thread. The worker reports its checks
 * (`it-check`); the page asks for the next frame's quality rows
 * (`it-quality`).
 */
import { servePresenter, type WorkerScope } from '../../../extensions/viewer/engine-present/present-worker';
import { CodeReader, verifying, type QualityRows } from './page-engine';

type TestMessage = { type: 'it-quality' };

const scope = self as unknown as WorkerScope & { postMessage(message: unknown): void };
let reader: CodeReader | null = null;

servePresenter(scope, {
  wrapPainter: async (painter) => {
    reader = await CodeReader.create();
    return verifying(painter, reader);
  },
  onPresented: (frame) => reader?.verify(frame)
});

// The page sums the checks of every worker it made (a popout gets a new one).
let sent = -1;
setInterval(() => {
  const c = reader?.check;
  if (!c || c.checked === sent) return;
  sent = c.checked;
  scope.postMessage({ type: 'it-check', check: { checked: c.checked, ok: c.ok, mismatches: [...c.mismatches], distinct: c.distinctSeqs.size } });
}, 50);

(scope as unknown as { addEventListener(type: 'message', listener: (event: MessageEvent<TestMessage>) => void): void }).addEventListener('message', (event) => {
  const message = event.data;
  if (message.type === 'it-quality') {
    if (!reader) return;
    void reader.nextQuality().then((rows: QualityRows) => scope.postMessage({ type: 'it-quality', rows }));
  }
});
