import { Sha256 } from '../../../../shared/sha256';

/** Window to worker: bytes for a job, or the end of it. */
export type FingerprintRequest = { id: number; bytes?: ArrayBuffer; end?: boolean; cancel?: boolean };
/** Worker to window: a finished job's lowercase hex SHA-256. */
export type FingerprintReply = { id: number; hex: string };
export interface FingerprintScope {
  onmessage: ((event: { data: FingerprintRequest }) => void) | null;
  postMessage(message: FingerprintReply): void;
}

export function serveFingerprints(scope: FingerprintScope): void {
  const jobs = new Map<number, Sha256>();
  scope.onmessage = ({ data }) => {
    if (data.cancel) { jobs.delete(data.id); return; }
    let digest = jobs.get(data.id);
    if (!digest) jobs.set(data.id, digest = new Sha256());
    if (data.bytes) digest.update(new Uint8Array(data.bytes));
    if (data.end) {
      jobs.delete(data.id);
      scope.postMessage({ id: data.id, hex: digest.digest() });
    }
  };
}
