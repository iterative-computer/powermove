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
