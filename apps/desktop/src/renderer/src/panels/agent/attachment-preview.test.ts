// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ModalController } from '../../overlays/modal';
import { installAgentShell } from '../agent-shell';
import AttachmentChips from './AttachmentChips.svelte';
import Turn from './Turn.svelte';
import { mountPromptAttachments } from './attachments';
import { resetAgentState } from './agent-state.svelte';

let component: ReturnType<typeof mount> | undefined;
let shell: ReturnType<typeof installAgentShell> | undefined;
let modal: ModalController | undefined;
let attachments: ReturnType<typeof mountPromptAttachments> | undefined;

beforeEach(() => {
  resetAgentState();
  document.body.innerHTML = '<div id="app"><button id="agent-launcher">Agent</button></div><div id="scrim"></div>';
});

afterEach(async () => {
  modal?.closeAll();
  attachments?.dispose();
  shell?.destroy();
  if (component) await unmount(component);
  component = undefined; shell = undefined; modal = undefined; attachments = undefined;
  document.body.replaceChildren();
});

it.each([
  { presentation: 'chip', activation: 'pointer' },
  { presentation: 'inline', activation: 'pointer' },
  { presentation: 'draft', activation: 'pointer' },
  { presentation: 'chip', activation: 'unfocused-pointer' },
  { presentation: 'inline', activation: 'unfocused-pointer' },
  { presentation: 'draft', activation: 'unfocused-pointer' },
  { presentation: 'chip', activation: 'keyboard' },
  { presentation: 'inline', activation: 'keyboard' },
  { presentation: 'draft', activation: 'keyboard' },
])('restores appropriate focus after $activation activation of a $presentation preview and Escape', async ({ presentation, activation }) => {
  const panel = document.createElement('div');
  panel.innerHTML = '<div class="attachments"></div><div class="spatial-input-row"><textarea class="agent-inline-prompt"></textarea></div>';
  document.body.append(panel);
  const prompt = panel.querySelector<HTMLTextAreaElement>('textarea')!;
  const target = panel.querySelector<HTMLElement>('.attachments')!;
  const PM = { panelInst: { agent: { el: panel } }, closeMenus: vi.fn(), icon: () => document.createElement('span'), modal: (options: any) => modal!.open(options) };
  modal = new ModalController(PM as any);
  shell = installAgentShell(PM);
  const item = { id: 'reference', name: 'reference.png', type: 'image/png', size: 1, dataUrl: 'data:image/png;base64,eA==' };
  if (presentation === 'chip') component = mount(AttachmentChips, { target, props: { PM, items: [item] } });
  else if (presentation === 'inline') component = mount(Turn, { target, props: { PM, message: { role: 'user', text: 'Inspect [Attachment: reference.png]', attachments: [item] } } });
  else attachments = mountPromptAttachments(PM, panel, () => [item]);
  flushSync();
  shell.open();
  await Promise.resolve();
  expect(document.activeElement).toBe(prompt);
  const trigger = panel.querySelector<HTMLButtonElement>('button[aria-label="View reference.png"]')!;
  expect(trigger).not.toBeNull();
  // A pointer activation need not transfer focus (including a toolbar press
  // that preserves the composer). Keyboard activation starts on the button.
  if (activation === 'keyboard') trigger.focus();
  if (activation === 'unfocused-pointer') { prompt.blur(); expect(document.activeElement).toBe(document.body); }
  trigger.click();
  flushSync();
  await Promise.resolve();
  const dialog = document.querySelector<HTMLElement>('.modal[role="dialog"]')!;
  expect(dialog.querySelector('img')?.getAttribute('alt')).toBe('reference.png');
  expect(dialog.contains(document.activeElement)).toBe(true);
  dialog.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  flushSync();
  expect(document.querySelector('.modal')).toBeNull();
  expect(shell.isOpen()).toBe(true);
  expect(trigger.isConnected).toBe(true);
  expect(document.activeElement).toBe(activation === 'keyboard' ? trigger
    : activation === 'pointer' ? prompt : document.getElementById('app'));
  if (activation !== 'keyboard') expect(document.activeElement).not.toBe(trigger);
});
