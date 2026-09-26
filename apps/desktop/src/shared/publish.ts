/*
 * Publishing an extension to the Store (store plan §2.2, §2.6, §6; P0 Q2, Q3).
 *
 * Shared by main (src/main/cloud/publish.ts), which owns the snapshot, the
 * native confirmation and the upload, and the renderer's publish sheet,
 * which only ever sees a `PublishPlanDto` (no bytes, no tokens) and sends a
 * `PublishForm` back. Main validates the form again; nothing here is trusted
 * on its own.
 */
import type { ApiErrorBody, Category, Visibility } from '@powermove/registry/wire';
import type { ScanKind } from '@powermove/registry/scan';
import type { ExtensionPermission } from './extensions';
export interface PermissionFinding { path: string; line: number; needs: ExtensionPermission; text: string }
import type { z } from 'zod';

import { EXTENSION_VERSION } from './extensions';

export const PUBLISH_LICENCES = ['MIT', 'Apache-2.0', 'BSD-3-Clause', 'GPL-3.0-or-later', 'AGPL-3.0-or-later', 'Unlicense'] as const;
export type PublishLicence = (typeof PUBLISH_LICENCES)[number];

export const PUBLISH_LIMITS = {
  nameChars: 80,
  taglineChars: 160,
  notesChars: 4000,
  iconBytes: 256 * 1024,
  waiverReasonMin: 3,
  waiverReasonMax: 200
} as const;

/** P0 decision Q3: shown above the Publish button. */
export const PUBLISH_TERMS = 'By publishing you grant Powermove the right to host and distribute this extension under the licence you chose, and you confirm you have the right to publish its contents.';

/** Commit identities use the handle at this mail domain (P0 Q4); the registry checks it. */
export const HANDLE_MAIL_DOMAIN = 'users.trypowermove.com';

export type PublishCategory = z.infer<typeof Category>;
export type PublishVisibility = z.infer<typeof Visibility>;

/** One scanner finding, as the sheet shows it. `reason` is an in-file `powermove-secret-ok:` comment, if any. */
export interface PublishFinding {
  path: string;
  line: number;
  kind: ScanKind;
  reason?: string;
}

export interface PublishListing {
  name: string;
  tagline: string;
  category: PublishCategory;
  licence: string;
  about?: string;
}

/** What publishing this folder would do, computed by main from the folder as it is now. */
export interface PublishPlanDto {
  localId: string;
  /** `<handle>/<slug>`. */
  coordinate: string;
  /** The manifest's version. */
  version: string;
  /** What the sheet prefills: the manifest's version, or the next patch when that is already taken. */
  suggestedVersion: string;
  /** The highest version already on the store for this repo, yanked or not. */
  lastVersion: string | null;
  /** No repo yet: the sheet asks for the listing, licence, visibility and icon. */
  firstPublish: boolean;
  treeSha: string;
  fileCount: number;
  sizeBytes: number;
  /** Publishing creates a fork of someone else's release. */
  isFork: boolean;
  origin?: { coordinate: string; version: string };
  /** The commit this one follows, if any. */
  parent?: string;
  /** Hard findings: no waiver. */
  blockedFindings: PublishFinding[];
  /** Undeclared capabilities block publishing until the manifest is fixed. */
  permissionFindings: PermissionFinding[];
  /** Soft findings: each needs a reason, sent as a waiver. One per file and line. */
  waivableFindings: PublishFinding[];
  manifest: { id: string; name: string; description: string | null };
  /** Defaults for a first publish; the current listing for an update (re-sent unchanged). */
  listing: PublishListing;
}

export interface PublishWaiver {
  path: string;
  line: number;
  reason: string;
}

export interface PublishForm {
  version: string;
  notes?: string;
  /** First publish only. */
  listing?: { name: string; tagline: string; category: PublishCategory; licence: PublishLicence };
  /** First publish only. */
  visibility?: PublishVisibility;
  /** First publish only: a square PNG, base64, ≤ 256 KiB. */
  iconPng?: string;
  waivers: PublishWaiver[];
}

export type StorePublishResult =
  | { published: false }
  | { published: true; coordinate: string; version: string; repoId: string; releaseId: string };

/** main → renderer while a publish runs (`store:publish-progress`). Uploads count files (blobs). */
export interface PublishProgress {
  localId: string;
  phase: 'checking' | 'uploading' | 'publishing';
  done: number;
  total: number;
}

/* ── versions ── */

function parts(version: string): [number, number, number] | null {
  if (!EXTENSION_VERSION.test(version)) return null;
  const [a, b, c] = version.split('.').map(Number);
  return [a ?? 0, b ?? 0, c ?? 0];
}

/** Negative, zero or positive, as `a` is lower, equal or higher. Invalid versions sort lowest. */
export function compareVersions(a: string, b: string): number {
  const x = parts(a);
  const y = parts(b);
  if (!x || !y) return (x ? 1 : 0) - (y ? 1 : 0);
  for (let index = 0; index < 3; index++) {
    const diff = x[index]! - y[index]!;
    if (diff) return diff;
  }
  return 0;
}

