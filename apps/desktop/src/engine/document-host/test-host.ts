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
import type { ExtensionRecord, ExtensionsChangedEvent } from '../../shared/extensions';
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

/** A host's extension list and bundles (main's registry and the /ext/ route, in memory). */
export interface TestExtensions {
  records: ExtensionRecord[];
  sources: Map<string, string>;
  service: NonNullable<DocumentHostServices['extensions']>;
  changed(event: ExtensionsChangedEvent): void;
  fetched: string[];
}

export function extensionHost(records: ExtensionRecord[] = [], sources: Record<string, string> = {}): TestExtensions {
  const listeners = new Set<(event: ExtensionsChangedEvent) => void>();
  const host: TestExtensions = {
    records,
    sources: new Map(Object.entries(sources)),
    fetched: [],
    changed: (event) => { for (const l of listeners) l(event); },
    service: {
      list: async () => copy(host.records),
      bundle: async (url) => {
        host.fetched.push(url);
        const source = host.sources.get(url);
        if (source === undefined) throw new Error(`404 ${url}`);
        return source;
      },
      onChanged: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    },
  };
  return host;
}

/** A user extension record as main's registry lists it. */
export function userRecord(id: string, manifest: Record<string, unknown> = {}, extra: Partial<ExtensionRecord> = {}): ExtensionRecord {
  return {
    id, scope: 'user', dir: `/extensions/${id}`, enabled: true,
    bundleUrl: `app://powermove/ext/${id}/bundle.js?v=h${id.length}`, bundleHash: `h${id.length}`,
    health: { state: 'ok' }, updatedAt: 0,
    manifest: { id, name: id, version: '1.0.0', apiVersion: 3, ...manifest },
    ...extra,
  } as ExtensionRecord;
}

/** An effect definition source line for a test bundle. */
export const effectSource = (id: string, def = 0.5) => `api.effects.register({ id: '${id}', label: '${id}', group: 'Stylize', params: [{ k: 'amount', label: 'Amount', def: ${def}, min: 0, max: 100 }],
    frag: '#version 300 es\\nprecision highp float;\\nin vec2 v_st;\\nuniform sampler2D u_tex;\\nuniform float u_p0;\\nout vec4 o;\\nvoid main(){ o = texture(u_tex, v_st) * (1.0 + u_p0); }' });`;
