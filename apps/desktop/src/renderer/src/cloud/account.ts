/* The Powermove Cloud account. Design pass: signing in is simulated and the
   signed-in user is a local record in PM.store; nothing talks to a server.
   One subscribe/notify pair drives every place that shows who you are: the
   home sidebar, Settings › Accounts and the Store's Yours page. */
import { flushSync, mount, unmount } from 'svelte';

import type { PMRegistry } from '../legacy/registry';
import { openPopoverMenu, type PopoverMenuHandle } from '../controls/popover-menu';
import Avatar from './Avatar.svelte';
import SignInSheet from './SignInSheet.svelte';

export type AccountProvider = 'apple' | 'google' | 'github' | 'email';

export type CloudUser = {
  name: string;
  /** Publisher namespace: extensions publish as `handle/id`. */
  handle: string;
  email: string;
  provider: AccountProvider;
};

export type SignInMode = 'sign-in' | 'sign-up';

type Listener = (user: CloudUser | null) => void;

const STORE_KEY = 'cloudAccount';

export const PROVIDER_LABEL: Record<AccountProvider, string> = {
  apple: 'Apple',
  google: 'Google',
  github: 'GitHub',
  email: 'email'
};

let registry: PMRegistry | null = null;
let user: CloudUser | null = null;
let sheet: { close(): void } | null = null;
const listeners = new Set<Listener>();

function notify(): void {
  for (const listener of listeners) listener(user);
}

function valid(value: unknown): value is CloudUser {
  const v = value as Partial<CloudUser> | null;
  return !!v && typeof v.name === 'string' && typeof v.handle === 'string' && typeof v.email === 'string';
}

export function currentUser(): CloudUser | null {
  return user;
}

/** Calls back now with the current user, then on every change. */
export function subscribeAccount(listener: Listener): () => void {
  listeners.add(listener);
  listener(user);
  return () => listeners.delete(listener);
}

export function completeSignIn(next: CloudUser): void {
  user = next;
  registry?.store?.set?.(STORE_KEY, next);
  notify();
}

export function signOut(): void {
  user = null;
  registry?.store?.set?.(STORE_KEY, null);
  notify();
}

export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (!words.length) return '?';
  const first = [...words[0]!][0] ?? '';
  const last = words.length > 1 ? [...words.at(-1)!][0] ?? '' : '';
  return (first + last).toUpperCase();
}

/** The sign-in and sign-up sheet. One at a time; asking again focuses it. */
export function openSignIn(mode: SignInMode = 'sign-in'): void {
  const PM = registry as any;
  if (!PM?.modal || sheet) return;
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
      onsignedin: (next: CloudUser) => {
        completeSignIn(next);
        handle.close();
      },
      oncancel: () => handle.close()
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
      { label: 'Your Library', run: () => PM?.StoreUI?.open?.('library') },
      { label: 'Account Settings…', run: () => PM?.SettingsUI?.open?.('accounts') },
      '-',
      { label: 'Sign Out', run: signOut }
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
  const saved = (PM as any).store?.get?.(STORE_KEY, null);
  user = valid(saved) ? saved : null;
  (PM as any).Account = {
    get user() { return user; },
    signIn: openSignIn,
    signOut,
    subscribe: subscribeAccount
  };
  notify();
}
