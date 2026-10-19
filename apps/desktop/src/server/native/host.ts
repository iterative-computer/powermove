import { prepareHostMedia } from './media';
import { profileStore } from './profile-store';
import { createRequire } from 'node:module';
import { SnapshotService } from '../../main/engine/snapshots';
import { WindowSnapshotSessions } from '../../main/engine/window-sessions';
import type { EngineSnapshotRequest, EngineSnapshotReply } from '../../shared/engine-ipc';
import { readShaderEnvironment, shaderCacheDirectory } from '../../main/engine/shader-environment';
/** macOS hidden host: in-process host services, supervised native engines and
 * one production document worker per project. Chromium is a lazy DOM service. */
import { existsSync } from 'node:fs';
import { readFile, stat } from 'node:fs/promises';
import { discoverBlender } from '../../main/blender-service';
import { validCompatAnswer } from '../../main/engine/compat-frames';
import os from 'node:os';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import type { AgentToolRequestEvent, AgentToolResponseEvent } from '../../shared/ipc';
import { IPC } from '../../shared/ipc';
import type { Store } from '../../main/storage';
import type { DocumentHostServices } from '../../engine/document-host/protocol';
import { DocumentHostPool } from '../../engine/document-host/pool';
import { NativeEngine } from '../../main/engine/native-engine';
import { AgentJobs } from '../../main/engine/agent-jobs';
import { packAgentProject } from '../../main/agent-tools/project-packer';
import { WorkerPool } from '../../main/media-store/hasher';
import { MediaStoreService, validLink } from '../../main/media-store/service';
import { STORE_DIRECTORY } from '../../main/media-store/store';
import { RemoteClient, type WebIpcMain } from '../clients';
import type { ProjectSessions, SessionMember } from '../sessions';
import { WEB } from '../../shared/wire';
import { NodeEngineTransport } from './ports';
import { EngineFontClient } from '../../main/engine-fonts';

export type NativeSnapshot = { project: Record<string, any>; document: unknown; definitions: import('../../shared/engine-protocol').DefinitionSnapshot[] };
export interface NativeBrowser {
  call(project: string, request: AgentToolRequestEvent): Promise<AgentToolResponseEvent>;
  compat(project: string, request: import('../../shared/engine-protocol').CompatFrameRequest): Promise<import('../../shared/engine-ipc').EngineCompatAnswer>;
  shader(session: number, request: import('../../shared/engine-protocol').ShaderFallbackRequest, textures: Uint16Array[]): Promise<import('../../main/engine/shader-fallback-relay').ShaderFallbackResult>;
  close(): Promise<void>;
}
export interface HiddenHostOptions {
  userData: string; exportsDir: string; distDir: string; rendererDir: string; log(line: string): void;
  ipc: WebIpcMain; store: Store; sessions: ProjectSessions; browser: NativeBrowser;
  /** Test/embedded worker bundle; production uses dist/engine/engine.mjs. */
  documentScript?: string;
  documentEnv?: Record<string, string>;
  ownerForRun?(run: string): number | null;
  extensions: NonNullable<DocumentHostServices['extensions']>;
}

class NativeOwner extends RemoteClient {
  constructor(id: number, ipc: WebIpcMain, readonly project: string, private readonly dispatchTool: (owner: NativeOwner, request: AgentToolRequestEvent) => void) {
    super(id, { send() {}, close() {} }, 'powermove://native-hidden', ipc);
    this.kind = 'engine';
  }
  override send(channel: string, ...args: unknown[]): void {
    if (channel === IPC.agentToolRequest && !this.isDestroyed()) this.dispatchTool(this, args[0] as AgentToolRequestEvent);
  }
}

