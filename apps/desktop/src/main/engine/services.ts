/*
 * The engine → host requests main serves (contract 14), independent of
 * processes and ports: capability handles, output destinations and
 * commits, leases, and hardware codec sessions. NativeEngine routes each
 * engine launch's requests here with the launch's host port; the shared
 * contract sequence (engine/crates/pm-host/tests/fixtures) runs against
 * this and against pm-host's CLI implementation, so every host behaves the
 * same (contract 14 invariant 9).
 */
import { mkdir } from 'node:fs/promises';
import path from 'node:path';

import type { CacheKind, CapabilityKind, ContentHash, EngineProcess, Request, Response } from '../../shared/engine-protocol';
import { globalBudget, HardwareArbiter, MemoryArbiter } from './arbiter';
import { HandleRejected, HandleTable } from './handles';
import { OutputDestinations, OutputError } from './outputs';
import { isHandle, isId, isSafeUint, ProtocolError } from './protocol';

export const CACHE_KINDS: readonly CacheKind[] = ['mediaIndex', 'thumbnails', 'generatedMedia', 'renderCache', 'blenderSurfaces', 'shaderCache', 'fontSubsets', 'largeStillMips', 'scratch'];
export const CACHE_DIRS: Readonly<Record<CacheKind, string>> = {
  mediaIndex: 'Media Index',
  thumbnails: 'Thumbnails',
  generatedMedia: 'Generated Media',
  renderCache: 'Render Cache',
  blenderSurfaces: 'Blender Surfaces',
  shaderCache: 'Shader Cache',
  fontSubsets: 'Font Subsets',
  largeStillMips: 'Large Still Mips',
  scratch: 'Scratch'
};

export type HostServicesOptions = {
  cacheDirectory: string;
  automaticOutputDirectory: string;
  physicalMemory: number;
  /** Overrides the global memory budget derived from physical memory (tests). */
  globalMemory?: number;
  resolveAsset?: (asset: string, context?: { process: EngineProcess; session: number }) => Promise<{ path: string; contentHash: ContentHash | null } | null>;
  /** A generated proxy of an asset whose original is offline (draft
   *  renders, section 14.8; Phase 4a generates proxies). */
  resolveProxy?: (asset: string) => Promise<{ path: string; contentHash: ContentHash | null } | null>;
  now?: () => number;
};

function outputFailure(error: unknown): Error {
  if (error instanceof OutputError) return new ProtocolError(error.code, error.message);
  if (error instanceof ProtocolError) return error;
  const code = (error as NodeJS.ErrnoException)?.code;
  return new ProtocolError('exportFailed', `The output could not be written${code ? ` (${code})` : ''}.`);
}

export class HostServicesCore {
  readonly handles = new HandleTable();
  /** Job sessions of draft renders ("Render using: Proxies (draft)"): their
   *  offline media resolves to proxies. */
  private readonly draftSessions = new Set<number>();
  readonly outputs: OutputDestinations;
  readonly memory: MemoryArbiter;
  readonly hardware = new HardwareArbiter();

  constructor(private readonly options: HostServicesOptions) {
    this.outputs = new OutputDestinations({ handles: this.handles, automaticDirectory: options.automaticOutputDirectory });
    this.memory = new MemoryArbiter(options.globalMemory ?? globalBudget(options.physicalMemory), options.now);
  }

  /** Marks a job session as a draft render (or not). */
  setDraftSession(session: number, draft: boolean): void {
    if (draft) this.draftSessions.add(session);
    else this.draftSessions.delete(session);
  }

  cacheDirectory(kind: CacheKind): string {
    return path.join(this.options.cacheDirectory, CACHE_DIRS[kind]);
  }

