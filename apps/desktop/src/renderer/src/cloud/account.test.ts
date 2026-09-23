// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import type { MeDto } from '@powermove/registry/wire';

import type { CloudBridge } from '../../../shared/cloud-ipc';
import SignInSheet from './SignInSheet.svelte';
import { currentUser, installCloudAccount, userFromMe } from './account';

const NO_HANDLE: MeDto = {
  user: { id: '3f1c9b1e-8f55-4d8f-9d0a-6f1d1c1b2a3e', name: 'Jude Kim', email: 'jude@example.test', image: null },
  publisher: null,
  settings: { rememberInstalls: true }
};
const WITH_HANDLE: MeDto = {
  ...NO_HANDLE,
  publisher: { id: '7d2a0b6e-1c3f-4a5b-8c9d-0e1f2a3b4c5d', handle: 'jude', tombstoned: false }
};

beforeAll(() => {
  // Reduced motion: happy-dom has no Web Animations, so step transitions run instantly.
  Object.defineProperty(window, 'matchMedia', { configurable: true, value: () => ({ matches: true, addEventListener() {}, removeEventListener() {} }) });
});

function bridgeWith(over: Partial<CloudBridge> = {}): CloudBridge {
  return {
    account: vi.fn(async () => ({ me: null })),
    signInSocial: vi.fn(async () => ({ ok: true as const, value: null })),
    emailSend: vi.fn(async () => ({ ok: true as const, value: null })),
    emailVerify: vi.fn(async () => ({ ok: true as const, value: NO_HANDLE })),
    claimHandle: vi.fn(async () => ({ ok: true as const, value: WITH_HANDLE })),
    setRememberInstalls: vi.fn(async () => ({ ok: true as const, value: NO_HANDLE })),
    signOut: vi.fn(async () => ({ ok: true as const, value: null })),
    deleteAccount: vi.fn(async () => ({ ok: true as const, value: { deleted: false } })),
    registryUrl: vi.fn(async () => ({ origin: 'https://cloud.trypowermove.com', isDefault: true })),
    setRegistryUrl: vi.fn(async () => ({ ok: true as const, value: { origin: 'https://cloud.trypowermove.com', isDefault: true, changed: false } })),
    onAccountChanged: vi.fn(() => () => {}),
    onSignInFailed: vi.fn(() => () => {}),
    ...over
  };
}

const PM = { ICONS: { google: '', github: '', chev: '' }, toast: vi.fn() };

let component: ReturnType<typeof mount> | null = null;
afterEach(() => {
  if (component) void unmount(component);
  component = null;
  document.body.replaceChildren();
});

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) { await Promise.resolve(); await tick(); }
  flushSync();
}

function heading(root: ParentNode): string {
  return root.querySelector('h2')?.textContent ?? '';
}

