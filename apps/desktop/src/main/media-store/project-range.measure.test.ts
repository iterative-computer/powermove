/*
 * Measurement, not a gate: main copying an embedded media range out of an
 * open project file into the store (section 21, "Embedded media moves to
 * the media store"), through the same IPC handlers the window calls, then
 * the finish that checks the recorded SHA-256 and hashes the copy (BLAKE3).
 *
 * Opt-in, because it writes a large file:
 *
 *   PM_MEDIA_STORE_MEASURE=1 [PM_MEDIA_STORE_MEASURE_MIB=1024] [PM_MEDIA_STORE_MEASURE_RUNS=3] \
 *     npx vitest run src/main/media-store/project-range.measure.test.ts
 *
 * With `out/main/media-hash-worker.js` built (electron-vite build), hashing
 * runs in the worker pool the app uses; otherwise in process (slower, and
 * reported as such). It prints seconds per GiB and the longest stall of
 * main's event loop while copying (monitorEventLoopDelay max).
 */
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdtemp, open, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';

import { describe, expect, it } from 'vitest';

import { MEDIA_STORE_IPC } from '../../shared/media-store';
import { InProcessExecutor, WorkerPool, type HashExecutor } from './hasher';
import { PROJECT_RANGE_BYTES, registerMediaStoreIpc } from './ipc';
import { MediaStoreService } from './service';

const enabled = process.env['PM_MEDIA_STORE_MEASURE'] === '1';
const MIB = 1024 * 1024;
const sizeMiB = Math.max(1, Number(process.env['PM_MEDIA_STORE_MEASURE_MIB'] ?? 1024) || 1024);
const runs = Math.max(1, Number(process.env['PM_MEDIA_STORE_MEASURE_RUNS'] ?? 3) || 3);
const HEADER = 4096;

describe.skipIf(!enabled)('main-side project range copy (measurement)', () => {
  it(`copies and verifies ${sizeMiB} MiB, ${runs} runs`, async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'pm-media-measure-'));
    const worker = path.resolve(__dirname, '../../../out/main/media-hash-worker.js');
    const executor: HashExecutor = existsSync(worker) ? new WorkerPool(worker) : new InProcessExecutor();
    try {
      // A project file: a header, then the media range (pseudo-random bytes).
      const project = path.join(root, 'Measure.pmv');
      const sha = createHash('sha256');
      const writer = await open(project, 'w');
      try {
        await writer.write(new Uint8Array(HEADER), 0, HEADER, 0);
        const block = new Uint8Array(MIB);
        let x = 7;
        for (let i = 0; i < sizeMiB; i++) {
          for (let j = 0; j < block.length; j += 4) { x = (Math.imul(x, 1103515245) + 12345) >>> 0; block[j] = x >>> 24; block[j + 1] = x >>> 16; block[j + 2] = x >>> 8; block[j + 3] = x; }
          sha.update(block);
          await writer.write(block, 0, block.length, HEADER + i * MIB);
        }
      } finally { await writer.close(); }
      const sha256 = sha.digest('hex');
      const length = sizeMiB * MIB;

      const reader = await open(project, 'r');
      const handlers = new Map<string, (...args: any[]) => any>();
      const ipc = { handle: (channel: string, fn: any) => handlers.set(channel, fn), on: (channel: string, fn: any) => handlers.set(channel, fn) };
      const sender = { id: 1, once: () => undefined };
      try {
        const results: Array<{ seconds: number; mibPerSecond: number; maxStallMs: number }> = [];
        for (let run = 0; run < runs; run++) {
          const svc = new MediaStoreService({
            root: path.join(root, `store-${run}`), userData: root, executor, enabled: true,
            classify: { diskInfo: async () => null, mountPoint: async () => '/' },
            relink: { spotlight: null },
          });
          handlers.clear();
          registerMediaStoreIpc(ipc as never, svc, {
            isTrustedSender: () => true,
            projectRead: async (_owner, _token, offset, want) => {
              const out = new Uint8Array(want);
              const { bytesRead } = await reader.read(out, 0, want, offset);
              return out.subarray(0, bytesRead);
            }
          });
          const call = (channel: string, value: unknown) => Promise.resolve().then(() => handlers.get(channel)!({ sender }, value));
          const delay = monitorEventLoopDelay({ resolution: 1 });
          const start = performance.now();
          delay.enable();
          const token = await call(MEDIA_STORE_IPC.beginPut, { key: 'media:v2:measure', type: 'video/quicktime', size: length, origin: 'project' });
          for (let at = 0; at < length; at += PROJECT_RANGE_BYTES) {
            await call(MEDIA_STORE_IPC.putProjectRange, { token, readToken: 'measure', offset: HEADER + at, length: Math.min(PROJECT_RANGE_BYTES, length - at), at });
          }
          const record = await call(MEDIA_STORE_IPC.finishPut, { token, sha256 });
          delay.disable();
          const seconds = (performance.now() - start) / 1000;
          expect(record).toMatchObject({ size: length, source: 'project' });
          results.push({ seconds, mibPerSecond: sizeMiB / seconds, maxStallMs: delay.max / 1e6 });
          await rm(path.join(root, `store-${run}`), { recursive: true, force: true });
        }
        const perGiB = results.map((result) => result.seconds * 1024 / sizeMiB);
        process.stdout.write(`${JSON.stringify({
          measurement: 'main-side project range copy + verification',
          sizeMiB, executor: executor instanceof WorkerPool ? 'worker pool (built media-hash-worker.js)' : 'in process',
          secondsPerGiB: perGiB.map((value) => Number(value.toFixed(3))),
          mibPerSecond: results.map((result) => Math.round(result.mibPerSecond)),
          maxEventLoopStallMs: results.map((result) => Number(result.maxStallMs.toFixed(2))),
        })}\n`);
      } finally { await reader.close(); }
    } finally {
      if (executor instanceof WorkerPool) await executor.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30 * 60_000);
});
