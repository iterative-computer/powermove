/*
 * `store:*` handlers: browsing the registry and installing, updating and
 * removing store extensions (store plan §2.2, §2.6, §6).
 *
 * Renderer-reachable IPC is hostile: every payload is zod-validated and the
 * sender must be a trusted app frame. Registry and install failures return
 * as `{ ok: false, error }`; malformed payloads and untrusted senders throw.
 * Every registry response was already parsed with its wire schema by the
 * store client before it gets here.
 */
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { ApiError, Category, Handle, type MeDto, type VarDecl } from '@powermove/registry/wire';
import { z } from 'zod';

import { IpcValidationError } from '../../shared/guards';
import { EXTENSION_ID, EXTENSION_VERSION, type ExtensionRecord } from '../../shared/extensions';
import {
  STORE_IPC,
  STORE_QUERY_MAX,
  type LibraryItemDto,
  type StoreCategory,
  type StoreChannels,
  type StoreErrorBody,
  type StoreResult,
  type StoreUpdates
} from '../../shared/store-ipc';
import { StoreLocalError, type StoreInstaller } from './install';
import { isMine } from './ownership';
import type { ProvenanceFile, ProvenanceRecord, ProvenanceStore } from './provenance';
import type { StoreClient } from './store-client';

type Sender = Pick<IpcMainInvokeEvent, 'sender' | 'senderFrame'>;

const none = z.union([z.undefined(), z.null(), z.strictObject({})]);
const uuid = z.uuid();
const localId = z.strictObject({ localId: z.string().regex(EXTENSION_ID) });
const releaseRequest = z.strictObject({ releaseId: uuid });
/** Longest cursor accepted from the renderer; the registry's are far shorter. */
const CURSOR_MAX = 512;
/** Longest source path asked for (the registry's tree paths are capped well below). */
const PATH_MAX = 1024;

export const storeSchemas = {
  'store:browse': none,
  'store:extensions': z.strictObject({
    category: Category.optional(),
    q: z.string().max(STORE_QUERY_MAX).optional(),
    cursor: z.string().max(CURSOR_MAX).regex(/^[A-Za-z0-9_-]+$/).optional(),
    sort: z.enum(['new', 'installs', 'name']).optional()
  }),
  'store:detail': z.strictObject({ handle: Handle, slug: z.string().regex(EXTENSION_ID) }),
  'store:release': releaseRequest,
  'store:tree': releaseRequest,
  'store:file': z.strictObject({
    handle: Handle,
    slug: z.string().regex(EXTENSION_ID),
    version: z.string().regex(EXTENSION_VERSION),
    path: z.string().min(1).max(PATH_MAX).refine((value) =>
      !value.includes('\0') && !value.includes('\\') && !value.startsWith('/') &&
      value.split('/').every((segment) => segment !== '' && segment !== '.' && segment !== '..'), 'invalid path')
  }),
  'store:compare': z.strictObject({ base: uuid, head: uuid }),
  'store:install': z.strictObject({ repoId: uuid, releaseId: uuid }),
  'store:update': localId,
  'store:uninstall': localId,
  'store:library': none,
  'store:check-updates': none
} as const satisfies Record<keyof StoreChannels, z.ZodType>;

function parse<T>(schema: z.ZodType<T>, channel: string, payload: unknown): T {
  const result = schema.safeParse(payload);
  if (!result.success) throw new IpcValidationError(channel, result.error.issues.map((issue) => issue.message).join('; '));
  return result.data;
}

export async function storeResult<T>(run: () => Promise<T>): Promise<StoreResult<T>> {
  try {
    return { ok: true, value: await run() };
  } catch (error) {
    let body: StoreErrorBody;
    if (error instanceof ApiError) body = error.body;
    else if (error instanceof StoreLocalError) body = { error: error.code, detail: error.detail };
    else {
      console.error('[store] action failed', error instanceof Error ? error.message : 'unknown error');
      body = { error: 'internal', detail: 'Something went wrong on this Mac. Try again.' };
    }
    return { ok: false, error: body };
  }
}

