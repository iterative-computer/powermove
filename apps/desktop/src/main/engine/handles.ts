/*
 * Capability handles (contract 14, plan section 19 "Security").
 *
 * The engine accepts no raw paths. Main mints a handle for every file or
 * cache area an engine may touch: 128 bits from the OS CSPRNG written as 32
 * lowercase hex digits, bound to one engine session and one port (the
 * engine launch's host connection). A handle presented with another session
 * or on another port is refused, and restarting an engine or closing a
 * session revokes its handles. Only the engine that owns the binding can
 * turn a handle into a path, by asking main on its own host port
 * (`resolveHandle`).
 */
import { randomBytes } from 'node:crypto';
import path from 'node:path';

import type { CapabilityKind, HandleAccess, HandleResolved } from '../../shared/engine-protocol';

export type HandleBinding = {
  kind: CapabilityKind;
  session: number;
  port: number;
  /** Absolute path: a file, or a directory for a whole cache area. */
  path: string;
};

export type HandleRejection = 'unknown' | 'wrongSession' | 'wrongPort';

export class HandleRejected extends Error {
  override name = 'HandleRejected';
  constructor(readonly reason: HandleRejection) {
    super(reason === 'unknown' ? 'unknown capability handle' : reason === 'wrongSession' ? 'capability handle bound to another session' : 'capability handle bound to another port');
  }
}

export function accessOf(kind: CapabilityKind): HandleAccess {
  switch (kind.kind) {
    case 'mediaRead':
    case 'sourceRead':
    case 'projectStore':
    case 'extensionBundle':
      return 'read';
    case 'cache':
      return 'readWrite';
    case 'outputWrite':
      return 'write';
  }
}

const MAX_PATH_BYTES = 4096;

export class HandleTable {
  private entries = new Map<string, HandleBinding>();

  constructor(private readonly random: (bytes: number) => Uint8Array = (n) => randomBytes(n)) {}

  /** Mints and binds a handle. */
  mint(binding: HandleBinding): string {
    if (!path.isAbsolute(binding.path) || binding.path.includes('\0') || Buffer.byteLength(binding.path) > MAX_PATH_BYTES) {
      throw new Error('A handle names an absolute path of at most 4096 bytes.');
    }
    if (!Number.isSafeInteger(binding.session) || binding.session < 0 || !Number.isSafeInteger(binding.port) || binding.port < 0) {
      throw new Error('A handle binds to a session and a port.');
    }
    const bytes = this.random(16);
    if (bytes.byteLength !== 16) throw new Error('The random source returned the wrong length.');
    const handle = Buffer.from(bytes).toString('hex');
    // A duplicate means the RNG is broken: refuse to hand it out.
    if (this.entries.has(handle)) throw new Error('The random source repeated a capability handle.');
    this.entries.set(handle, { ...binding, path: path.normalize(binding.path) });
    return handle;
  }

  /** Checks a handle presented by `session` on `port`. Never trusts the
   *  engine's word about what a handle grants. */
  check(handle: string, session: number, port: number): HandleBinding {
    const binding = this.entries.get(handle);
    if (!binding) throw new HandleRejected('unknown');
    if (binding.session !== session) throw new HandleRejected('wrongSession');
    if (binding.port !== port) throw new HandleRejected('wrongPort');
    return binding;
  }

  /** The answer to `resolveHandle`, after the binding check. */
  resolve(handle: string, session: number, port: number): HandleResolved {
    const binding = this.check(handle, session, port);
    return { handle, kind: binding.kind, access: accessOf(binding.kind), path: binding.path };
  }

  revoke(handle: string): boolean {
    return this.entries.delete(handle);
  }

  /** Drops every handle of a session on a port (the session closed). */
  revokeSession(session: number, port: number): number {
    let removed = 0;
    for (const [handle, binding] of this.entries) {
      if (binding.session === session && binding.port === port) {
        this.entries.delete(handle);
        removed++;
      }
    }
    return removed;
  }

  /** Drops every handle bound to a port (the engine launch ended). */
  revokePort(port: number): number {
    let removed = 0;
    for (const [handle, binding] of this.entries) {
      if (binding.port === port) {
        this.entries.delete(handle);
        removed++;
      }
    }
    return removed;
  }

  get size(): number {
    return this.entries.size;
  }
}
