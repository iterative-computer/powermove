/*
 * The engine presentation worker's script (present-worker.ts): the viewer
 * starts it with `new Worker(new URL('./engine-present/present-worker-entry.ts', import.meta.url), { type: 'module' })`.
 */
import { servePresenter, type WorkerScope } from './present-worker';

servePresenter(self as unknown as WorkerScope);