/* ── the Library ─────────────────────────────────────────── */

/** The store category a local extension reads as, from what it contributes. */
const CATEGORY_BY_CONTRIBUTION: Record<string, StoreCategory> = {
  effects: 'effects',
  transitions: 'transitions',
  panels: 'panels',
  inspector: 'panels',
  status: 'panels',
  themes: 'themes',
  commands: 'commands',
  keybindings: 'commands',
  palette: 'commands',
  menus: 'commands',
  layers: 'layers',
  media: 'layers'
};

export function categoryFor(contributes: readonly string[]): StoreCategory {
  for (const kind of contributes) {
    const category = CATEGORY_BY_CONTRIBUTION[kind];
    if (category) return category;
  }
  return 'tools';
}

function varsOf(record: ExtensionRecord): VarDecl[] {
  return (record.manifest?.vars ?? []).map((decl) => ({
    key: decl.key,
    label: decl.label,
    ...(decl.secret ? { secret: true } : {}),
    ...(decl.required ? { required: true } : {}),
    ...(decl.hint ? { hint: decl.hint } : {})
  }));
}

export interface LibraryInput {
  records: ExtensionRecord[];
  provenance: ProvenanceFile;
  updates: StoreUpdates;
  me: MeDto | null;
  /** Whether a store install's folder no longer matches what was installed. */
  modified(localId: string): boolean;
}

/**
 * Everything on this Mac, grouped per plan §2.2: published and mine → Yours;
 * else installed from the store → From the store; else made here → Yours;
 * built-ins → Built in.
 */
export function buildLibrary(input: LibraryInput): LibraryItemDto[] {
  return input.records.map((record): LibraryItemDto => {
    const provenance: ProvenanceRecord | undefined = record.scope === 'builtin' ? undefined : input.provenance[record.id];
    const origin = provenance?.origin;
    const contributes = [...(record.manifest?.contributes ?? [])];
    const modified = origin ? input.modified(record.id) : false;
    const group = record.scope === 'builtin'
      ? 'builtin'
      : provenance?.published && isMine(provenance, input.me)
        ? 'yours'
        : origin ? 'store' : 'yours';
    const handle = origin?.coordinate.split('/')[0];
    const maker: LibraryItemDto['maker'] = group === 'builtin'
      ? { builtin: true }
      : group === 'store' && handle ? { handle } : { you: true };

    const check = input.updates[record.id];
    const pending = provenance?.pendingUpdate;
    let update: LibraryItemDto['update'] = null;
    if (origin && check?.state === 'ok' && check.latest && check.latest.releaseId !== origin.releaseId) {
      update = {
        version: check.latest.version,
        releaseId: check.latest.releaseId,
        modified,
        state: pending?.releaseId === check.latest.releaseId ? 'staged-for-merge' : 'available'
      };
    } else if (origin && pending && pending.releaseId !== origin.releaseId && check?.state !== 'removed' && check?.state !== 'tombstoned') {
      update = { version: pending.version, releaseId: pending.releaseId, modified, state: 'staged-for-merge' };
    }

    const item: LibraryItemDto = {
      localId: record.id,
      name: record.manifest?.name ?? record.id,
      version: record.manifest?.version ?? origin?.version ?? '',
      category: categoryFor(contributes),
      contributes,
      vars: varsOf(record),
      health: record.health,
      enabled: record.enabled,
      description: record.manifest?.description ?? null,
      group,
      maker,
      update,
      modified
    };
    if (origin) item.origin = { coordinate: origin.coordinate, version: origin.version, repoId: origin.repoId, releaseId: origin.releaseId };
    if (record.manifest?.forkedFrom) item.forkedFrom = record.manifest.forkedFrom;
    if (provenance?.published) {
      item.published = { coordinate: null, version: provenance.published.version, releaseId: provenance.published.releaseId };
    }
    if (check?.state === 'removed' || check?.state === 'tombstoned') item.removed = true;
    return item;
  });
}

/* ── handlers ────────────────────────────────────────────── */

