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
import { ApiError, Category, Handle, Visibility, type MeDto, type VarDecl } from '@powermove/registry/wire';
import { z } from 'zod';

import { IpcValidationError } from '../../shared/guards';
import { EXTENSION_ID, EXTENSION_VERSION, type ExtensionRecord, type ExtensionsChangedEvent } from '../../shared/extensions';
import {
  STORE_IPC,
  STORE_QUERY_MAX,
  type LibraryItemDto,
  type StoreCategory,
  type StoreChannels,
  type StoreErrorBody,
  type StoreResult,
  type StoreTrustResult,
  type StoreUpdates
} from '../../shared/store-ipc';
import { PUBLISH_LICENCES, PUBLISH_LIMITS } from '../../shared/publish';
import { StoreLocalError, type StoreInstaller } from './install';
import { isMine } from './ownership';
import type { Publisher } from './publish';
import type { ProvenanceFile, ProvenanceRecord, ProvenanceStore } from './provenance';
import type { StoreClient } from './store-client';
import { trustLevelFor } from './trust';

type Sender = Pick<IpcMainInvokeEvent, 'sender' | 'senderFrame'>;

const none = z.union([z.undefined(), z.null(), z.strictObject({})]);
const uuid = z.uuid();
const localId = z.strictObject({ localId: z.string().regex(EXTENSION_ID) });
const releaseRequest = z.strictObject({ releaseId: uuid });
/** Longest cursor accepted from the renderer; the registry's are far shorter. */
const CURSOR_MAX = 512;
/** Longest source path asked for (the registry's tree paths are capped well below). */
const PATH_MAX = 1024;
/** Base64 of at most 256 KiB. */
const ICON_BASE64_MAX = Math.ceil(PUBLISH_LIMITS.iconBytes / 3) * 4;

const publishForm = z.strictObject({
  version: z.string().regex(EXTENSION_VERSION),
  notes: z.string().max(PUBLISH_LIMITS.notesChars).optional(),
  listing: z.strictObject({
    name: z.string().trim().min(1).max(PUBLISH_LIMITS.nameChars),
    tagline: z.string().max(PUBLISH_LIMITS.taglineChars),
    category: Category,
    licence: z.enum(PUBLISH_LICENCES)
  }).optional(),
  visibility: Visibility.optional(),
  iconPng: z.string().max(ICON_BASE64_MAX).regex(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/).optional(),
  waivers: z.array(z.strictObject({
    path: z.string().min(1).max(PATH_MAX),
    line: z.number().int().positive().max(10_000_000),
    reason: z.string().trim().min(PUBLISH_LIMITS.waiverReasonMin).max(PUBLISH_LIMITS.waiverReasonMax)
  })).max(1000)
});

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
  'store:check-updates': none,
  'store:publish-prepare': localId,
  'store:publish': z.strictObject({ localId: z.string().regex(EXTENSION_ID), form: publishForm }),
  'store:yank': z.strictObject({ repoId: uuid, version: z.string().regex(EXTENSION_VERSION) }),
  'store:trust': localId,
  'store:untrust': localId
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
  /** The folder's snapshot tree, when it was taken (folders with provenance); null when unreadable. */
  tree?(localId: string): string | null | undefined;
}

/**
 * What publishing a user folder now would do (store plan §2.2): a folder made
 * here or a changed install of someone else's release makes a new repo; a
 * folder already published, or my own release installed here, adds a release
 * when its tree moved on. Signed out or without a handle, nothing is mine to
 * publish.
 */
function publishStateOf(record: ExtensionRecord, provenance: ProvenanceRecord | undefined, input: LibraryInput, modified: boolean): LibraryItemDto['publish'] {
  const mine = input.me?.publisher?.id ?? null;
  if (record.scope !== 'user' || !mine) return null;
  const published = provenance?.published?.ownerPublisherId === mine ? provenance.published : undefined;
  const origin = provenance?.origin;
  if (published) {
    const base = published.localTreeSha ?? published.publishedTreeSha;
    const tree = input.tree?.(record.id);
    return base && typeof tree === 'string' && tree !== base ? 'update' : null;
  }
  if (origin) return modified ? (origin.ownerPublisherId === mine ? 'update' : 'first') : null;
  return 'first';
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
      trust: record.trust ?? trustLevelFor(record, provenance, input.me),
      permissions: [...(record.manifest?.permissions ?? [])],
      description: record.manifest?.description ?? null,
      group,
      maker,
      update,
      modified
    };
    if (origin) item.origin = { coordinate: origin.coordinate, version: origin.version, repoId: origin.repoId, releaseId: origin.releaseId };
    if (record.manifest?.forkedFrom) item.forkedFrom = record.manifest.forkedFrom;
    const published = provenance?.published;
    if (published) {
      item.published = { coordinate: published.coordinate ?? null, version: published.version, releaseId: published.releaseId, repoId: published.repoId };
      if (origin && origin.ownerPublisherId !== published.ownerPublisherId) {
        item.fork = {
          coordinate: origin.coordinate,
          version: origin.version,
          releaseId: origin.releaseId,
          upstreamReleaseId: provenance.upstream?.releaseId ?? origin.releaseId
        };
      }
    }
    if (record.scope === 'user') item.publish = publishStateOf(record, provenance, input, modified);
    if (check?.state === 'removed' || check?.state === 'tombstoned') item.removed = true;
    return item;
  });
}

/* ── handlers ────────────────────────────────────────────── */

