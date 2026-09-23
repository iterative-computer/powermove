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
  type CloudAccount,
  type CloudChannels,
  type CloudResult
} from '../../shared/cloud-ipc';
import type { CloudAuth } from './auth';
import type { CloudSession } from './session';

type Sender = Pick<IpcMainInvokeEvent, 'sender' | 'senderFrame'>;

export interface CloudIpcOptions {
  auth: CloudAuth;
  session: CloudSession;
  isTrusted(event: Sender): boolean;
}

const email = z.email().max(CLOUD_EMAIL_MAX);
const none = z.union([z.undefined(), z.null(), z.strictObject({})]);
export const cloudSchemas = {
  'cloud:account-get': none,
  'cloud:sign-in-social': z.strictObject({ provider: z.enum(['google', 'github']) }),
  'cloud:email-send': z.strictObject({ email }),
  'cloud:email-verify': z.strictObject({ email, otp: z.string().regex(CLOUD_OTP) }),
  'cloud:claim-handle': z.strictObject({ handle: z.string().regex(CLOUD_HANDLE) }),
  'cloud:set-remember-installs': z.strictObject({ value: z.boolean() }),
  'cloud:sign-out': none,
  'cloud:delete-account': none
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
}