export interface StoreIpcOptions {
  store: StoreClient;
  installer: StoreInstaller;
  provenance: ProvenanceStore;
  registry: { list(): ExtensionRecord[] };
  me(): MeDto | null;
  /** Removes a user extension through the `ext:remove` path for this sender (asks about values). */
  removeExtension(event: IpcMainInvokeEvent, localId: string): Promise<boolean>;
  isTrusted(event: Sender): boolean;
}

export function registerStoreIpc(ipcMain: Pick<IpcMain, 'handle'>, options: StoreIpcOptions): void {
  const { store, installer } = options;
  /* Snapshotting a folder reads every file; the answer holds until the
     registry sees that folder change (a new `updatedAt`) or a new install. */
  const modifiedCache = new Map<string, { key: string; modified: boolean }>();

  function handle<C extends keyof StoreChannels, T>(
    channel: C,
    schema: z.ZodType<T>,
    run: (request: T, event: IpcMainInvokeEvent) => Promise<StoreChannels[C]['res']>
  ): void {
    ipcMain.handle(channel, async (event: IpcMainInvokeEvent, payload: unknown) => {
      if (!options.isTrusted(event)) throw new IpcValidationError(channel, 'untrusted sender');
      return run(parse(schema, channel, payload), event);
    });
  }

  async function library(): Promise<LibraryItemDto[]> {
    const records = options.registry.list();
    let provenance: ProvenanceFile = {};
    try {
      provenance = await options.provenance.read();
    } catch (error) {
      console.error('[store] provenance unreadable', error instanceof Error ? error.message : 'unknown error');
    }
    const modified = new Map<string, boolean>();
    await Promise.all(records.map(async (record) => {
      const origin = record.scope === 'user' ? provenance[record.id]?.origin : undefined;
      if (!origin) return;
      const key = `${record.updatedAt}:${origin.treeSha}`;
      const cached = modifiedCache.get(record.id);
      if (cached?.key === key) {
        modified.set(record.id, cached.modified);
        return;
      }
      const value = await installer.isModified(record.id);
      modifiedCache.set(record.id, { key, modified: value });
      modified.set(record.id, value);
    }));
    return buildLibrary({
      records,
      provenance,
      updates: installer.updates(),
      me: options.me(),
      modified: (id) => modified.get(id) ?? false
    });
  }

  handle(STORE_IPC.browse, storeSchemas['store:browse'], () => storeResult(() => store.browse()));
  handle(STORE_IPC.extensions, storeSchemas['store:extensions'], (request) => storeResult(() => store.extensions(request)));
  handle(STORE_IPC.detail, storeSchemas['store:detail'], (request) => storeResult(() => store.detail(request.handle, request.slug)));
  handle(STORE_IPC.release, storeSchemas['store:release'], (request) => storeResult(() => store.release(request.releaseId)));
  handle(STORE_IPC.tree, storeSchemas['store:tree'], (request) => storeResult(() => store.tree(request.releaseId)));
  handle(STORE_IPC.file, storeSchemas['store:file'], (request) =>
    storeResult(() => store.file(request.handle, request.slug, request.version, request.path)));
  handle(STORE_IPC.compare, storeSchemas['store:compare'], (request) => storeResult(() => store.compare(request.base, request.head)));
  handle(STORE_IPC.install, storeSchemas['store:install'], (request) => storeResult(() => installer.installRelease(request)));
  handle(STORE_IPC.update, storeSchemas['store:update'], (request) => storeResult(async () => {
    const result = await installer.updateRelease(request.localId);
    modifiedCache.delete(request.localId);
    return result;
  }));
  handle(STORE_IPC.uninstall, storeSchemas['store:uninstall'], (request, event) => storeResult(async () => {
    modifiedCache.delete(request.localId);
    return installer.uninstall(request.localId, (id) => options.removeExtension(event, id));
  }));
  handle(STORE_IPC.library, storeSchemas['store:library'], () => library());
  handle(STORE_IPC.checkUpdates, storeSchemas['store:check-updates'], () => storeResult(() => installer.checkUpdates()));
}
