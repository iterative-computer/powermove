/* View models for the Store, built from the registry's DTOs and the Library
   main assembles from this Mac (LibraryItemDto). Nothing here talks to a
   server; StoreScreen.svelte calls the `extensionStore` bridge and hands the
   answers to these functions. */
import type { ExtensionDetailDto, ListingDto, VarDecl } from '@powermove/registry/wire';

import { EXTENSION_API_VERSION } from '../../../shared/extensions';
import type { ExtensionPermission, ExtensionRecord } from '../../../shared/extensions';
import { CLOUD_UNREACHABLE } from '../../../shared/cloud-ipc';
import type { LibraryItemDto, StoreBridge, StoreCategory, StoreErrorBody } from '../../../shared/store-ipc';
import type { PMRegistry } from '../legacy/registry';
import type { CloudUser } from '../cloud/account';
import { bridge } from '../kernel/bridge';

export type StoreKind = StoreCategory;

/* ── the PM surfaces the Store uses ── */

export type StorePage = 'browse' | 'library' | `kind:${StoreKind}`;

export type StoreUISurface = {
  open(page?: StorePage): void;
  close(): void;
  readonly isOpen: boolean;
};

export type StorePM = PMRegistry & {
  Vars?: { openSetup(record: ExtensionRecord): void };
  Account?: { readonly user: CloudUser | null; signIn(): void };
  StoreUI?: StoreUISurface;
};

/* ── kinds ── */

export const KIND_LABEL: Record<StoreKind, string> = {
  effects: 'Effect',
  transitions: 'Transition',
  panels: 'Panel',
  themes: 'Theme',
  commands: 'Commands',
  layers: 'Layer',
  tools: 'Tool'
};

export const KIND_PLURAL: Record<StoreKind, string> = {
  effects: 'Effects',
  transitions: 'Transitions',
  panels: 'Panels',
  themes: 'Themes',
  commands: 'Commands',
  layers: 'Layers',
  tools: 'Tools'
};

export const KIND_ICON: Record<StoreKind, string> = {
  effects: 'sparkle',
  transitions: 'next',
  panels: 'panel',
  themes: 'sun',
  commands: 'return',
  layers: 'layers',
  tools: 'tools'
};

export const KINDS: StoreKind[] = ['effects', 'transitions', 'panels', 'themes', 'commands', 'layers', 'tools'];

export function isKind(value: string): value is StoreKind {
  return KINDS.some((kind) => kind === value);
}

/* ── small helpers ── */

const MONTH_DAY = new Intl.DateTimeFormat('en', { month: 'short', day: 'numeric' });
const MONTH_DAY_YEAR = new Intl.DateTimeFormat('en', { month: 'short', day: 'numeric', year: 'numeric' });
const DAY_MS = 24 * 60 * 60 * 1000;

/** "Today", "Yesterday", "3 days ago", then "Sep 19" (and the year once it isn't this one). */
export function relativeDate(iso: string, now: Date = new Date()): string {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return '';
  const startOf = (date: Date): number => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(then)) / DAY_MS);
  if (days <= 0) return 'Today';
  if (days === 1) return 'Yesterday';
  if (days < 7) return `${days} days ago`;
  return then.getFullYear() === now.getFullYear() ? MONTH_DAY.format(then) : MONTH_DAY_YEAR.format(then);
}