  /** Serves one engine → host request from `process` on `hostPort`. */
  async serve(process: EngineProcess, hostPort: number, body: Request, session: number | null): Promise<Response> {
    switch (body.type) {
      case 'mintHandle': {
        if (!isSafeUint(body.session)) throw new ProtocolError('invalidMessage', 'a handle is minted for a session');
        const { path: file, contentHash } = await this.pathFor(body.kind, body.session, process);
        const handle = this.handles.mint({ kind: body.kind, session: body.session, port: hostPort, path: file });
        return { type: 'handle', handle, kind: body.kind, contentHash };
      }
      case 'resolveHandle': {
        if (!isHandle(body.handle) || !isSafeUint(body.session)) throw new ProtocolError('invalidMessage', 'resolveHandle needs a handle and a session');
        try {
          return { type: 'handleResolved', ...this.handles.resolve(body.handle, body.session, hostPort) };
        } catch (error) {
          if (error instanceof HandleRejected) throw new ProtocolError('capabilityRejected', error.message);
          throw error;
        }
      }
      case 'outputDestination': {
        if (process !== 'job') throw new ProtocolError('capabilityRejected', 'Only the job engine writes outputs.');
        try {
          return { type: 'outputDestination', ...await this.outputs.destination(body, session ?? 0, hostPort) };
        } catch (error) {
          throw outputFailure(error);
        }
      }
      case 'commitOutput': {
        if (process !== 'job') throw new ProtocolError('capabilityRejected', 'Only the job engine writes outputs.');
        if (!isHandle(body.temporary) || !isId(body.job)) throw new ProtocolError('invalidMessage', 'commitOutput needs a job and a handle');
        try {
          return { type: 'outputCommitted', ...await this.outputs.commit(body, session ?? 0, hostPort) };
        } catch (error) {
          throw outputFailure(error);
        }
      }
      case 'requestLease': {
        if (body.process !== process || !isSafeUint(body.wanted)) throw new ProtocolError('invalidMessage', 'a lease request names its own process and a byte count');
        const grant = this.memory.request(body);
        return { type: 'lease', ...grant.lease };
      }
      case 'hardwareSessions': {
        try {
          return { type: 'hardwareSessions', ...this.hardware.request(process, body) };
        } catch (error) {
          throw new ProtocolError('invalidMessage', error instanceof Error ? error.message : String(error));
        }
      }
      case 'blenderStatus':
      case 'blenderRender':
      case 'blenderModel':
        throw new ProtocolError('unsupported', 'Blender jobs from the new engine are not connected yet; the current engine runs Blender.');
      default:
        throw new ProtocolError('invalidMessage', `Engines do not send ${body.type} to the host.`);
    }
  }

  private async pathFor(kind: CapabilityKind, session: number, process: EngineProcess): Promise<{ path: string; contentHash: ContentHash | null }> {
    switch (kind.kind) {
      case 'cache': {
        if (!CACHE_KINDS.includes(kind.cache)) throw new ProtocolError('invalidMessage', 'unknown cache kind');
        const directory = this.cacheDirectory(kind.cache);
        await mkdir(directory, { recursive: true, mode: 0o700 });
        return { path: directory, contentHash: null };
      }
      case 'mediaRead': {
        if (!isId(kind.asset)) throw new ProtocolError('invalidMessage', 'invalid asset id');
        const resolved = await this.options.resolveAsset?.(kind.asset, { process, session })
          ?? (this.draftSessions.has(session) ? await this.options.resolveProxy?.(kind.asset) : null);
        if (!resolved) throw new ProtocolError('mediaOffline', 'The asset\'s file is not available.');
        return resolved;
      }
      case 'outputWrite':
        throw new ProtocolError('capabilityRejected', 'Output handles come from outputDestination.');
      case 'projectStore':
      case 'extensionBundle':
        throw new ProtocolError('unsupported', 'Document host handles are served by the document host (Phase 3c).');
    }
  }

  /** An engine launch ended: its handles go, its unfinished outputs are
   *  removed, its codec sessions are released. Returns removed outputs. */
  async endLaunch(process: EngineProcess, hostPort: number): Promise<number> {
    this.handles.revokePort(hostPort);
    this.hardware.releaseAll(process);
    return this.outputs.abandonPort(hostPort);
  }
}
