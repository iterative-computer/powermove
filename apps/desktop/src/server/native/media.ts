/** Agent imports read immutable host-store objects and ask pm-node to probe them.
 * No renderer file input, browser decoder or flat legacy media cache is involved. */
import { open, stat } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import path from 'node:path';
import type { MediaStoreService } from '../../main/media-store/service';
import type { NativeEngine } from '../../main/engine/native-engine';

export type PreparedMedia = { fingerprint: string; storageKey: string; sourcePath: string; kind: string; prepared: Record<string, any> };
export async function sampledFingerprint(file: string, size: number): Promise<string> {
  const sample = Math.min(64 * 1024, size), digest = createHash('sha256').update(`powermove-media-v2\n${size}\n`);
  const handle = await open(file, 'r');
  try {
    for (const offset of new Set([0, Math.max(0, Math.floor((size - sample) / 2)), Math.max(0, size - sample)])) {
      const buffer = Buffer.alloc(sample); let read = 0;
      while (read < sample) { const result = await handle.read(buffer, read, sample - read, offset + read); if (!result.bytesRead) throw new Error('Media file ended unexpectedly'); read += result.bytesRead; }
      digest.update(buffer);
    }
    return `v2:${size}:${digest.digest('hex')}`;
  } finally { await handle.close(); }
}
export async function prepareHostMedia(media: MediaStoreService, engine: NativeEngine,
  probes: Map<string, { path: string; contentHash: Uint8Array }>, args: { path: string; storage?: string }): Promise<PreparedMedia> {
  const source = args.path;
  if (typeof source !== 'string' || !path.isAbsolute(source) || source.length > 4096 || source.includes('\0')) throw new Error('Provide an absolute local media file path.');
  if (args.storage !== undefined && !['link', 'copy'].includes(args.storage)) throw new Error('storage is "copy" or "link".');
  const info = await stat(source);
  if (!info.isFile() || info.size > 512 * 1024 * 1024) throw new Error('Choose a regular media file of at most 512 MB.');
  const linked = args.storage === 'link' ? await media.link(source) : null;
  const stored = linked ? null : await media.store.putFile(source, { key: null, origin: 'import' });
  const hash = linked?.hash ?? stored!.hash;
  const file = linked ? source : media.store.objectPath(hash);
  const size = linked?.size ?? stored!.size;
  const fingerprint = await sampledFingerprint(file, size), storageKey = `media:${fingerprint}`;
  // Key writes never replace the object: Undo and frozen jobs retain its hash.
  if (stored) await media.store.putFile(file, { key: storageKey, revision: stored.revision, origin: 'import' });
  const asset = `probe:${randomUUID()}`;
  probes.set(asset, { path: file, contentHash: Buffer.from(hash, 'hex') });
  try {
    const facts = await engine.probeMedia(asset, null);
    const kind = facts.kind === 'still' ? 'image' : facts.kind;
    if ((kind === 'audio' || facts.facts.some(f => f.label === 'Audio')) && !facts.audio) throw new Error('native-runtime-version-mismatch: Rebuild the native CLI runtime to provide numeric audio import facts.');
    const hasAudio = !!facts.audio;
    const [numerator, denominator = '1'] = (facts.settings.find(s => s.member === 'frameRate')?.value ?? '').split('/');
    const fps = Number(numerator) / Number(denominator);
    return { kind, fingerprint, storageKey, sourcePath: source, prepared: { size, w: facts.displaySize?.[0] ?? 0, h: facts.displaySize?.[1] ?? 0,
      dur: facts.duration ?? 0, fps: Number.isFinite(fps) && fps > 0 ? fps : undefined, hasAudio, audioDur: hasAudio ? facts.duration ?? 0 : 0,
      channels: facts.audio?.channels ?? 0, sampleRate: facts.audio?.sampleRate ?? 0, contentHash: hash,
      ...(linked ? { storage: 'linked', link: linked } : {}) } };
  } finally { probes.delete(asset); }
}
