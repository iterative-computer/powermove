/*
 * `cloud:*` account handlers. Renderer-reachable IPC is hostile (store plan
 * §2.6): every payload is zod-validated and the sender must be a trusted app
 * frame. Registry errors return as `{ ok: false, error }`; malformed payloads
 * and untrusted senders throw. Deleting the account is confirmed by a native
 * dialog owned by main, so no renderer can delete it by calling the channel.
 */
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { ApiError, type ApiErrorBody } from '@powermove/registry/wire';
import { z } from 'zod';

import { IpcValidationError } from '../../shared/guards';
import {
  CLOUD_EMAIL_MAX,
  CLOUD_HANDLE,
  CLOUD_IPC,
  CLOUD_OTP,
  CLOUD_REGISTRY_URL_MAX,
  type CloudAccount,
  type CloudChannels,
  type CloudRegistry,
  type CloudResult
} from '../../shared/cloud-ipc';
import type { CloudAuth } from './auth';
import type { Aerolite } from './aerolite';
import { PLAN_TIERS } from '../../shared/aerolite';
import { normalizeOrigin } from './client';
import { DEFAULT_REGISTRY_ORIGIN } from './registry-url';
import type { CloudSession } from './session';

type Sender = Pick<IpcMainInvokeEvent, 'sender' | 'senderFrame'>;

export interface CloudIpcOptions {
  auth: CloudAuth;
  session: CloudSession;
  /** Settings › Advanced › Registry URL. `change` shows main's confirmation and signs out first. */
  registry: { get(): string; fromEnvironment?(): boolean; change(origin: string): Promise<boolean> };
  isTrusted(event: Sender): boolean;
  aerolite?: Aerolite;
}

/** An http(s) URL a registry can live at; anything else is refused before main asks. */
function isRegistryUrl(value: string): boolean {
  try {
    normalizeOrigin(value);
    return true;
  } catch {
    return false;
  }
}

const email = z.email().max(CLOUD_EMAIL_MAX);
const none = z.union([z.undefined(), z.null(), z.strictObject({})]);
export const cloudSchemas = {
  'cloud:account-get': none,
  'cloud:sign-in-social': z.strictObject({ provider: z.enum(['google']) }),
  'cloud:email-send': z.strictObject({ email }),
  'cloud:email-verify': z.strictObject({ email, otp: z.string().regex(CLOUD_OTP) }),
  'cloud:claim-handle': z.strictObject({ handle: z.string().regex(CLOUD_HANDLE) }),
  'cloud:set-remember-installs': z.strictObject({ value: z.boolean() }),
  'cloud:sign-out': none,
  'cloud:delete-account': none,
  'cloud:registry-url': none,
  'cloud:set-registry-url': z.strictObject({ origin: z.string().trim().min(1).max(CLOUD_REGISTRY_URL_MAX).refine(isRegistryUrl, 'invalid registry URL') }),
  'cloud:plan-get': none,
  'cloud:checkout': z.strictObject({ tier: z.enum(PLAN_TIERS) }),
  'cloud:billing-portal': none,
  'cloud:shares-list': none,
  // Shape is checked again by the cloud; here only size and kind, so a renderer cannot push arbitrary payloads.
  'cloud:share-create': z.strictObject({
    snapshot: z.looseObject({ kind: z.enum(['project', 'extension']), title: z.string().min(1).max(120) }).refine(value => JSON.stringify(value).length <= 256_000, 'snapshot too large'),
    poster: z.string().max(2_800_000).optional()
  }),
  'cloud:share-revoke': z.strictObject({ id: z.string().regex(/^[A-Za-z0-9]{6,32}$/) })
} as const satisfies Record<keyof CloudChannels, z.ZodType>;

function parse<T>(schema: z.ZodType<T>, channel: string, payload: unknown): T {
  const result = schema.safeParse(payload);
  if (!result.success) throw new IpcValidationError(channel, result.error.issues.map((issue) => issue.message).join('; '));
  return result.data;
}

