// @vitest-environment happy-dom
import { flushSync } from 'svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ModalController } from './modal';
import { installPointerFocus } from '../controls/pointer-focus';

let controller: ModalController;
let release: () => void;
beforeEach(() => {
  document.body.innerHTML = '<div id="app"><button id="opener">Open</button></div><div id="scrim"></div>';
  controller = new ModalController({ closeMenus: vi.fn() } as any);
  release = installPointerFocus();
});
afterEach(() => { controller.closeAll(); release(); document.body.replaceChildren(); });

async function fixture() {
  const body = document.createElement('div');
  body.innerHTML = '<button type="button">Video</button><input aria-label="Quality" />';
  const opener = document.querySelector<HTMLButtonElement>('#opener')!;
  opener.focus();
  const modal = controller.open({ title: 'Export', body });
  await Promise.resolve(); flushSync();
  const button = body.querySelector('button')!;
  button.focus();
  button.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  return { modal, button, input: body.querySelector('input')!, opener };
}

it('keeps Escape routed to the modal after a pointer-clicked action releases button focus', async () => {
  const { modal, button, opener } = await fixture();
  expect(document.activeElement).not.toBe(button);
  const pageShortcut = vi.fn();
  document.addEventListener('keydown', pageShortcut);
  try {
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(modal.el.isConnected).toBe(false);
    expect(document.activeElement).toBe(opener);
    expect(pageShortcut).not.toHaveBeenCalled();
  } finally { document.removeEventListener('keydown', pageShortcut); }
});

it('keeps Tab inside the modal after clicking a button without activating it with Space', async () => {
  const { modal, button } = await fixture();
  expect(document.activeElement).toBe(modal.el);
  document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
  expect(document.activeElement).toBe(button);
});

it('allows an action to focus a field after the pointer focus handoff', async () => {
  const { button, input } = await fixture();
  button.onclick = () => input.focus();
  button.focus();
  button.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  expect(document.activeElement).toBe(input);
});

it('preserves a focused field when a pointer action does not take focus', async () => {
  const { button, input } = await fixture();
  input.focus();
  button.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  expect(document.activeElement).toBe(input);
});

it('opens a sheet of only buttons focused on itself, not on its first button', async () => {
  const body = document.createElement('div');
  body.innerHTML = '<button type="button">Video</button><button type="button">Images</button>';
  const modal = controller.open({ title: 'Export', body, actions: [{ label: 'Cancel' }, { label: 'Export', pri: true }] });
  await Promise.resolve(); flushSync();
  expect(document.activeElement).toBe(modal.el);
});

it('opens on the first text field even when buttons come before it', async () => {
  const body = document.createElement('div');
  body.innerHTML = '<button type="button">Video</button><input aria-label="Name" />';
  controller.open({ title: 'Export', body });
  await Promise.resolve(); flushSync();
  expect(document.activeElement).toBe(body.querySelector('input'));
});

it('confirms with Return from a text field, committing the field first', async () => {
  const body = document.createElement('div');
  body.innerHTML = '<input aria-label="Name" /><textarea aria-label="Notes"></textarea>';
  const input = body.querySelector('input')!;
  const committed = vi.fn();
  input.addEventListener('change', committed);
  const run = vi.fn();
  const modal = controller.open({ title: 'Rename', body, actions: [{ label: 'Cancel' }, { label: 'Rename', pri: true, run }] });
  await Promise.resolve(); flushSync();

  body.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, cancelable: true }));
  const claimed = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  claimed.preventDefault();
  input.dispatchEvent(claimed);
  expect(run).not.toHaveBeenCalled();

  input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  expect(committed).toHaveBeenCalledTimes(1);
  expect(run).toHaveBeenCalledTimes(1);
  expect(modal.el.isConnected).toBe(false);
});

it('leaves Return alone in a sheet without a primary action', async () => {
  const body = document.createElement('div');
  body.innerHTML = '<input aria-label="Name" />';
  const modal = controller.open({ title: 'Details', body, actions: [{ label: 'Close' }] });
  await Promise.resolve(); flushSync();
  body.querySelector('input')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  expect(modal.el.isConnected).toBe(true);
});