const DOM = new Set(['get_workspace_state', 'get_panel_layout', 'set_panel_layout', 'open_panel', 'get_panel_state', 'interact_panel', 'capture_panel', 'computer_use_panel', '__panel_bounds', '__prepare_panel_input']);
export class NativeHiddenHost {
  readonly engine: NativeEngine;
  readonly nowUs: () => number;
  private readonly sharedPixels = new Map<string, ReturnType<typeof setTimeout>>();
  readonly jobs: AgentJobs;
  readonly pool: DocumentHostPool;
  readonly media: MediaStoreService;
  readonly snapshots: SnapshotService;
  private readonly unlinkPixels: (name: string) => void;
  private readonly windowSessions: WindowSnapshotSessions;
  private readonly interactiveProjects = new Map<number, string>();
  private readonly cpuProjects = new Map<number, string>();
  private readonly definitionBytes = new Map<number, string>();
  readonly fonts = new EngineFontClient();
  private readonly jobFonts = new EngineFontClient();
  private readonly probes = new Map<string, { path: string; contentHash: Uint8Array }>();
  private readonly hashes: WorkerPool;
  private readonly owners = new Map<string, NativeOwner>();
  private readonly aliases = new Map<number, string>();
  private active = 'powermove-home';
  private nextOwner = 1_000_000;
  private readonly members = new Map<string, SessionMember>();
  private readonly patches = new Set<(patch: import('../../engine/document-host/protocol').SessionPatch) => void>();