export function nextPatch(version: string): string {
  const p = parts(version);
  return p ? `${p[0]}.${p[1]}.${p[2] + 1}` : '1.0.0';
}

/** Why `version` can't be published after `last`, as one sentence; null when it can. */
export function versionProblem(version: string, last: string | null): string | null {
  const value = version.trim();
  if (!value) return 'Enter a version.';
  if (!EXTENSION_VERSION.test(value)) return 'Use three numbers, like 1.2.0.';
  if (last && compareVersions(value, last) <= 0) return `Use a version higher than ${last}.`;
  return null;
}

/** Why a waiver reason isn't enough, as one sentence; null when it is. */
export function waiverProblem(reason: string | undefined): string | null {
  const value = (reason ?? '').trim();
  if (value.length < PUBLISH_LIMITS.waiverReasonMin) return 'Say why this is safe to publish.';
  if (value.length > PUBLISH_LIMITS.waiverReasonMax) return `Keep the reason under ${PUBLISH_LIMITS.waiverReasonMax} characters.`;
  return null;
}

export const findingKey = (finding: { path: string; line: number }): string => `${finding.path}:${finding.line}`;

/* ── errors ── */

const LIMIT_TEXT: Record<string, string> = {
  too_many_files: 'Extensions can have at most 400 files. Remove some and try again.',
  tree_too_large: 'Extensions can be at most 8 MB. Remove large files and try again.',
  file_too_large: 'Each file can be at most 2 MB. Remove or shrink the large file and try again.',
  path_too_long: 'A file path is too long. Keep paths under 200 characters.',
  envelope: 'The upload was too large for one request. Try again.'
};

export interface PublishErrorContext {
  coordinate?: string;
  version?: string;
}

/**
 * A publish failure as one sentence for the person publishing. Main writes
 * it into `detail` so every window shows the same words.
 */
export function publishErrorText(error: ApiErrorBody, context: PublishErrorContext = {}): string {
  switch (error.error) {
    case 'head_moved':
      return 'This extension was published from another Mac since you last published here. Reinstall that version from the store first, then make your changes and publish.';
    case 'version_exists':
      return context.version ? `Version ${context.version} is already on the store. Choose a higher version.` : 'That version is already on the store. Choose a higher version.';
    case 'version_invalid':
      return 'Use a version with three numbers, like 1.2.0.';
    case 'author_mismatch':
      return 'Your account changed while publishing. Sign in again, then publish.';
    case 'object_missing':
    case 'commit_missing':
      return 'Some files didn’t finish uploading. Try again.';
    case 'scan_blocked': {
      const first = error.findings[0];
      if (!first) return 'A file looks like it contains a secret. Remove it before publishing.';
      const more = error.findings.length > 1 ? ` and ${error.findings.length - 1} more` : '';
      return `${first.path} line ${first.line}${more} looks like a secret. Remove it before publishing.`;
    }
    case 'same_as_origin':
      return 'Nothing has changed since you installed it. Change something before publishing your own version.';
    case 'self_origin':
      return 'This extension is already yours on the store. Publish an update to it instead.';
    case 'limit_exceeded':
      return LIMIT_TEXT[error.code] ?? 'This extension is too large to publish.';
    case 'too_large':
      return LIMIT_TEXT[error.limit] ?? 'This extension is too large to publish.';
    case 'manifest_invalid':
      return error.detail ? `manifest.json isn’t valid: ${error.detail}.` : 'manifest.json isn’t valid. Fix it and try again.';
    case 'tree_invalid':
      return 'The store couldn’t read these files as an extension. Try again.';
    case 'rate_limited':
      return 'You’ve published a lot in the last hour. Wait a while and try again.';
    case 'quota_exceeded':
      return 'You have too many uploads waiting to be published. Try again tomorrow.';
    case 'not_owner':
      return 'You can only publish under your own handle.';
    case 'forbidden':
      return 'Choose a handle first.';
    case 'unauthorized':
      return 'Sign in to publish.';
    case 'gone':
      return error.reason === 'removed'
        ? 'This extension was removed from the store and can’t be published again.'
        : context.coordinate
          ? `You deleted ${context.coordinate} from the store, so it can’t be published again.`
          : 'You deleted this extension from the store, so it can’t be published again.';
    case 'not_found':
      return 'The version this was installed from is no longer on the store, so it can’t be published as a fork.';
    case 'object_conflict':
      return 'A file clashed with one already on the store. Try again.';
    case 'client_too_old':
      return 'Update Powermove to publish.';
    case 'bad_request':
      return 'The store couldn’t accept this publish. Try again.';
    case 'internal':
      return error.detail ?? 'Something went wrong. Try again.';
    default:
      return error.detail ?? 'The store couldn’t publish this extension. Try again.';
  }
}