export interface StoreIpcOptions {
  store: StoreClient;
  installer: StoreInstaller;
  publisher: Publisher;
  provenance: ProvenanceStore;
  registry: {
    list(): ExtensionRecord[];
    refresh(ids?: string[]): Promise<void>;
    emitChanged(event: ExtensionsChangedEvent): void;
  };
  me(): MeDto | null;
  /**
   * The native "Give <name> full access to Powermove?" dialog (`trustDialog`),
   * owned by main so no renderer can answer it. Resolves true for Trust.
   */
  confirmTrust(name: string): Promise<boolean>;
  now?(): number;
  /** Removes a user extension through the `ext:remove` path for this sender (asks about values). */
  removeExtension(event: IpcMainInvokeEvent, localId: string): Promise<boolean>;
  isTrusted(event: Sender): boolean;
}

export function registerStoreIpc(ipcMain: Pick<IpcMain, 'handle'>, options: StoreIpcOptions): void {
  const { store, installer } = options;
  /* Snapshotting a folder reads every file; the answer holds until the
     registry sees that folder change (a new `updatedAt`). */
  const treeCache = new Map<string, { key: number; tree: string | null }>();

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
    const trees = new Map<string, string | null>();
    await Promise.all(records.map(async (record) => {
      const entry = record.scope === 'user' ? provenance[record.id] : undefined;
      if (!entry?.origin && !entry?.published) return;
      const cached = treeCache.get(record.id);
      if (cached?.key === record.updatedAt) {
        trees.set(record.id, cached.tree);
        return;
      }
      const tree = await installer.localTree(record.id);
      treeCache.set(record.id, { key: record.updatedAt, tree });
      trees.set(record.id, tree);
    }));
    return buildLibrary({
      records,
      provenance,
      updates: installer.updates(),
      me: options.me(),
      // Unreadable is not what was installed.
      modified: (id) => {
        const origin = provenance[id]?.origin;
        return !!origin && trees.has(id) && trees.get(id) !== origin.treeSha;
      },
      tree: (id) => trees.get(id)
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
    treeCache.delete(request.localId);
    return result;
  }));
  handle(STORE_IPC.uninstall, storeSchemas['store:uninstall'], (request, event) => storeResult(async () => {
    treeCache.delete(request.localId);
    return installer.uninstall(request.localId, (id) => options.removeExtension(event, id));
  }));
  handle(STORE_IPC.library, storeSchemas['store:library'], () => library());
  handle(STORE_IPC.checkUpdates, storeSchemas['store:check-updates'], () => storeResult(() => installer.checkUpdates()));
  handle(STORE_IPC.publishPrepare, storeSchemas['store:publish-prepare'], (request) => storeResult(() => options.publisher.prepare(request.localId)));
  handle(STORE_IPC.publish, storeSchemas['store:publish'], (request) => storeResult(async () => {
    const { form } = request;
    const result = await options.publisher.publish(request.localId, {
      version: form.version,
      waivers: form.waivers,
      ...(form.notes !== undefined ? { notes: form.notes } : {}),
      ...(form.listing ? { listing: form.listing } : {}),
      ...(form.visibility ? { visibility: form.visibility } : {}),
      ...(form.iconPng ? { iconPng: form.iconPng } : {})
    });
    treeCache.delete(request.localId);
    return result;
  }));
  handle(STORE_IPC.yank, storeSchemas['store:yank'], (request) => storeResult(() => options.publisher.yank({ repoId: request.repoId, version: request.version })));

  /* Trust: only a store install someone else wrote can be trusted, and only
     the native dialog can say yes. */
  async function storeInstall(localId: string): Promise<{ record: ExtensionRecord; entry: ProvenanceRecord }> {
    const record = options.registry.list().find((candidate) => candidate.id === localId && candidate.scope === 'user');
    const entry = record ? await options.provenance.get(localId) : null;
    if (!record || !entry?.origin) throw new StoreLocalError('not_installed', 'That extension isn’t installed from the store on this Mac.');
    return { record, entry };
  }
  async function applyTrust(localId: string): Promise<void> {
    await options.registry.refresh([localId]);
    options.registry.emitChanged({ ids: [localId], reason: 'reload' });
  }
  handle(STORE_IPC.trust, storeSchemas['store:trust'], (request) => storeResult(async (): Promise<StoreTrustResult> => {
    const { localId } = request;
    const { record, entry } = await storeInstall(localId);
    const level = trustLevelFor(record, entry, options.me());
    if (level === 'store-trusted') return { localId, trusted: true };
    if (level !== 'store') throw new StoreLocalError('not_installed', 'Extensions you made don’t need your trust.');
    if (!(await options.confirmTrust(record.manifest?.name ?? localId))) return { localId, trusted: false };
    const permissions = [...(record.manifest?.permissions ?? [])];
    const at = new Date((options.now ?? Date.now)()).toISOString();
    await options.provenance.update(localId, (current) => current && { ...current, trusted: { at, permissions } });
    await applyTrust(localId);
    return { localId, trusted: true };
  }));
  handle(STORE_IPC.untrust, storeSchemas['store:untrust'], (request) => storeResult(async (): Promise<StoreTrustResult> => {
    const { localId } = request;
    const { entry } = await storeInstall(localId);
    if (entry.trusted) {
      await options.provenance.update(localId, (current) => {
        if (!current) return null;
        const { trusted: _revoked, ...rest } = current;
        return rest;
      });
      await applyTrust(localId);
    }
    return { localId, trusted: false };
  }));
}
