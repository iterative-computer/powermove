// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import AgentPanel from '../AgentPanel.svelte';
import { install as installSpatial } from '../../legacy/assistant/spatial';
import { registerAgentPanel } from '../register-agent';
import {
  agentState,
  resetAgentState,
  setAgentComposerDraft,
  setAgentSnapshot,
  type AgentSnapshot
} from './agent-state.svelte';

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;
let PM: Record<string, any>;

function h(selector: string, ...children: any[]): HTMLElement {
  const [tag, ...classes] = selector.split('.');
  const element = document.createElement(tag || 'div');
  if (classes.length) element.className = classes.join(' ');
  for (const child of children.flat(Infinity)) {
    if (child == null) continue;
    element.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return element;
}

function modal({ title, body, actions = [], onClose }: Record<string, any>): Record<string, any> {
  const scrim = document.querySelector<HTMLElement>('#scrim')!;
  scrim.classList.add('on');
  const element = h('div.modal');
  if (title) element.append(h('h3', title));
  const modalBody = h('div.mb', body);
  element.append(modalBody);
  const close = (): void => {
    element.remove();
    scrim.classList.remove('on');
    onClose?.();
  };
  if (actions.length) {
    const footer = h('div.mf');
    for (const action of actions) {
      const button = h(`button.btn${action.pri ? '.pri' : ''}`, action.label) as HTMLButtonElement;
      button.type = 'button';
      button.onclick = () => { if (action.run?.() === false) return; close(); };
      footer.append(button);
    }
    element.append(footer);
  }
  document.body.append(element);
  return { el: element, body: modalBody, close };
}

function snapshot(overrides: Partial<AgentSnapshot> = {}): AgentSnapshot {
  return {
    legacyPhase: 'idle',
    requestToken: 0,
    conversation: [],
    activity: '',
    plan: null,
    run: null,
    panelRun: null,
    attachments: [],
    steps: [],
    stepsExpanded: false,
    scope: 'workspace',
    autoApplyPanels: true,
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    accessMode: 'editor',
    composerDraft: '',
    pendingEntering: false,
    models: [
      { id: 'gpt-5.6-sol', label: '5.6 Sol' },
      { id: 'gpt-5.6-terra', label: '5.6 Terra' }
    ],
    reasoningEfforts: ['low', 'high'],
    accessModes: [
      { id: 'editor', label: 'Editor', detail: 'Powermove source only' },
      { id: 'project', label: 'Project + web', detail: 'Project tools' },
      { id: 'computer', label: 'Computer', detail: 'Full Mac access' }
    ],
    ...overrides
  };
}

function renderPanel(initial = snapshot()): void {
  setAgentSnapshot(initial);
  instance = mount(AgentPanel, { target, props: { panelId: 'agent', spec: {} } });
  flushSync();
}

beforeEach(() => {
  const scrim = document.createElement('div');
  scrim.id = 'scrim';
  document.body.append(scrim);
  target = document.createElement('div');
  document.body.append(target);
  const store = { set: vi.fn() };
  PM = {
    ICONS: { sparkle: '', panel: '', chev: '', plus: '', x: '', return: '', frame: '', cam: '', clock: '', link: '', project: '' },
    PANELS: { viewer: { title: 'Composition' }, timeline: { title: 'Timeline' } },
    WS: { current: { layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] }, hiddenPanels: [] } },
    AgentHarness: { describeCommand: vi.fn(() => 'Set layer opacity') },
    SpatialAssistant: {
      math: {
        textareaLayout: (scrollHeight: number, minHeight: number, maxHeight: number) => {
          const min = Number.isFinite(minHeight) ? Math.max(0, minHeight) : 0;
          const max = Number.isFinite(maxHeight) ? Math.max(min, maxHeight) : Number.POSITIVE_INFINITY;
          const height = Math.min(max, Math.max(min, Math.ceil(scrollHeight)));
          return { height, overflowY: scrollHeight > height + 1 ? 'auto' : 'hidden' };
        }
      }
    },
    assetKind: vi.fn(() => false),
    h,
    modal: vi.fn(modal),
    store,
    AgentUI: {
      submit: vi.fn(), stop: vi.fn(), setDraft: vi.fn((value: string) => setAgentComposerDraft(value)), setStepsExpanded: vi.fn(), setModel: vi.fn(),
      setAccess: vi.fn(), confirmComputerAccess: vi.fn(),
      setScope: vi.fn(), toggleAutoApplyPanels: vi.fn(), dismissPlan: vi.fn(), applyPlan: vi.fn(),
      addAttachments: vi.fn(), removeAttachment: vi.fn(), importArtifact: vi.fn(), revealArtifact: vi.fn(),
      undoPanelRun: vi.fn(), keepPanelRun: vi.fn(), undoSceneRun: vi.fn(), keepSceneRun: vi.fn()
    }
  };
  window.PM = PM as any;
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
  document.querySelector('#scrim')?.remove();
  document.querySelectorAll('.modal').forEach((element) => element.remove());
  resetAgentState();
  delete window.PM;
  vi.restoreAllMocks();
});

