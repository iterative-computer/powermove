/*
 * Powermove Cloud account IPC (store plan §2.3, §2.6, §2.7).
 *
 * Every channel here is renderer-reachable and therefore hostile input: main
 * validates each payload with zod and checks the sender
 * (src/main/cloud/ipc.ts). The bearer token never crosses this boundary;
 * the renderer only ever sees the account (`MeDto`).
 *
 * Registry failures come back as `{ ok: false, error }` results, not thrown
 * errors, so the sign-in sheet can show them next to the field that failed.
 * Malformed payloads and untrusted senders still throw.
 */
import type { ApiErrorBody, MeDto } from '@powermove/registry/wire';

export const CLOUD_IPC = {
  accountGet: 'cloud:account-get',
  signInSocial: 'cloud:sign-in-social',
  emailSend: 'cloud:email-send',
  emailVerify: 'cloud:email-verify',
  claimHandle: 'cloud:claim-handle',
  setRememberInstalls: 'cloud:set-remember-installs',
  signOut: 'cloud:sign-out',
  deleteAccount: 'cloud:delete-account',
  /** main → renderer: the signed-in account changed (`MeDto | null`). */
  accountChanged: 'cloud:account-changed',
  /** main → renderer: a browser sign-in came back and could not be completed (`ApiErrorBody`). */
  signInFailed: 'cloud:sign-in-failed'
} as const;

export type CloudSocialProvider = 'google' | 'github';
export type CloudProvider = CloudSocialProvider | 'email';

export interface CloudAccount {
  me: MeDto | null;
  /** How this Mac signed in, when known. */
  provider?: CloudProvider;
}

export type CloudResult<T> = { ok: true; value: T } | { ok: false; error: ApiErrorBody };

/** Request and response types per invoke channel. */
export interface CloudChannels {
  'cloud:account-get': { req: void; res: CloudAccount };
  'cloud:sign-in-social': { req: { provider: CloudSocialProvider }; res: CloudResult<null> };
  'cloud:email-send': { req: { email: string }; res: CloudResult<null> };
  'cloud:email-verify': { req: { email: string; otp: string }; res: CloudResult<MeDto | null> };
  'cloud:claim-handle': { req: { handle: string }; res: CloudResult<MeDto | null> };
  'cloud:set-remember-installs': { req: { value: boolean }; res: CloudResult<MeDto | null> };
  'cloud:sign-out': { req: void; res: CloudResult<null> };
  /** `deleted: false` means the confirmation was cancelled. */
  'cloud:delete-account': { req: void; res: CloudResult<{ deleted: boolean }> };
}

export type CloudChannel = keyof CloudChannels;

/** Longest email accepted (RFC 5321 path limit). */
export const CLOUD_EMAIL_MAX = 254;
/** The emailed sign-in code. */
export const CLOUD_OTP = /^\d{6}$/;
/** Publisher handles, as the registry enforces them. */
export const CLOUD_HANDLE = /^[a-z0-9][a-z0-9-]{1,38}$/;

export interface CloudBridge {
  account(): Promise<CloudAccount>;
  signInSocial(req: CloudChannels['cloud:sign-in-social']['req']): Promise<CloudChannels['cloud:sign-in-social']['res']>;
  emailSend(req: CloudChannels['cloud:email-send']['req']): Promise<CloudChannels['cloud:email-send']['res']>;
  emailVerify(req: CloudChannels['cloud:email-verify']['req']): Promise<CloudChannels['cloud:email-verify']['res']>;
  claimHandle(req: CloudChannels['cloud:claim-handle']['req']): Promise<CloudChannels['cloud:claim-handle']['res']>;
  setRememberInstalls(req: CloudChannels['cloud:set-remember-installs']['req']): Promise<CloudChannels['cloud:set-remember-installs']['res']>;
  signOut(): Promise<CloudChannels['cloud:sign-out']['res']>;
  deleteAccount(): Promise<CloudChannels['cloud:delete-account']['res']>;
  onAccountChanged(cb: (me: MeDto | null) => void): () => void;
  onSignInFailed(cb: (error: ApiErrorBody) => void): () => void;
}
