// @vitest-environment happy-dom
import { afterEach, expect, it } from 'vitest';
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
