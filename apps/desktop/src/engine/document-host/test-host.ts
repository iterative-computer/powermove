/*
 * Test support for the document host: the engine bundle built into a
 * temporary directory, and host services made of the real ProjectSessions
 * (server/sessions.ts) over an in-memory profile store. Used by the pool
 * tests and the hidden-host contract tests; not part of any bundle.
 */
import { execFile } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import path from 'node:path';
import { ProjectSessions, type SessionMember } from '../../server/sessions';
import type { DocumentHostServices, SessionPatch } from './protocol';

const desktop = path.resolve(__dirname, '../../..');
export const WASM = process.env['POWERMOVE_PM_WASM'] ?? path.join(desktop, 'src/renderer/public/engine/pm_wasm.wasm');
export const BRIDGE = process.env['POWERMOVE_PM_BRIDGE'] ?? path.resolve(desktop, '../../engine/target/pm-bridge/pm_bridge.node');
export const FIXTURE = JSON.parse(readFileSync(path.join(__dirname, 'fixtures/project.json'), 'utf8'));

/** Builds engine.mjs (the supervisor and worker entry) once. */
export async function buildEngineBundle(): Promise<{ script: string; dispose(): void }> {
  // Inside the package (out/ is ignored), so the bundle resolves `ws` from node_modules as it does when installed.
  mkdirSync(path.join(desktop, 'out'), { recursive: true });
  const out = mkdtempSync(path.join(desktop, 'out', 'dochost-test-'));
  // A child process: Playwright's module hooks cannot load vite's config in-process.
  await new Promise<void>((resolve, reject) => {
    execFile(path.join(desktop, 'node_modules/.bin/vite'), ['build', '-c', 'engine.vite.config.ts', '--logLevel', 'error'], {
      cwd: desktop, env: { ...process.env, POWERMOVE_ENGINE_OUT: out }, maxBuffer: 16 * 1024 * 1024,
    }, (error, _stdout, stderr) => (error ? reject(new Error(`engine bundle build failed: ${stderr || error.message}`)) : resolve()));
  });
  return { script: path.join(out, 'engine.mjs'), dispose: () => rmSync(out, { recursive: true, force: true }) };
}

export interface TestHost {
  values: Map<string, unknown>;
  sessions: ProjectSessions;
  services: DocumentHostServices;
  /** Another session member (a tab) that can send remote edits. */
  tab: SessionMember;
  seed(id: string, name: string, project?: unknown): void;
  log: string[];
}

const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value));

/** A profile store and the host's sessions; pass `values` to start a "restarted" host on the same store. */
export function hostServices(values = new Map<string, unknown>(), extra: Partial<DocumentHostServices> = {}): TestHost {
  const listeners = new Set<(patch: SessionPatch) => void>();
  const log: string[] = [];
  const member: SessionMember = {
    id: 1,
    send: (_channel, payload) => { for (const l of listeners) l(payload as SessionPatch); },
    isDestroyed: () => false,
    once: () => undefined,
  };
  const sessions = new ProjectSessions({
    read: (key) => (values.has(key) ? copy(values.get(key)) : null),
    set: (key, value) => { values.set(key, copy(value)); },
  }, { channel: 'web:sync-patch', version: 'test' });
  const tab: SessionMember = { id: 2, send: () => {}, isDestroyed: () => false, once: () => undefined };
  const services: DocumentHostServices = {
    store: {
      read: async (key) => (values.has(key) ? copy(values.get(key)) : null),
      write: async (key, value) => { values.set(key, copy(value)); },
      remove: async (key) => { values.delete(key); },
    },
    session: {
      join: async (projectId) => sessions.join(member, projectId, null),
      patch: async (projectId, patches) => sessions.patch(member, projectId, patches),
      leave: (projectId) => sessions.leave(member, projectId),
      onPatch: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    },
    log: (line) => { log.push(line); },
    ...extra,
  };
  const seed = (id: string, name: string, project?: unknown) => {
    values.set(`project.${id}`, { v: 'test', proj: project ? { ...copy(project as object), id } : { ...copy(FIXTURE), id, name } });
  };
  return { values, sessions, services, seed, tab, log };
}
