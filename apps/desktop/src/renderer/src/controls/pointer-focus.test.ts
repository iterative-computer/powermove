// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { installPointerFocus } from './pointer-focus';

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  document.body.replaceChildren();
});

it('releases a pointer-clicked button, including clicks on its icon, before its action', () => {
  dispose = installPointerFocus();
  document.body.innerHTML = '<button><svg><path /></svg></button><input />';
  const button = document.querySelector('button')!;
  const input = document.querySelector('input')!;
  let focusedDuringAction: Element | null = null;
  button.onclick = () => { focusedDuringAction = document.activeElement; input.focus(); };
  button.focus();
  document.querySelector('path')!.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true, composed: true }));
  expect(focusedDuringAction).not.toBe(button);
  expect(document.activeElement).toBe(input);
});

it('preserves keyboard activation and text-entry focus', () => {
  dispose = installPointerFocus();
  document.body.innerHTML = '<button>Action</button><input />';
  const button = document.querySelector('button')!;
  const input = document.querySelector('input')!;
  button.focus();
  button.click();
  expect(document.activeElement).toBe(button);
  input.focus();
  input.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  expect(document.activeElement).toBe(input);
});

it('also releases pointer focus on tabs and removes the listener on disposal', () => {
  dispose = installPointerFocus();
  const tab = document.createElement('div');
  tab.setAttribute('role', 'tab');
  tab.tabIndex = 0;
  document.body.append(tab);
  tab.focus();
  tab.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  expect(document.activeElement).not.toBe(tab);
  dispose();
  tab.focus();
  tab.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  expect(document.activeElement).toBe(tab);
});

it('hands pointer focus to a focusable dialog that owns the control, so its keys stay inside', () => {
  dispose = installPointerFocus();
  document.body.innerHTML = '<div role="dialog" tabindex="-1"><div><button>Video</button></div></div>'
    + '<div role="alertdialog" tabindex="-1"><button>Discard</button></div>';
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
  const alert = document.querySelector<HTMLElement>('[role="alertdialog"]')!;
  const escape = vi.fn();
  dialog.addEventListener('keydown', escape);
  const video = dialog.querySelector('button')!;
  let focusedDuringAction: Element | null = null;
  video.onclick = () => { focusedDuringAction = document.activeElement; };
  video.focus();
  video.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true, composed: true }));
  expect(focusedDuringAction).toBe(dialog);
  expect(document.activeElement).toBe(dialog);
  document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  expect(escape).toHaveBeenCalledOnce();

  const discard = alert.querySelector('button')!;
  discard.focus();
  discard.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true, composed: true }));
  expect(document.activeElement).toBe(alert);

  // Keyboard activation keeps the control itself focused, as before.
  video.focus();
  video.click();
  expect(document.activeElement).toBe(video);
});

it('keeps releasing to <body> inside a dialog that is not focusable or cannot take focus', () => {
  dispose = installPointerFocus();
  // The agent popover is a dialog without tabindex; an enclosing focusable dialog must not claim its controls.
  document.body.innerHTML = '<div role="dialog" tabindex="-1" id="outer"><section role="dialog"><button>View</button></section></div>'
    + '<div role="dialog" tabindex="-1" id="stuck"><button>Stay</button></div>';
  const view = document.querySelector<HTMLButtonElement>('section button')!;
  view.focus();
  view.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true, composed: true }));
  expect(document.activeElement).toBe(document.body);

  const stuck = document.querySelector<HTMLElement>('#stuck')!;
  stuck.focus = vi.fn();
  const stay = stuck.querySelector('button')!;
  stay.focus();
  stay.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true, composed: true }));
  expect(stuck.focus).toHaveBeenCalledWith({ preventScroll: true });
  expect(document.activeElement).not.toBe(stay);
});

it('leaves text fields inside a focusable dialog focused', () => {
  dispose = installPointerFocus();
  document.body.innerHTML = '<div role="dialog" tabindex="-1"><input /></div>';
  const input = document.querySelector('input')!;
  input.focus();
  input.dispatchEvent(new MouseEvent('click', { detail: 1, bubbles: true }));
  expect(document.activeElement).toBe(input);
});
