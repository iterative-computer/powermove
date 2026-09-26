/* The Powermove Cloud account, as the renderer sees it. Main owns the session
   (src/main/cloud); this module mirrors who is signed in through the
   `cloud:*` bridge and never sees a token. One subscribe/notify pair drives
   every place that shows who you are: the home sidebar, Settings › Accounts
   and the Store's Yours page. */
import { flushSync, mount, unmount } from 'svelte';
import type { MeDto } from '@powermove/registry/wire';

import type { PMRegistry } from '../legacy/registry';
import type { CloudBridge, CloudProvider } from '../../../shared/cloud-ipc';
import { openPopoverMenu, type PopoverMenuHandle } from '../controls/popover-menu';
import Avatar from './Avatar.svelte';
import SignInSheet from './SignInSheet.svelte';
import { bridge as hostBridge } from '../kernel/bridge';

export type AccountProvider = CloudProvider;

/** Who is signed in, derived from `MeDto`. */
export type CloudUser = {
  name: string;
  /** Publisher namespace: extensions publish as `handle/id`. Null until claimed. */
  handle: string | null;
  email: string;
  image: string | null;
};

export type SignInMode = 'sign-in' | 'sign-up';

type Listener = (user: CloudUser | null, me: MeDto | null) => void;

/* The design pass kept a simulated user under this store key. Real sessions
   live in main; the old record is cleared on launch. */
const LEGACY_STORE_KEY = 'cloudAccount';

export const PROVIDER_LABEL: Record<AccountProvider, string> = {
  google: 'Google',
  email: 'email'
};

let registry: PMRegistry | null = null;
let bridge: CloudBridge | null = null;
let me: MeDto | null = null;
let user: CloudUser | null = null;
let provider: AccountProvider | undefined;
let sheet: { close(): void } | null = null;
let offChanged: (() => void) | null = null;
const listeners = new Set<Listener>();

/** The label every surface shows: the handle when claimed (that is the
    publisher identity), otherwise the email's local part until it is. The
    provider's display name is not shown anywhere. */
export function userFromMe(value: MeDto): CloudUser {
  const handle = value.publisher?.handle ?? null;
  const name = handle ? `@${handle}` : value.user.email.split('@')[0] || value.user.email;
  return { name, handle, email: value.user.email, image: value.user.image };
}

function notify(): void {
  for (const listener of listeners) listener(user, me);
}

function setMe(next: MeDto | null): void {
  me = next;
  user = next ? userFromMe(next) : null;
  if (!next) provider = undefined;
  notify();
}

export function currentUser(): CloudUser | null {
  return user;
}

export function currentMe(): MeDto | null {
  return me;
}

export function currentProvider(): AccountProvider | undefined {
  return provider;
}

/** The bridge, or null outside Electron and before install. */
export function cloudBridge(): CloudBridge | null {
  return bridge;
}

/** Calls back now with the current user, then on every change. */
export function subscribeAccount(listener: Listener): () => void {
  listeners.add(listener);
  listener(user, me);
  return () => listeners.delete(listener);
}

/** Apply an account main just returned (verify, claim, settings), ahead of the broadcast. */
export function applyAccount(next: MeDto | null): void {
  setMe(next);
}

export async function signOut(): Promise<void> {
  const PM = registry as any;
  if (!bridge) return;
  try {
    const result = await bridge.signOut();
    if (!result.ok) PM?.toast?.('Unable to sign out. Try again.', 4000, { error: true });
    else setMe(null);
  } catch {
    PM?.toast?.('Unable to sign out. Try again.', 4000, { error: true });
  }
}

export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (!words.length) return '?';
  const first = [...words[0]!][0] ?? '';
  const last = words.length > 1 ? [...words.at(-1)!][0] ?? '' : '';
  return (first + last).toUpperCase();
}

/** The sign-in sheet. One at a time. Signed in without a handle, it opens on
    the handle step so an interrupted sign-up can finish. */
export function openSignIn(mode: SignInMode = 'sign-in'): void {
  const PM = registry as any;
  if (!PM?.modal || sheet || !bridge) return;
  if (me?.publisher) return;
  const body = document.createElement('div');
  let component: ReturnType<typeof mount> | null = null;
  const handle = PM.modal({
    body,
    width: 400,
    actions: [],
    onClose: () => {
      if (component) void unmount(component);
      component = null;
      sheet = null;
    }
  });
  handle.el.classList.add('account-modal');
  sheet = handle;
  component = mount(SignInSheet, {
    target: body,
    props: {
      PM,
      mode,
      bridge,
      me,
      onclose: () => handle.close()
    }
  });
  flushSync();
}

/** The signed-in menu, opened upward from the sidebar's account row. It is
    ours, not an NSMenu: native menus are for right-click. Who you are heads
    it, avatar beside name and email, above the rows. */
export function openAccountMenu(anchor: HTMLElement, onClose?: () => void): PopoverMenuHandle | null {
  const PM = registry as any;
  if (!user) return null;
  const header = document.createElement('div');
  header.className = 'acct-menu-head';
  const copy = document.createElement('div');
  const name = document.createElement('b');
  name.textContent = user.name;
  const email = document.createElement('span');
  email.textContent = user.email;
  copy.append(name, email);
  const avatar = mount(Avatar, { target: header, props: { user, size: 28 } });
  header.append(copy);
  return openPopoverMenu({
    anchor,
    label: 'Account',
    header,
    side: 'top',
    items: [
      ...(user.handle ? [] : [{ label: 'Choose Handle…', run: () => openSignIn() }]),
      { label: 'Your Library', run: () => PM?.StoreUI?.open?.('library') },
      { label: 'Account Settings…', run: () => PM?.SettingsUI?.open?.('accounts') },
      '-',
      { label: 'Sign Out', run: () => void signOut() }
    ],
    onClose: () => {
      /* The menu fades out for a beat; the avatar leaves with it. */
      window.setTimeout(() => void unmount(avatar), 220);
      onClose?.();
    }
  });
}

export function installCloudAccount(PM: PMRegistry): void {
  registry = PM;
  const store = (PM as any).store;
  if (store?.get?.(LEGACY_STORE_KEY, null) != null) store.set?.(LEGACY_STORE_KEY, null);
  bridge = hostBridge()?.cloud ?? null;
  (PM as any).Account = {
    get user() { return user; },
    get me() { return me; },
    signIn: openSignIn,
    signOut,
    subscribe: subscribeAccount
  };
  offChanged?.();
  offChanged = bridge?.onAccountChanged((next) => setMe(next)) ?? null;
  notify();
  void bridge?.account().then((account) => {
    provider = account.me ? account.provider : undefined;
    setMe(account.me);
    // Signed in but no handle yet: finish the sign-up, in the window the
    // user is looking at rather than in every restored window at once.
    if (!account.me || account.me.publisher) return;
    const prompt = (): void => { if (me && !me.publisher) openSignIn('sign-up'); };
    if (document.hasFocus()) prompt();
    else window.addEventListener('focus', prompt, { once: true });
  }).catch(() => {
    // No account service (it failed to boot): stay signed out.
  });
}
