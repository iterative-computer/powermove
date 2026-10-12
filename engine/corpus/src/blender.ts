/**
 * Blender snapshot capture. The capture never runs Blender: the main process's
 * `blender:run` handler is replaced by a recorder that keeps each job's
 * snapshot and asset bytes and answers with an empty render. Snapshots are what
 * `renderSnapshot` (core/scene3d/rendering.ts) hands Blender today; section 25's
 * "Blender snapshots" parity class compares `pm-blender` against them.
 */
import path from 'node:path';
import type { LegacyApp } from './app';
import { BLENDER_RUN_CHANNEL } from './catalog';
import { writeFile, type FileRecord } from './output';

export const BLENDER_FORMAT = 'powermove-corpus-blender';

export interface RecordedJob { operation: string; snapshot: unknown; assets: Array<{ id: string; name: string; base64: string }> }

/** Replace the main process's Blender handler with the recorder (once per app launch). */
export async function installBlenderRecorder(legacy: LegacyApp): Promise<void> {
  await legacy.evaluateMain(({ ipcMain }: any, channel: string) => {
    const g = globalThis as any;
    g.__corpusBlenderJobs = [];
    const plain = (value: any): any => {
      if (ArrayBuffer.isView(value)) return Array.from(value as any);
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, plain(item)]));
      return value;
    };
    ipcMain.removeHandler(channel);
    ipcMain.handle(channel, async (_event: unknown, job: any) => {
      g.__corpusBlenderJobs.push({
        operation: job?.operation, snapshot: plain(job?.snapshot),
        assets: (job?.assets || []).map((asset: any) => ({ id: asset.id, name: asset.name, base64: Buffer.from(asset.data).toString('base64') })),
      });
      return { files: [], manifest: null };
    });
  }, BLENDER_RUN_CHANNEL);
}

export async function takeBlenderJobs(legacy: LegacyApp): Promise<RecordedJob[]> {
  return legacy.evaluateMain(() => (globalThis as any).__corpusBlenderJobs.splice(0));
}

export interface SnapshotRequest { comp: string; time: number; width?: number; height?: number; preview: boolean }

export interface SnapshotPass {
  view?: string | null;
  error?: string;
  /** Set when the recorded jobs do not pair one to one with the requests. */
  mismatch?: string;
  snapshots: Array<{ request: SnapshotRequest | null; operation: string; snapshot: unknown; assets: Array<{ id: string; name: string; file: string; bytes: number; sha256: string }> }>;
}

/**
 * Pair requests with jobs in order, writing asset bytes content-addressed under
 * `<dir>/blender/assets/`. Returns the pass and the asset files written.
 */
export function pairSnapshots(out: string, dir: string, requests: SnapshotRequest[], jobs: RecordedJob[], written: Map<string, FileRecord>,
  extra: { view?: string | null; error?: string } = {}): SnapshotPass {
  const pass: SnapshotPass = { ...extra, snapshots: [] };
  if (requests.length !== jobs.length) pass.mismatch = `${requests.length} requests, ${jobs.length} recorded jobs`;
  jobs.forEach((job, index) => {
    const assets = job.assets.map(asset => {
      const bytes = new Uint8Array(Buffer.from(asset.base64, 'base64'));
      const hash = new Bun.CryptoHasher('sha256').update(bytes).digest('hex');
      const ext = path.extname(asset.name).toLowerCase().replace(/[^.a-z0-9]/g, '');
      const file = `${dir}/blender/assets/${hash}${ext}`;
      if (!written.has(file)) written.set(file, writeFile(out, file, bytes));
      return { id: asset.id, name: asset.name, file, bytes: bytes.length, sha256: hash };
    });
    pass.snapshots.push({ request: pass.mismatch ? null : requests[index]!, operation: job.operation, snapshot: job.snapshot, assets });
  });
  return pass;
}