async function result<T>(run: () => Promise<T>): Promise<CloudResult<T>> {
  try {
    return { ok: true, value: await run() };
  } catch (error) {
    const body: ApiErrorBody = error instanceof ApiError
      ? error.body
      : { error: 'internal', detail: 'Something went wrong on this Mac. Try again.' };
    if (!(error instanceof ApiError)) console.error('[cloud] account action failed', error instanceof Error ? error.message : 'unknown error');
    return { ok: false, error: body };
  }
}

export function registerCloudIpc(ipcMain: Pick<IpcMain, 'handle'>, options: CloudIpcOptions): void {
  const { auth, session } = options;

  function handle<C extends keyof CloudChannels, T>(
    channel: C,
    schema: z.ZodType<T>,
    run: (request: T) => Promise<CloudChannels[C]['res']>
  ): void {
    ipcMain.handle(channel, async (event: IpcMainInvokeEvent, payload: unknown) => {
      if (!options.isTrusted(event)) throw new IpcValidationError(channel, 'untrusted sender');
      return run(parse(schema, channel, payload));
    });
  }

  handle(CLOUD_IPC.accountGet, cloudSchemas['cloud:account-get'], async (): Promise<CloudAccount> => {
    // Offline or a registry hiccup: the cached account still answers.
    const me = session.currentToken() ? await session.refreshMe().catch(() => session.me()) : null;
    const provider = me ? session.provider() : undefined;
    return provider ? { me, provider } : { me };
  });
  handle(CLOUD_IPC.signInSocial, cloudSchemas['cloud:sign-in-social'], (request) => result(async () => { await auth.beginSocialSignIn(request.provider); return null; }));
  handle(CLOUD_IPC.emailSend, cloudSchemas['cloud:email-send'], (request) => result(async () => { await auth.sendEmailCode(request.email); return null; }));
  handle(CLOUD_IPC.emailVerify, cloudSchemas['cloud:email-verify'], (request) => result(() => auth.verifyEmailCode(request.email, request.otp)));
  handle(CLOUD_IPC.claimHandle, cloudSchemas['cloud:claim-handle'], (request) => result(() => auth.claimHandle(request.handle)));
  handle(CLOUD_IPC.setRememberInstalls, cloudSchemas['cloud:set-remember-installs'], (request) => result(() => auth.setRememberInstalls(request.value)));
  handle(CLOUD_IPC.signOut, cloudSchemas['cloud:sign-out'], () => result(async () => { await auth.signOut(); return null; }));
  handle(CLOUD_IPC.deleteAccount, cloudSchemas['cloud:delete-account'], () => result(async () => ({ deleted: await auth.deleteAccount() })));
  const registry = (): CloudRegistry => {
    const origin = options.registry.get();
    return { origin, isDefault: origin === DEFAULT_REGISTRY_ORIGIN,
      ...(options.registry.fromEnvironment?.() ? { fromEnvironment: true } : {}) };
  };
  handle(CLOUD_IPC.registryUrl, cloudSchemas['cloud:registry-url'], async () => registry());
  handle(CLOUD_IPC.setRegistryUrl, cloudSchemas['cloud:set-registry-url'], (request) => result(async () => {
    const changed = await options.registry.change(request.origin);
    return { ...registry(), changed };
  }));
  const aerolite = options.aerolite;
  const needsAerolite = () => { if (!aerolite) throw new ApiError({ error: 'internal', detail: 'Plans are unavailable in this build.' }); return aerolite; };
  handle(CLOUD_IPC.planGet, cloudSchemas['cloud:plan-get'], () => result(() => needsAerolite().plan()));
  handle(CLOUD_IPC.checkout, cloudSchemas['cloud:checkout'], (request) => result(async () => { await needsAerolite().checkout(request.tier); return null; }));
  handle(CLOUD_IPC.billingPortal, cloudSchemas['cloud:billing-portal'], () => result(async () => { await needsAerolite().portal(); return null; }));
  handle(CLOUD_IPC.sharesList, cloudSchemas['cloud:shares-list'], () => result(() => needsAerolite().shares()));
  handle(CLOUD_IPC.shareCreate, cloudSchemas['cloud:share-create'], (request) => result(() => needsAerolite().createShare(request as never)));
  handle(CLOUD_IPC.shareRevoke, cloudSchemas['cloud:share-revoke'], (request) => result(async () => { await needsAerolite().revokeShare(request.id); return null; }));
}