/** Two hues for the preview artwork, fixed per repo until icons ship. */
export function artFor(seed: string): [string, string] {
  let hash = 0x811c9dc5;
  for (let index = 0; index < seed.length; index++) {
    hash ^= seed.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  const first = hash % 360;
  const second = (first + 20 + ((hash >>> 9) % 60)) % 360;
  const lightness = 45 + ((hash >>> 17) % 16);
  return [`oklch(${lightness}% .11 ${first})`, `oklch(${lightness + 22}% .08 ${second})`];
}

/** What the extension adds, as a person would say it. */
const CONTRIBUTION_LABEL: Record<string, string> = {
  panels: 'Panels',
  inspector: 'Inspector',
  media: 'Media',
  commands: 'Commands',
  keybindings: 'Shortcuts',
  effects: 'Effects',
  transitions: 'Transitions',
  layers: 'Layers',
  themes: 'Themes',
  palette: 'Command palette',
  menus: 'Menus',
  status: 'Status bar',
  hooks: 'Hooks'
};

export function includesText(contributes: readonly string[]): string {
  return contributes.map((kind) => CONTRIBUTION_LABEL[kind] ?? kind).join(', ');
}

/** API versions are capabilities, not app release numbers. */
export function requiresText(apiVersion: number): string {
  return apiVersion >= 1 && apiVersion <= EXTENSION_API_VERSION
    ? 'Supported by this version of Powermove'
    : 'A newer version of Powermove';
}

/* ── permissions ── */

/** What an extension declares it uses, as the Store discloses it. */
const PERMISSION_LABEL: Record<ExtensionPermission, string> = {
  network: 'Uses the network',
  clipboard: 'Uses the clipboard',
  assets: 'Imports files',
  'project:write': 'Edits your project',
  'full-access': 'Needs full access to Powermove'
};
const PERMISSION_ORDER: ExtensionPermission[] = ['full-access', 'project:write', 'assets', 'network', 'clipboard'];

export type PermissionLine = { label: string; warn: boolean };

/** One entry per declared permission, full access first (it's the one that matters). */
export function permissionLines(permissions: readonly string[] | undefined): PermissionLine[] {
  const declared = new Set(permissions ?? []);
  return PERMISSION_ORDER.filter((permission) => declared.has(permission))
    .map((permission) => ({ label: PERMISSION_LABEL[permission], warn: permission === 'full-access' }));
}

export function asksFullAccess(permissions: readonly string[] | undefined): boolean {
  return (permissions ?? []).includes('full-access');
}

/* ── listings ── */

export type Lineage = { handle: string; slug: string; version: string; releaseId: string | null };

export type StoreListing = {
  repoId: string;
  publisher: string;
  /** The slug: with the publisher, the coordinate `publisher/id`. */
  id: string;
  name: string;
  tagline: string;
  kind: StoreKind;
  version: string;
  /** Relative, for display: "Today", "Sep 19". */
  updated: string;
  installed: boolean;
  latestReleaseId: string | null;
  apiVersion: number | null;
  /** Registry lineage: the release this was forked from. */
  forkedFrom?: Lineage;
  /** What the latest release declares it uses (apiVersion 3). */
  permissions: ExtensionPermission[];
  /** Variables the extension reads at runtime (from the latest release). */
  vars?: VarDecl[];
  art: [string, string];
  iconUrl: string | null;
  installCount: number;
  forkCount: number;
  visibility: 'public' | 'unlisted';
};

export function coordinate(l: { publisher: string; id: string }): string {
  return `${l.publisher}/${l.id}`;
}

/** The Library item for this repo: published from this Mac, or installed from it. */
export function libraryItemFor(repoId: string, library: readonly LibraryItemDto[], listing?: { publisher: string; id: string }): LibraryItemDto | undefined {
  return library.find((item) => item.published?.repoId === repoId)
    ?? library.find((item) => item.origin?.repoId === repoId)
    ?? (listing?.publisher === 'powermove'
      ? library.find((item) => item.group === 'builtin' && item.localId === listing.id)
      : undefined);
}

/** Local authorship does not grant ownership of a Store repository. */
export function ownsListing(listing: { publisher: string } | null, account: { handle: string | null } | null): boolean {
  return !!listing && !!account?.handle && listing.publisher === account.handle;
}

export function listingFromDto(dto: ListingDto, library: readonly LibraryItemDto[], now: Date = new Date()): StoreListing {
  const listing: StoreListing = {
    repoId: dto.repoId,
    publisher: dto.owner.handle,
    id: dto.slug,
    name: dto.name,
    tagline: dto.tagline,
    kind: dto.category,
    version: dto.latest?.version ?? '',
    updated: relativeDate(dto.latest?.publishedAt ?? dto.updatedAt, now),
    installed: libraryItemFor(dto.repoId, library, { publisher: dto.owner.handle, id: dto.slug }) !== undefined,
    latestReleaseId: dto.latest?.id ?? null,
    apiVersion: dto.latest?.apiVersion ?? null,
    permissions: [...(dto.permissions ?? [])],
    art: artFor(dto.repoId),
    iconUrl: dto.iconUrl,
    installCount: dto.installCount,
    forkCount: dto.forkCount,
    visibility: dto.visibility
  };
  if (dto.forkedFrom) {
    listing.forkedFrom = { handle: dto.forkedFrom.handle, slug: dto.forkedFrom.slug, version: dto.forkedFrom.version, releaseId: dto.forkedFrom.releaseId };
  }
  return listing;
}

/* ── detail ── */

export type VersionEntry = { id: string; version: string; date: string; note: string | null; withdrawn: boolean };

export type StoreDetail = StoreListing & {
  about: string | null;
  contributes: string[];
  versions: VersionEntry[];
};

export function detailFromDto(dto: ExtensionDetailDto, library: readonly LibraryItemDto[], now: Date = new Date()): StoreDetail {
  const listing = listingFromDto(dto, library, now);
  const latest = dto.releases.find((release) => release.id === dto.latest?.id) ?? dto.releases.find((release) => release.yankedAt === null);
  if (latest?.manifest.vars.length) listing.vars = latest.manifest.vars;
  return {
    ...listing,
    about: dto.about ?? latest?.manifest.description ?? null,
    contributes: latest?.manifest.contributes ?? [],
    versions: dto.releases.map((release) => ({
      id: release.id,
      version: release.version,
      date: relativeDate(release.publishedAt, now),
      note: release.notes,
      withdrawn: release.yankedAt !== null
    }))
  };
}

/** `forkedFrom` from a manifest: a store coordinate links; a built-in's does not. */
export function parseLineage(forkedFrom: string): { store: Lineage } | { builtin: { id: string; version: string } } | null {
  const at = forkedFrom.lastIndexOf('@');
  if (at < 0) return null;
  const version = forkedFrom.slice(at + 1);
  const origin = forkedFrom.slice(0, at);
  const slash = origin.indexOf('/');
  if (slash < 0) return origin ? { builtin: { id: origin, version } } : null;
  const handle = origin.slice(0, slash);
  const slug = origin.slice(slash + 1);
  return handle && slug ? { store: { handle, slug, version, releaseId: null } } : null;
}

/* ── the Library ── */

export type LibraryGroups = { store: LibraryItemDto[]; yours: LibraryItemDto[]; builtin: LibraryItemDto[] };

/** Store installs first (they change under you), yours next, built-ins last. */
export function groupLibrary(items: readonly LibraryItemDto[]): LibraryGroups {
  const byName = (a: LibraryItemDto, b: LibraryItemDto): number => a.name.localeCompare(b.name);
  return {
    store: items.filter((item) => item.group === 'store').sort(byName),
    yours: items.filter((item) => item.group === 'yours').sort(byName),
    builtin: items.filter((item) => item.group === 'builtin')
  };
}

export function needsSetup(item: LibraryItemDto): boolean {
  return item.health.state === 'needs-setup';
}

/** Someone else's extension that declares full access and hasn't been trusted: it stays off. */
export function needsTrust(item: LibraryItemDto): boolean {
  return item.health.state === 'needs-trust';
}

export function needsAttention(item: LibraryItemDto): boolean {
  return needsSetup(item) || needsTrust(item) || (!!item.update && item.update.state === 'available');
}

/** Who made it, after the name: "by mara", "By you", "Built in". */
export function makerText(item: LibraryItemDto): string {
  if ('handle' in item.maker) return `by ${item.maker.handle}`;
  if ('builtin' in item.maker) return 'Built in';
  return 'By you';
}

/** The third line of a Library row: what's up with it, or nothing. A trusted install says so. */
export function statusText(item: LibraryItemDto): string | null {
  const status = baseStatusText(item);
  if (item.trust !== 'store-trusted' || needsTrust(item)) return status;
  return status ? `${status} · Trusted` : 'Trusted';
}

function baseStatusText(item: LibraryItemDto): string | null {
  if (item.removed) return 'No longer on the store';
  if (needsTrust(item)) return 'Needs full access';
  if (needsSetup(item)) return 'Needs setup';
  if (item.update?.state === 'staged-for-merge') return `You changed the files · ${item.update.version} is beside your folder`;
  if (item.update?.modified) return `You changed the files · Update to ${item.update.version}`;
  if (item.update) return `Installed ${item.origin?.version ?? item.version} · Update to ${item.update.version}`;
  if (item.published && item.group === 'yours') {
    return item.publish ? `Published ${item.published.version} · Changed since` : `Published ${item.published.version}`;
  }
  if (item.modified) return 'You changed the files';
  if (item.group === 'store') return `Installed ${item.origin?.version ?? item.version}`;
  if (item.published) return `Published ${item.published.version}`;
  return null;
}

export function statusIsHot(item: LibraryItemDto): boolean {
  return needsAttention(item);
}

/* ── actions ── */

export type ActionKind = 'install' | 'setup' | 'trust' | 'update' | 'publish' | 'none' | 'toggle';

export type Action = {
  label: string;
  kind: ActionKind;
  primary?: boolean;
  /** Drawn as quiet text, not a button. */
  quiet?: boolean;
  disabled?: boolean;
};

/**
 * The one action for an extension, from what the store says and what is on
 * this Mac:
 *
 *   not here, no declared values     Install
 *   not here, declared values      Install and set up
 *   here, needs full access        Trust…
 *   here, values missing           Set up
 *   here, newer version            Update
 *   here, newer version, changed   Update…
 *   here, up to date               Installed
 *   yours, changed since publishing  Publish… / Publish Update…
 *   the store page of what you forked  Forked
 *   yours or built in              Open (not yet)
 */
export function detailAction(input: { vars?: readonly VarDecl[] | undefined; item?: LibraryItemDto | undefined; repoId?: string | undefined }): Action {
  const { item } = input;
  if (!item) {
    return input.vars?.length
      ? { label: 'Install and set up', kind: 'install', primary: true }
      : { label: 'Install', kind: 'install', primary: true };
  }
  if (item.group === 'builtin') return { label: 'Built in', kind: 'none', quiet: true };
  if (needsTrust(item)) return TRUST_ACTION;
  if (needsSetup(item)) return { label: 'Set up', kind: 'setup', primary: true };
  // The original's page, seen from your published fork of it.
  if (item.fork && input.repoId && input.repoId !== item.published?.repoId) return { label: 'Forked', kind: 'none', quiet: true };
  if (item.update && !item.removed) return { label: item.update.modified ? 'Update…' : 'Update', kind: 'update', primary: true };
  if (item.group === 'yours' && item.publish) return { ...publishAction(item), primary: true };
  if (item.group === 'store') return { label: 'Installed', kind: 'none', quiet: true };
  return { label: 'Open', kind: 'none', disabled: true };
}

const TRUST_ACTION: Action = { label: 'Trust…', kind: 'trust', primary: true };

/** Publish… for something new; Publish Update… once it is on the store. */
export function publishAction(item: LibraryItemDto): Action {
  return { label: item.publish === 'update' ? 'Publish Update…' : 'Publish…', kind: 'publish' };
}

/**
 * A second, quieter way to publish from the detail page when the primary
 * action is something else: your changed copy of someone else's extension
 * (it publishes as a fork) or your own extension installed from the store.
 */
export function secondaryPublish(item: LibraryItemDto | undefined): Action | null {
  if (!item || item.group !== 'store' || !item.publish) return null;
  return item.publish === 'first' ? { label: 'Publish Your Version…', kind: 'publish' } : publishAction(item);
}

/** The trailing control of a Library row: Trust, Update, Set up, Publish, or On/Off. */
export function libraryAction(item: LibraryItemDto): Action {
  if (needsTrust(item)) return TRUST_ACTION;
  if (needsSetup(item)) return { label: 'Set up', kind: 'setup', primary: true };
  if (item.update && item.update.state === 'available' && !item.removed) {
    return { label: item.update.modified ? 'Update…' : 'Update', kind: 'update', primary: true };
  }
  if (item.group === 'yours' && item.publish) return publishAction(item);
  return { label: item.enabled ? 'On' : 'Off', kind: 'toggle', quiet: true };
}

/* ── errors ── */

export type LoadError = { offline: boolean; message: string };

export function loadError(error: StoreErrorBody): LoadError {
  if (error.error === 'internal' && error.detail === CLOUD_UNREACHABLE) return { offline: true, message: 'Can’t reach the store' };
  return { offline: false, message: actionErrorText(error) };
}

/** A failure, as one sentence for the person who asked. */
export function actionErrorText(error: StoreErrorBody): string {
  if (error.error === 'internal' && error.detail === CLOUD_UNREACHABLE) return 'Can’t reach the store. Check your connection and try again.';
  if (error.error === 'gone') return 'This extension is no longer on the store.';
  if (error.error === 'not_found') return 'This extension isn’t on the store.';
  if (error.error === 'rate_limited') return 'Too many requests. Wait a moment and try again.';
  if (error.error === 'client_too_old') return 'This version of Powermove is too old for the store. Update Powermove and try again.';
  return error.detail ?? 'Something went wrong. Try again.';
}

/** A publish or withdraw failure: main already wrote the sentence into `detail`. */
export function publishErrorText(error: StoreErrorBody): string {
  if (error.error === 'internal' && error.detail === CLOUD_UNREACHABLE) return 'Can’t reach the store. Check your connection and try again.';
  return error.detail ?? actionErrorText(error);
}

/* ── the bridge ── */

export function storeBridge(): StoreBridge | null {
  return bridge()?.extensionStore ?? null;
}