  constructor(private readonly options: HiddenHostOptions) {
    const { distDir, userData, sessions, store, log } = options;
    const bundled = path.join(distDir, 'native', `${process.platform}-${process.arch}`);
    const addon = process.env['POWERMOVE_ENGINE_ADDON'] ?? path.join(bundled, 'pm_node.node');
    const bridge = process.env['POWERMOVE_PM_BRIDGE'] ?? path.join(bundled, 'pm_bridge.node');
    if (!existsSync(addon) || !existsSync(bridge)) throw new Error(`native-runtime-unavailable: Build/install the ${process.platform}-${process.arch} native CLI addons.`);
    const loaded = createRequire(path.join(distDir, 'server.mjs'))(bridge) as { takePixels(name: string, length: number): Uint8Array; nowUs(): number; publishPixels(bytes: Uint8Array): string; unlinkPixels(name: string): void };
    this.nowUs = () => loaded.nowUs();
    this.unlinkPixels = name => loaded.unlinkPixels(name);
    const probeDirectory = shaderCacheDirectory(userData);
    const transport = new NodeEngineTransport();
    this.hashes = new WorkerPool(path.join(distDir, 'native-media-hash.cjs'), 1);
    this.media = new MediaStoreService({ root: path.join(userData, STORE_DIRECTORY), userData, executor: this.hashes, enabled: true });
    this.engine = new NativeEngine({ spawn: transport.spawn(path.join(distDir, 'native-engine-child.cjs'), addon, log, {
      POWERMOVE_ENGINE_PARITY_ENV: process.env['POWERMOVE_ENGINE_PARITY_ENV'] ?? readShaderEnvironment(probeDirectory),
      POWERMOVE_ENGINE_SHADER_CACHE: probeDirectory
    }), channel: transport.channel,
      cacheDirectory: path.join(userData, 'Engine Cache'), automaticOutputDirectory: options.exportsDir,
      physicalMemory: os.totalmem(), hostNow: this.nowUs,
      log: (level, message) => log(`[native:${level}] ${message}`), resolveAsset: (id, context) => this.probes.has(id) ? Promise.resolve(this.probes.get(id)!) : context?.process === 'interactive' && this.interactiveProjects.has(context.session)
        ? this.resolveProjectAsset(this.interactiveProjects.get(context.session)!, id) : this.media.resolveAsset(id, context),
      blender: { tempRoot: os.tmpdir(), locate: async () => {
        const configured = await readFile(path.join(userData, 'blender-path.json'), 'utf8').then(raw => { const value = JSON.parse(raw)?.path; return typeof value === 'string' && path.isAbsolute(value) ? value : undefined; }).catch(() => undefined);
        return discoverBlender(configured);
      } },
      compatFrame: async request => {
        const answer = await options.browser.compat(request.document, request);
        if (!validCompatAnswer(answer)) throw new Error('invalidMessage: The compatibility runner returned invalid pixels.');
        if (answer.status !== 'pixels') throw new Error(`notHeadlessRenderable: ${answer.message}`);
        let payload: import('../../shared/engine-protocol').PixelPayload = { kind: 'inline', bytes: answer.data };
        if (answer.data.length > 1024 * 1024) { const name = loaded.publishPixels(answer.data); const timer = setTimeout(() => { loaded.unlinkPixels(name); this.sharedPixels.delete(name); }, 60_000); timer.unref(); this.sharedPixels.set(name, timer); payload = { kind: 'sharedMemory', name, length: answer.data.length }; }
        return { type: 'compatFrame', key: answer.key, width: answer.width, height: answer.height, format: answer.format, origin: answer.origin, scale: answer.scale, payload };
      },
      shaderFallback: (session, request, textures) => options.browser.shader(session, request, textures) });
    this.jobs = new AgentJobs({ engine: this.engine, definitions: id => this.engine.definitions.current(id),
      sessionOwner: (session, owner) => {
        const project = owner === null ? undefined : this.aliases.get(owner);
        return this.jobFonts.setSessionOwner(session, project ? this.owner(project).id : owner);
      },
      bindMediaSession: async (session, project) => { if (project) await this.migrateMedia(project); await this.media.bindJob(session, project); },
      packProject: (project, session, document) => packAgentProject(project, { directory: options.exportsDir,
        resolveAsset: id => this.media.resolveAsset(id, { process: 'job', session }) }, document), log: (level, message) => log(`[jobs:${level}] ${message}`) });
    const services: DocumentHostServices = {
      store: profileStore(store, keys => { for (const client of options.ipc.all()) client.send(IPC.storeChanged, { keys }); }),
      session: {
        join: async project => {
          let member = this.members.get(project);
          if (!member) {
            member = { id: this.nextOwner++, isDestroyed: () => false, once: () => {}, send: (channel, payload) => {
              if (channel === WEB.syncPatch) for (const listener of this.patches) listener(payload as never);
            } };
            this.members.set(project, member);
          }
          return sessions.join(member, project, null);
        },
        patch: async (project, patches) => sessions.patch(this.members.get(project)!, project, patches),
        leave: project => { const member = this.members.get(project); if (member) sessions.leave(member, project); this.members.delete(project); },
        onPatch: listener => { this.patches.add(listener); return () => { this.patches.delete(listener); }; }
      },
      extensions: options.extensions,
      mediaImport: { prepare: args => prepareHostMedia(this.media, this.engine, this.probes, args) },
      mediaPath: { lookup: async request => {
        const doc = sessions.document(request.projectId ?? this.active) as any;
        const meta = doc?.assets?.[request.assetId];
        if (!meta) return null;
        if (meta.storage === 'linked' && validLink(meta.link)) { const found = await this.media.locate(meta.link); return 'path' in found ? { path: found.path, origin: 'source' } : null; }
        const resolved = await this.resolveProjectAsset(request.projectId ?? this.active, request.assetId);
        return resolved ? { path: resolved.path, origin: 'host' } : null;
      } }, log,
    };
    this.pool = new DocumentHostPool({ services, newEngine: true, script: options.documentScript ?? path.join(distDir, 'engine', 'engine.mjs'),
      env: { POWERMOVE_PM_WASM: path.join(options.rendererDir, 'engine', 'pm_wasm.wasm'), POWERMOVE_PM_BRIDGE: bridge, ...options.documentEnv } });
    this.windowSessions = new WindowSnapshotSessions(this.engine, this.engine.definitions,
      (level, message) => log(`[snapshots:${level}] ${message}`), undefined, async (session, owner) => {
        if (owner === null) this.interactiveProjects.delete(session);
        else { const project = this.cpuProjects.get(owner);  if (project) this.interactiveProjects.set(session, project); }
        await this.fonts.setSessionOwner(session, owner === null ? null : owner - 100_000_000);
      });
    this.snapshots = new SnapshotService({ engine: this.engine, windowSessions: this.windowSessions,
      takePixels: (name, length) => loaded.takePixels(name, length), log: (level, message) => log(`[snapshots:${level}] ${message}`) });
    this.fonts.mirror(this.jobFonts);
    this.engine.addClient(-1001, (_info, port) => this.fonts.connect(port), { peer: 'fonts', processes: ['interactive'] });
    this.engine.addClient(-1002, (_info, port) => this.jobFonts.connect(port), { peer: 'fonts', processes: ['job'] });
  }