describe('account', () => {
  it('derives the user from MeDto', () => {
    expect(userFromMe(WITH_HANDLE)).toEqual({ name: 'Jude Kim', handle: 'jude', email: 'jude@example.test', image: null });
    expect(userFromMe({ ...NO_HANDLE, user: { ...NO_HANDLE.user, name: null } })).toMatchObject({ name: 'jude', handle: null });
  });

  it('the sheet opens on the handle step when the account has no publisher', async () => {
    const target = document.createElement('div');
    document.body.append(target);
    component = mount(SignInSheet, { target, props: { PM, mode: 'sign-in', bridge: bridgeWith(), me: NO_HANDLE, onclose: vi.fn() } });
    await settle();
    expect(heading(target)).toBe('Choose your handle');
    expect(target.querySelector<HTMLInputElement>('input[aria-label="Handle"]')!.value).toBe('jude-kim');
  });

  it('email sign-in ends on the handle step when the account has no publisher, and claims it', async () => {
    const bridge = bridgeWith();
    const onclose = vi.fn();
    const target = document.createElement('div');
    document.body.append(target);
    component = mount(SignInSheet, { target, props: { PM, mode: 'sign-in', bridge, me: null, onclose } });
    await settle();
    expect(heading(target)).toBe('Sign in to Powermove');

    const email = target.querySelector<HTMLInputElement>('input[type="email"]')!;
    email.value = 'jude@example.test';
    email.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    target.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(bridge.emailSend).toHaveBeenCalledWith({ email: 'jude@example.test' });
    expect(heading(target)).toBe('Check your email');

    const code = target.querySelector<HTMLInputElement>('input[autocomplete="one-time-code"]')!;
    code.value = '123456';
    code.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(bridge.emailVerify).toHaveBeenCalledWith({ email: 'jude@example.test', otp: '123456' });
    expect(heading(target)).toBe('Choose your handle');

    // A taken handle is said in plain words next to the field.
    vi.mocked(bridge.claimHandle).mockResolvedValueOnce({ ok: false, error: { error: 'handle_taken' } });
    target.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(target.querySelector('.acct-handle-status')!.textContent).toBe('@jude-kim is taken. Try another.');
    expect(onclose).not.toHaveBeenCalled();

    const field = target.querySelector<HTMLInputElement>('input[aria-label="Handle"]')!;
    field.value = 'jude';
    field.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    target.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(bridge.claimHandle).toHaveBeenLastCalledWith({ handle: 'jude' });
    expect(onclose).toHaveBeenCalled();
  });

  it('shows reserved and invalid handles in plain words', async () => {
    const bridge = bridgeWith({
      claimHandle: vi.fn()
        .mockResolvedValueOnce({ ok: false, error: { error: 'handle_reserved' } })
        .mockResolvedValueOnce({ ok: false, error: { error: 'handle_invalid' } })
    });
    const target = document.createElement('div');
    document.body.append(target);
    component = mount(SignInSheet, { target, props: { PM, mode: 'sign-in', bridge, me: NO_HANDLE, onclose: vi.fn() } });
    await settle();
    const submit = () => target.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    submit();
    await settle();
    expect(target.querySelector('.acct-handle-status')!.textContent).toBe('@jude-kim is reserved. Try another.');
    expect(target.querySelector('.acct-handle-status')!.getAttribute('data-state')).toBe('error');
    submit();
    await settle();
    expect(target.querySelector('.acct-handle-status')!.textContent).toBe('Use 2 to 39 characters, starting with a letter or number.');
  });

  it('a browser sign-in waits for the account and closes when it arrives with a handle', async () => {
    let changed: ((me: MeDto | null) => void) | null = null;
    const bridge = bridgeWith({ onAccountChanged: vi.fn((cb) => { changed = cb; return () => {}; }) });
    const modalClose = vi.fn();
    const pm = {
      ...PM,
      modal: vi.fn(({ body, onClose }: { body: HTMLElement; onClose: () => void }) => {
        document.body.append(body);
        return { el: body, close: () => { modalClose(); onClose(); } };
      })
    };
    (window as unknown as { powermove: { cloud: CloudBridge } }).powermove = { cloud: bridge };
    installCloudAccount(pm as never);
    await settle();
    expect(currentUser()).toBeNull();

    (await import('./account')).openSignIn();
    await settle();
    (document.querySelector<HTMLButtonElement>('button[data-provider="github"]'))!.click();
    await settle();
    expect(bridge.signInSocial).toHaveBeenCalledWith({ provider: 'github' });
    expect(heading(document.body)).toBe('Finish signing in in your browser');

    changed!(WITH_HANDLE);
    await settle();
    expect(currentUser()).toMatchObject({ handle: 'jude' });
    expect(modalClose).toHaveBeenCalled();
  });

  it('opens the handle step on launch when the saved account has no publisher', async () => {
    const bridge = bridgeWith({ account: vi.fn(async () => ({ me: NO_HANDLE, provider: 'google' as const })) });
    const pm = {
      ...PM,
      modal: vi.fn(({ body, onClose }: { body: HTMLElement; onClose: () => void }) => {
        document.body.append(body);
        return { el: body, close: onClose };
      })
    };
    (window as unknown as { powermove: { cloud: CloudBridge } }).powermove = { cloud: bridge };
    installCloudAccount(pm as never);
    await settle();
    expect(pm.modal).toHaveBeenCalledTimes(1);
    expect(heading(document.body)).toBe('Choose your handle');
  });
});
