/*
 * Signing in to Powermove Cloud (store plan §2.3).
 *
 * Browser hand-off, the desktop's half of the protocol:
 *   1. `beginSocialSignIn` makes `verifier` (32 random bytes, base64url),
 *      `state` (16 random bytes, hex) and
 *      `challenge = base64url(SHA-256(verifier bytes))`, keeps
 *      `{ state, verifier, expires, provider }` in memory for 10 minutes (one
 *      pending sign-in at a time; starting again replaces it), and opens
 *      `<origin>/v1/auth/desktop?provider&state&challenge` in the browser.
 *   2. The registry signs the user in and opens `powermove://auth?state&token`.
 *   3. `handleDeepLink` requires `state` to equal the pending one (anything
 *      else is ignored, so a stray link cannot cancel a real sign-in), clears
 *      the pending entry, and exchanges `{ state, token, verifier }` for a
 *      bearer session. Another app that registers `powermove://` and catches
 *      the link still cannot finish: it does not have the verifier.
 *
 * Email: `sendEmailCode` then `verifyEmailCode`, which returns a session the
 * same way. Tokens and verifiers are never logged.
 */
import { createHash, randomBytes, timingSafeEqual } from 'node:crypto';
import { ApiError, Auth, Me, type ApiErrorBody, type MeDto, type SessionDto } from '@powermove/registry/wire';

import type { CloudProvider, CloudSocialProvider } from '../../shared/cloud-ipc';
import type { CloudSession } from './session';

export const PENDING_SIGN_IN_MS = 10 * 60 * 1000;
export const DEEP_LINK_SCHEME = 'powermove';

export function base64url(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('base64url');
}

/** `base64url(SHA-256(verifier bytes))`, the server's `challengeOf`. */
export function challengeFor(verifier: string): string {
  return base64url(createHash('sha256').update(Buffer.from(verifier, 'base64url')).digest());
}

interface PendingSignIn {
  state: string;
  verifier: string;
  expires: number;
  provider: CloudSocialProvider;
}

export interface CloudAuthOptions {
  session: CloudSession;
  openExternal(url: string): Promise<void>;
  /** Tell every window who is signed in now. */
  notifyAccount(me: MeDto | null): void;
  /** Tell every window a browser sign-in came back and failed. */
  notifySignInFailed(error: ApiErrorBody): void;
  /** The main-owned native confirmation for deleting the account. */
  confirmDelete(): Promise<boolean>;
  /** Test seams. */
  random?(size: number): Uint8Array;
  now?(): number;
  log?(message: string): void;
}

export interface CloudAuth {
  beginSocialSignIn(provider: CloudSocialProvider): Promise<void>;
  /** A `powermove://…` URL from `open-url` or `second-instance`. Never throws. */
  handleDeepLink(url: string): Promise<void>;
  sendEmailCode(email: string): Promise<void>;
  verifyEmailCode(email: string, otp: string): Promise<MeDto | null>;
  claimHandle(handle: string): Promise<MeDto | null>;
  setRememberInstalls(value: boolean): Promise<MeDto | null>;
  signOut(): Promise<void>;
  /** False when the confirmation was cancelled. */
  deleteAccount(): Promise<boolean>;
  /** Test and diagnostics: whether a browser sign-in is waiting. */
  hasPending(): boolean;
}

function sameText(a: string, b: string): boolean {
  const left = Buffer.from(a);
  const right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}

/** `{ state, token }` from `powermove://auth?state=…&token=…`, or null. */
export function parseAuthLink(url: string): { state: string; token: string } | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== `${DEEP_LINK_SCHEME}:` || parsed.hostname !== 'auth') return null;
  const state = parsed.searchParams.get('state');
  const token = parsed.searchParams.get('token');
  if (!state || !token || state.length > 128 || token.length > 4096) return null;
  return { state, token };
}

export function createCloudAuth(options: CloudAuthOptions): CloudAuth {
  const { session } = options;
  const random = options.random ?? ((size: number) => new Uint8Array(randomBytes(size)));
  const now = options.now ?? Date.now;
  const log = options.log ?? ((message: string) => console.warn(`[cloud] ${message}`));
  let pending: PendingSignIn | null = null;

  /** Store the session, fetch who it belongs to and tell the windows. */
  async function signedIn(dto: SessionDto, provider: CloudProvider): Promise<MeDto | null> {
    await session.save(dto, provider);
    const me = await session.refreshMe();
    options.notifyAccount(me);
    return me;
  }

  async function refreshAndNotify(): Promise<MeDto | null> {
    const me = await session.refreshMe();
    options.notifyAccount(me);
    return me;
  }

  return {
    async beginSocialSignIn(provider) {
      const verifier = base64url(random(32));
      const state = Buffer.from(random(16)).toString('hex');
      const challenge = challengeFor(verifier);
      pending = { state, verifier, expires: now() + PENDING_SIGN_IN_MS, provider };
      const url = session.client().api.v1.auth.desktop.$url({ query: { provider, state, challenge } });
      await options.openExternal(url.toString());
    },

    async handleDeepLink(url) {
      const link = parseAuthLink(url);
      if (!link) {
        log('Ignored a powermove:// link that is not a sign-in.');
        return;
      }
      const current = pending;
      if (!current) {
        log('Ignored a sign-in link: no sign-in is waiting.');
        return;
      }
      if (current.expires <= now()) {
        pending = null;
        log('Ignored a sign-in link: the sign-in expired.');
        return;
      }
      if (!sameText(link.state, current.state)) {
        log('Ignored a sign-in link for a different sign-in.');
        return;
      }
      pending = null;
      try {
        const dto = await session.client().request(Auth.DesktopExchange.Res, (api) =>
          api.v1.auth.desktop.exchange.$post({ json: { state: link.state, token: link.token, verifier: current.verifier } })
        );
        await signedIn(dto, current.provider);
      } catch (error) {
        const body: ApiErrorBody = error instanceof ApiError ? error.body : { error: 'internal' };
        log(`Sign-in exchange failed: ${body.error}`);
        options.notifySignInFailed(body);
      }
    },

    async sendEmailCode(email) {
      await session.client().request(Auth.EmailSend.Res, (api) => api.v1.auth.email.send.$post({ json: { email } }));
    },

    async verifyEmailCode(email, otp) {
      const dto = await session.client().request(Auth.EmailVerify.Res, (api) =>
        api.v1.auth.email.verify.$post({ json: { email, otp } })
      );
      return signedIn(dto, 'email');
    },

    async claimHandle(handle) {
      // Built as a variable, not a literal: the route reads the JSON body
      // itself today, and this stays valid if it gains a json validator.
      const args = { json: { handle } };
      await session.client().request(Me.SetHandle.Res, (api) => api.v1.me.handle.$post(args));
      return refreshAndNotify();
    },

    async setRememberInstalls(value) {
      await session.client().request(Me.Settings.Res, (api) => api.v1.me.settings.$patch({ json: { rememberInstalls: value } }));
      return refreshAndNotify();
    },

    async signOut() {
      await session.signOut();
      options.notifyAccount(null);
    },

    async deleteAccount() {
      if (!session.currentToken()) throw new ApiError({ error: 'unauthorized' });
      if (!(await options.confirmDelete())) return false;
      await session.client().request(null, (api) => api.v1.me.$delete());
      await session.clear();
      options.notifyAccount(null);
      return true;
    },

    hasPending: () => pending !== null
  };
}