describe('AgentPanel', () => {
  it('renders idle, prompt, running, preview, and result blocks from state setters', () => {
    renderPanel();
    expect(target.querySelector('[data-agent-phase="idle"]')).toBeTruthy();

    flushSync(() => setAgentSnapshot(snapshot({ legacyPhase: 'conversation', conversation: [{ role: 'assistant', text: 'Ready.' }] })));
    expect(target.querySelector('[data-agent-panel]')?.getAttribute('data-agent-phase')).toBe('prompt');
    expect(target.textContent).toContain('Ready.');

    flushSync(() => setAgentSnapshot(snapshot({
      legacyPhase: 'working', requestToken: 1, activity: 'Inspecting the composition…',
      steps: [{ id: '1-0', title: 'Inspect composition', status: 'active' }]
    })));
    expect(target.querySelector('[data-agent-panel]')?.getAttribute('data-agent-phase')).toBe('running');
    expect(target.querySelector('[aria-label="Stop current run"]')).toBeTruthy();

    flushSync(() => setAgentSnapshot(snapshot({
      legacyPhase: 'conversation',
      plan: { kind: 'panels', panelEdit: { actions: [{ type: 'hide', panelId: 'timeline' }] } },
      steps: [{ id: '1-0', title: 'Arrange panels', status: 'complete' }]
    })));
    expect(target.querySelector('[data-agent-phase="preview"]')?.textContent).toContain('Hide Timeline');

    flushSync(() => setAgentSnapshot(snapshot({
      legacyPhase: 'result',
      panelRun: { summary: '1 panel change applied', actions: [{ type: 'hide', panelId: 'timeline' }] }
    })));
    expect(target.querySelector('[data-agent-phase="result"]')?.textContent).toContain('1 panel change applied');
  });

  it('clears the focused composer after Enter and does not resend an empty draft', () => {
    renderPanel();
    const textarea = target.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message Powermove agent"]')!;
    expect(target.querySelector(`label[for="${textarea.id}"]`)?.textContent).toBe('Message Powermove agent');

    textarea.focus();
    textarea.value = 'Make the title bounce';
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));
    PM.AgentUI.submit.mockImplementationOnce(() => {
      setAgentSnapshot(snapshot({
        legacyPhase: 'working',
        requestToken: 1,
        composerDraft: '',
        conversation: [{ role: 'user', text: 'Make the title bounce', entering: true }],
        activity: 'Looking at the composition and workspace…'
      }));
    });
    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    flushSync();

    expect(PM.AgentUI.submit).toHaveBeenCalledExactlyOnceWith('Make the title bounce');
    expect(textarea.value).toBe('');
    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(PM.AgentUI.submit).toHaveBeenCalledOnce();
  });

  it('separates the model and effort triggers and keeps no scope or authority pickers', () => {
    renderPanel();
    expect(target.querySelector('select[aria-label="Agent authority"]')).toBeNull();
    expect(target.querySelector('select[aria-label="Agent scope"]')).toBeNull();

    const model = target.querySelector<HTMLSelectElement>('select[aria-label="Model"]')!;
    model.value = 'gpt-5.6-terra';
    flushSync(() => model.dispatchEvent(new Event('change', { bubbles: true })));
    expect(PM.AgentUI.setModel).toHaveBeenCalledWith('gpt-5.6-terra', 'high');

    const effort = target.querySelector<HTMLSelectElement>('select[aria-label="Reasoning effort"]')!;
    effort.value = 'low';
    flushSync(() => effort.dispatchEvent(new Event('change', { bubbles: true })));
    expect(PM.AgentUI.setModel).toHaveBeenCalledWith('gpt-5.6-sol', 'low');
  });

  it('accumulates progress in state while rendering only the newest word-staggered line', () => {
    renderPanel(snapshot({ legacyPhase: 'working', requestToken: 4, activity: 'Inspecting source…' }));
    flushSync(() => setAgentSnapshot(snapshot({ legacyPhase: 'working', requestToken: 4, activity: 'Designing the edit…' })));

    const log = target.querySelector('[role="log"]')!;
    expect(log.getAttribute('aria-live')).toBe('polite');
    expect(target.querySelector('[role="status"]')).toBeNull();
    expect(agentState.progressLines).toEqual([
      'Inspecting source…',
      'Designing the edit…'
    ]);
    expect([...log.querySelectorAll('.agent-thinking-word')].map((word) => word.textContent).join('')).toBe('Designing the edit…');
    expect(log.querySelector('.agent-thinking-word')?.getAttribute('style')).toMatch(/--word-index:\s*0/);
  });

  it('self-heals a stale panel scope to the workspace', async () => {
    renderPanel(snapshot({ scope: 'panel:missing' }));
    await Promise.resolve();
    flushSync();
    expect(PM.AgentUI.setScope).toHaveBeenCalledWith('workspace');
  });

  it('wires the running Stop button to the legacy stop action', () => {
    renderPanel(snapshot({ legacyPhase: 'working', requestToken: 2, activity: 'Working…' }));
    target.querySelector<HTMLButtonElement>('[aria-label="Stop current run"]')!.click();
    expect(PM.AgentUI.stop).toHaveBeenCalledOnce();
  });

  it('calls the unchanged keep and undo actions from result controls', () => {
    renderPanel(snapshot({
      legacyPhase: 'result',
      panelRun: { summary: 'Panel changes ready', actions: [{ type: 'move', panelId: 'timeline', dockId: 'right' }] }
    }));
    const buttons = [...target.querySelectorAll<HTMLButtonElement>('[data-agent-phase="result"] button')];
    buttons.find((button) => button.textContent === 'Undo change')?.click();
    buttons.find((button) => button.textContent === 'Keep change')?.click();

    expect(PM.AgentUI.undoPanelRun).toHaveBeenCalledOnce();
    expect(PM.AgentUI.keepPanelRun).toHaveBeenCalledOnce();
  });
});