  async start(): Promise<void> {
    await this.media.store.init();
    await this.engine.start();
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => { off(); reject(new Error('native-engine-unavailable: Native engines did not become ready.')); }, 30_000);
      const check = () => { if (this.engine.isReady('job') && this.engine.isReady('interactive')) { clearTimeout(timer); off(); resolve(); } };
      const off = this.engine.listen({ status: check }); check();
    });
  }
  owner(project = this.active): RemoteClient {
    let owner = this.owners.get(project);
    if (!owner) { owner = new NativeOwner(this.nextOwner++, this.options.ipc, project, (o, r) => { void this.dispatch(o, r); }); this.owners.set(project, owner); }
    return owner;
  }
  projectForSession(session: number): string {
    const owner = this.jobs.ownerOf(session);
    const project = [...this.owners.values()].find(item => item.id === owner)?.project ?? (owner === null ? undefined : this.aliases.get(owner));
    if (!project) throw new Error('shader-session-unavailable: The job has no pinned project.');
    return project;
  }
  projectForOwner(id: number): string {
    const project = [...this.owners.values()].find(owner => owner.id === id)?.project ?? this.aliases.get(id);
    if (!project) throw new Error('project-session-unavailable');
    return project;
  }
  private respond(owner: NativeOwner, response: AgentToolResponseEvent): void {
    this.options.ipc.emitOn(IPC.agentToolResponse, { sender: owner, senderFrame: owner.mainFrame, preventDefault() {} }, response);
  }
  async snapshot(project: string): Promise<NativeSnapshot> {
    const reply = await this.pool.call(project, { runId: 'native-host', callId: randomUUID(), tool: '__host_snapshot', arguments: {}, baseRevision: 0 });
    if (!reply.ok) throw new Error(reply.error);
    const value = JSON.parse((reply.content.find(c => c.type === 'text') as { text: string }).text) as NativeSnapshot;
    this.updateDefinitions(this.owner(project).id, value.definitions);
    return value;
  }
  updateDefinitions(owner: number, definitions: NativeSnapshot['definitions']): void {
    const bytes = JSON.stringify(definitions);
    if (this.definitionBytes.get(owner) === bytes) return;
    this.engine.definitions.update(owner, { generation: (this.engine.definitions.generationOf(owner) ?? 0) + 1, full: true, upserts: definitions, removals: [] });
    this.definitionBytes.set(owner, bytes);
  }
  async cpuSnapshot(project: string, request: EngineSnapshotRequest, definitions: NativeSnapshot['definitions']): Promise<EngineSnapshotReply> {
    if (request.document?.id !== project || request.session !== null) throw new Error('invalidMessage: CPU snapshots must name the pinned project document.');
    const owner = this.owner(project).id + 100_000_000;
    this.cpuProjects.set(owner, project);
    this.updateDefinitions(owner, definitions);
    return this.snapshots.snapshot(owner, request);
  }
  private async resolveProjectAsset(project: string, id: string) {
    const meta = (this.options.sessions.document(project) as any)?.assets?.[id];
    if (!meta) return null;
    if (meta.storage === 'linked' && validLink(meta.link)) {
      const located = await this.media.locate(meta.link);
      return 'path' in located ? { path: located.path, contentHash: Buffer.from(meta.link.hash, 'hex') } : null;
    }
    await this.migrateMedia({ assets: { [id]: meta } });
    const hash = typeof meta.contentHash === 'string' ? meta.contentHash : (await this.media.store.record(meta.storageKey ?? id))?.hash;
    return hash && await this.media.store.hasObject(hash) ? { path: this.media.store.objectPath(hash), contentHash: Buffer.from(hash, 'hex') } : null;
  }
  /** Promote the previous CLI's flat copied media into immutable host objects.
   * Keep the source files so flag-off profiles and older app versions still work. */
  private async migrateMedia(project: Record<string, any>): Promise<void> {
    for (const meta of Object.values<any>(project.assets ?? {})) {
      if (!meta || meta.storage === 'linked') continue;
      const key = meta.storageKey ?? meta.id;
      if (typeof key !== 'string' || await this.media.store.record(key)) continue;
      const file = path.join(this.options.userData, 'Media Store', key.replace(/[^A-Za-z0-9._-]/g, '_'));
      const info = await stat(file).catch(error => { if (error.code === 'ENOENT') return null; throw error; });
      if (info?.isFile()) await this.media.store.putFile(file, { key, origin: 'import' });
    }
  }
  private async dispatch(owner: NativeOwner, request: AgentToolRequestEvent): Promise<void> {
    try {
      const alias = this.options.ownerForRun?.(request.runId);
      if (alias !== undefined && alias !== null) this.aliases.set(alias, owner.project);
      let result: AgentToolResponseEvent;
      const reply = (value: unknown): AgentToolResponseEvent => ({ runId: request.runId, callId: request.callId, ok: true, content: [{ type: 'text', text: JSON.stringify(value) }] });
      if (request.tool === '__mcp_list_projects') result = reply({ currentProjectId: this.active === 'powermove-home' ? null : this.active, projects: this.options.store.snapshot()['projects'] ?? [] });
      else if (request.tool === '__mcp_create_project') {
        result = await this.pool.call('powermove-home', { ...request, tool: '__host_create_project' });
        if (result.ok) { const value = JSON.parse((result.content[0] as { text: string }).text); this.active = value.projectId; delete value.project; result = reply(value); }
      } else if (request.tool === '__mcp_open_project') {
        const id = String(request.arguments.projectId);
        if (!/^[A-Za-z0-9_-]{1,120}$/.test(id)) throw new Error('Invalid project id.');
        await this.pool.open(id); this.active = id;
        result = reply({ projectId: id, name: (await this.snapshot(id)).project.name });
      } else if (DOM.has(request.tool)) result = await this.options.browser.call(owner.project, request);
      else {
        if (['__engine_frames_plan', '__mcp_context', '__engine_save_plan'].includes(request.tool) && owner.project !== 'powermove-home') { const snapshot = await this.snapshot(owner.project); if (alias !== undefined && alias !== null) this.updateDefinitions(alias, snapshot.definitions); }
        result = await this.pool.call(owner.project, request);
      }
      this.respond(owner, result);
    } catch (error) { this.respond(owner, { runId: request.runId, callId: request.callId, ok: false, content: [], error: error instanceof Error ? error.message : String(error) }); }
  }
  async close(): Promise<void> {
    for (const [name, timer] of this.sharedPixels) { clearTimeout(timer); this.unlinkPixels(name); } this.sharedPixels.clear();
    for (const owner of this.owners.values()) owner.destroy();
    try { await this.pool.shutdown(); } finally {
      this.windowSessions.dispose(); this.jobs.dispose();
      try { await this.engine.stop(); } finally { await this.hashes.close(); }
    }
  }
}