describe('agent bridge', () => {
  it('maps bridge snapshots to view phases, including a panel run result', () => {
    const legacyState = snapshot({ legacyPhase: 'conversation' });
    const registry: Record<string, any> = { registerPanel: vi.fn() };
    const action = vi.fn();
    registerAgentPanel(registry, {
      snapshot: () => legacyState,
      submit: action,
      stop: action,
      setDraft: action,
      setStepsExpanded: action,
      setModel: action,
      setAccess: action,
      confirmComputerAccess: action,
      setScope: action,
      toggleAutoApplyPanels: action,
      dismissPlan: action,
      applyPlan: action,
      addAttachments: async () => {},
      removeAttachment: action,
      importArtifact: async () => {},
      revealArtifact: action,
      undoPanelRun: action,
      keepPanelRun: action,
      undoSceneRun: action,
      keepSceneRun: action
    });

    expect(agentState.phase).toBe('prompt');
    legacyState.panelRun = { summary: 'Applied panel change', actions: [] };
    registry.AgentUI.update();
    expect(agentState.phase).toBe('result');
  });

  it('persists spatial raw access choices and refuses unconfirmed computer escalation', () => {
    const store = {
      get: vi.fn((_key: string, fallback: unknown) => fallback),
      set: vi.fn()
    };
    const registry: Record<string, any> = { store, registerPanel: vi.fn() };
    installSpatial(registry);

    registry.AgentUI.setAccess('project');
    expect(store.set).toHaveBeenCalledWith('agentAccessMode', 'project');
    expect(agentState.accessMode).toBe('project');

    registry.AgentUI.setAccess('computer');
    expect(agentState.accessMode).toBe('project');
    expect(store.set).not.toHaveBeenCalledWith('agentAccessMode', 'computer');

    registry.AgentUI.confirmComputerAccess();
    expect(agentState.accessMode).toBe('computer');
    expect(store.set).not.toHaveBeenCalledWith('agentAccessMode', 'computer');
  });
});
