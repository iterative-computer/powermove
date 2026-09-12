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
    trace: [],
    plan: null,
    run: null,
    panelRun: null,
    attachments: [],
    steps: [],
    stepsExpanded: false,
    scope: 'workspace',
    autoApplyPanels: true,
    provider: 'chatgpt',
    model: 'gpt-5.6-sol',
    reasoningEffort: 'high',
    accessMode: 'editor',
    composerDraft: '',
    pendingEntering: false,
    models: [
      { id: 'gpt-5.6-sol', label: '5.6 Sol' },
      { id: 'gpt-5.6-terra', label: '5.6 Terra' }
    ],
    providers: [
      { id: 'chatgpt', label: 'ChatGPT' },
      { id: 'claude', label: 'Claude' }
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
    uid: (prefix: string) => `${prefix}${crypto.randomUUID()}`,
    ICONS: { sparkle: '', panel: '', chev: '', plus: '', x: '', return: '', frame: '', cam: '', clock: '', link: '', project: '' },
    PANELS: { viewer: { title: 'Composition' }, timeline: { title: 'Timeline' } },
    WS: { current: { layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] }, hiddenPanels: [] } },
    AgentHarness: { describeCommand: vi.fn(() => 'Set layer opacity') },
    SpatialAssistant: {
      requestFix: vi.fn(),
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
      submit: vi.fn(), stop: vi.fn(), setDraft: vi.fn((value: string) => setAgentComposerDraft(value)), setStepsExpanded: vi.fn(), setModel: vi.fn(), setProvider: vi.fn(),
      setAccess: vi.fn(), confirmComputerAccess: vi.fn(),
      setScope: vi.fn(), toggleAutoApplyPanels: vi.fn(), dismissPlan: vi.fn(), applyPlan: vi.fn(),
      addAttachments: vi.fn(), removeAttachment: vi.fn(), importArtifact: vi.fn(), revealArtifact: vi.fn(),
      undoPanelRun: vi.fn(), keepPanelRun: vi.fn(), undoSceneRun: vi.fn(), keepSceneRun: vi.fn()
    }
  };
  window.PM = PM as any;
  Object.defineProperty(window, 'powermove', {
    configurable: true,
    value: {
      chatgpt: {
        status: vi.fn(async () => ({ state: 'connected', email: null, planType: 'plus', detail: null })),
        connect: vi.fn(),
        disconnect: vi.fn(),
        onChanged: vi.fn(() => () => undefined)
      }
    }
  });
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
  document.querySelector('#scrim')?.remove();
  document.querySelectorAll('.modal').forEach((element) => element.remove());
  resetAgentState();
  delete window.PM;
  Reflect.deleteProperty(window, 'powermove');
  vi.restoreAllMocks();
});

describe('AgentPanel', () => {
  it('shows ChatGPT connection inside the agent and restores the composer after sign-in', async () => {
    let changed: ((status: any) => void) | null = null;
    const connect = vi.fn(async () => ({
      state: 'connecting', email: null, planType: null, detail: 'Finish signing in in your browser.'
    }));
    Object.defineProperty(window, 'powermove', {
      configurable: true,
      value: {
        chatgpt: {
          status: vi.fn(async () => ({ state: 'disconnected', email: null, planType: null, detail: null })),
          connect,
          disconnect: vi.fn(),
          onChanged: vi.fn((listener: (status: any) => void) => {
            changed = listener;
            return () => { changed = null; };
          })
        }
      }
    });

    renderPanel();
    await vi.waitFor(() => {
      flushSync();
      expect(target.querySelector<HTMLButtonElement>('.agent-connect-button')?.textContent).toBe('Connect ChatGPT');
    });
    expect(target.querySelector('[aria-label="Message composer"]')).toBeTruthy();

    target.querySelector<HTMLButtonElement>('.agent-connect-button')!.click();
    await vi.waitFor(() => expect(connect).toHaveBeenCalledOnce());
    await vi.waitFor(() => {
      flushSync();
      expect(target.textContent).toContain('Finish signing in in your browser.');
    });
    expect(target.querySelector<HTMLButtonElement>('.agent-connect-button')?.textContent).toBe('Waiting…');

    flushSync(() => changed?.({ state: 'connected', email: null, planType: 'plus', detail: null }));
    expect(target.querySelector('[aria-label="Message composer"]')).toBeTruthy();
    expect(target.querySelector('.agent-connect-gate')).toBeNull();
  });

  it('keeps existing threads visible while the provider connection is unavailable', async () => {
    Object.defineProperty(window, 'powermove', {
      configurable: true,
      value: {
        chatgpt: {
          status: vi.fn(async () => ({ state: 'unavailable', email: null, planType: null, detail: 'Offline' })),
          connect: vi.fn(), disconnect: vi.fn(), onChanged: vi.fn(() => () => undefined)
        }
      }
    });
    renderPanel(snapshot({
      threadId: 'saved-thread',
      threads: [{ id: 'saved-thread', title: 'Saved conversation' }]
    }));

    await vi.waitFor(() => expect(target.querySelector('.agent-connect-gate')).toBeTruthy());
    const picker = target.querySelector<HTMLButtonElement>('[aria-label="Switch thread"]');
    expect(picker).toBeTruthy();
    expect(picker?.textContent).toContain('Saved conversation');
  });

  it('offers accessible new-thread and switching controls and disables them during a run', () => {
    PM.AgentUI.newThread = vi.fn(); PM.AgentUI.switchThread = vi.fn(); PM.AgentUI.deleteThread = vi.fn();
    const now = Date.now();
    const threads = [
      { id: 'first', title: 'Animate the title', updatedAt: now },
      { id: 'second', title: 'New thread', updatedAt: now - 5 * 24 * 3600_000 }
    ];
    renderPanel(snapshot({ threadId: 'first', threads }));
    const threadBar = target.querySelector<HTMLElement>('[role="group"][aria-label="Agent threads"]')!;
    const picker = target.querySelector<HTMLButtonElement>('[aria-label="Switch thread"]')!;
    const newThread = target.querySelector<HTMLButtonElement>('[aria-label="New thread"]')!;
    expect(threadBar).toBeTruthy();
    expect(threadBar.contains(picker)).toBe(true);
    expect(threadBar.contains(newThread)).toBe(true);
    expect(picker.textContent).toContain('Animate the title');

    flushSync(() => picker.click());
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.thread-row'));
    expect(rows).toHaveLength(2);
    expect(rows[0]!.getAttribute('aria-selected')).toBe('true');
    expect(rows[0]!.textContent).toContain('Current');
    expect(rows[1]!.textContent).toContain('Opened 5 days ago');

    flushSync(() => rows[0]!.querySelector<HTMLButtonElement>('[aria-label="Delete thread: Animate the title"]')!.click());
    expect(PM.AgentUI.deleteThread).toHaveBeenCalledWith('first');
    expect(PM.AgentUI.switchThread).not.toHaveBeenCalled();

    flushSync(() => rows[1]!.click());
    expect(PM.AgentUI.switchThread).toHaveBeenCalledWith('second');
    newThread.click();
    expect(PM.AgentUI.newThread).toHaveBeenCalledOnce();
    flushSync(() => setAgentSnapshot(snapshot({threadId:'first',threads,threadSwitchBlocked:true})));
    expect(picker.disabled).toBe(true);
    expect(newThread.disabled).toBe(true);
    expect(picker.title).toContain('Finish or stop');
  });

  it('filters threads by the picker search field', () => {
    PM.AgentUI.switchThread = vi.fn();
    const threads = [{ id: 'first', title: 'Animate the title' }, { id: 'second', title: 'Colour grade' }];
    renderPanel(snapshot({ threadId: 'first', threads }));
    flushSync(() => target.querySelector<HTMLButtonElement>('[aria-label="Switch thread"]')!.click());
    const search = document.querySelector<HTMLInputElement>('[aria-label="Search threads"]')!;
    search.value = 'grade';
    flushSync(() => search.dispatchEvent(new Event('input', { bubbles: true })));
    const rows = Array.from(document.querySelectorAll<HTMLElement>('.thread-row'));
    expect(rows).toHaveLength(1);
    expect(rows[0]!.textContent).toContain('Colour grade');
    flushSync(() => rows[0]!.click());
    expect(PM.AgentUI.switchThread).toHaveBeenCalledWith('second');
  });
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

  it('renders steering as a compact continuation of the active user request', () => {
    renderPanel(snapshot({
      legacyPhase: 'working',
      requestToken: 2,
      conversation: [
        { role: 'user', text: 'Make a progressive blur effect' },
        { role: 'user', text: 'continue', steering: true, entering: true }
      ]
    }));

    const userTurns = target.querySelectorAll<HTMLElement>('.agent-msg.user');
    expect(userTurns).toHaveLength(2);
    expect(userTurns[0]?.classList.contains('is-steering')).toBe(false);
    expect(userTurns[1]?.classList.contains('is-steering')).toBe(true);
    expect(userTurns[1]?.querySelector('.agent-bubble')?.textContent).toBe('continue');
  });

  it('separates the model and effort triggers and keeps no scope or authority pickers', () => {
    renderPanel();
    expect(target.querySelector('select[aria-label="Agent authority"]')).toBeNull();
    expect(target.querySelector('select[aria-label="Agent scope"]')).toBeNull();

    const provider = target.querySelector<HTMLSelectElement>('select[aria-label="Provider"]')!;
    provider.value = 'claude';
    flushSync(() => provider.dispatchEvent(new Event('change', { bubbles: true })));
    expect(PM.AgentUI.setProvider).toHaveBeenCalledWith('claude');

    const model = target.querySelector<HTMLSelectElement>('select[aria-label="Model"]')!;
    model.value = 'gpt-5.6-terra';
    flushSync(() => model.dispatchEvent(new Event('change', { bubbles: true })));
    expect(PM.AgentUI.setModel).toHaveBeenCalledWith('gpt-5.6-terra', 'high');

    const effort = target.querySelector<HTMLSelectElement>('select[aria-label="Reasoning effort"]')!;
    effort.value = 'low';
    flushSync(() => effort.dispatchEvent(new Event('change', { bubbles: true })));
    expect(PM.AgentUI.setModel).toHaveBeenCalledWith('gpt-5.6-sol', 'low');
  });

  it('accumulates progress in state while rendering only the newest shimmer line', () => {
    renderPanel(snapshot({ legacyPhase: 'working', requestToken: 4, activity: 'Inspecting source…' }));
    flushSync(() => setAgentSnapshot(snapshot({ legacyPhase: 'working', requestToken: 4, activity: 'Designing the edit…' })));

    const log = target.querySelector('[role="log"]')!;
    expect(log.getAttribute('aria-live')).toBe('polite');
    expect(target.querySelector('[role="status"]')).toBeNull();
    expect(agentState.progressLines).toEqual([
      'Inspecting source…',
      'Designing the edit…'
    ]);
    // Before the trace arrives the activity string is one shimmering line —
    // no per-word stagger, which fought the shimmer's own text clip.
    expect(log.querySelector('.agent-trace .shimmer-text')?.textContent).toBe('Designing the edit…');
    expect(log.querySelector('.agent-thinking-word')).toBeNull();
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

  it('renders an extension recovery action inside the failing result turn', () => {
    renderPanel(snapshot({
      legacyPhase: 'conversation',
      conversation: [{
        role: 'assistant',
        text: "Broken Mod didn't load: Unexpected token",
        fixExtensionId: 'broken-mod'
      }]
    }));

    const button = [...target.querySelectorAll<HTMLButtonElement>('button')]
      .find((candidate) => candidate.textContent === 'Fix it')!;
    button.click();
    expect(PM.SpatialAssistant.requestFix).toHaveBeenCalledExactlyOnceWith('broken-mod');
  });

  it('shows autonomous results once in the conversation without a completion card', () => {
    renderPanel(snapshot({
      legacyPhase: 'result',
      conversation: [{ role: 'assistant', text: 'The title now fades in.' }],
      run: {
        autonomous: true, changed: true, summary: 'The title now fades in.',
        review: { message: 'The editable Powermove result is ready to review.' },
        frames: { images: ['data:image/png;base64,AA=='], times: [0] }
      }
    }));

    const log = target.querySelector('[role="log"]')!;
    expect(target.textContent?.split('The title now fades in.')).toHaveLength(2);
    expect(target.textContent).not.toContain('Autonomous result');
    expect(target.querySelector('.agent-footer .agent-card')).toBeNull();
    expect(target.textContent).not.toContain('Keep change');
    expect(target.textContent).not.toContain('ready to review');
    const undo = [...log.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === 'Undo change')!;
    undo.click();
    expect(PM.AgentUI.undoSceneRun).toHaveBeenCalledOnce();
    expect(log.querySelector('details')?.open).toBe(false);
    expect(log.querySelector('details img')?.getAttribute('alt')).toBe('Rendered composition at 0 seconds');
  });

  it('keeps files, notes, and warnings inline for an autonomous response with no source changes', () => {
    PM.assetKind.mockReturnValue('video');
    renderPanel(snapshot({
      legacyPhase: 'result',
      conversation: [{ role: 'assistant', text: 'I found a clip.' }],
      run: {
        autonomous: true, changed: false, summary: 'I found a clip.',
        review: { message: 'Review the license before publishing.' },
        reviewError: 'The clip could not be imported.',
        externalActions: ['Downloaded reference footage'],
        artifacts: [{ name: 'clip.mp4', path: 'artifacts/clip.mp4', mime: 'video/mp4', size: 2048 }]
      }
    }));

    const log = target.querySelector('[role="log"]')!;
    expect(log.textContent).toContain('Review the license before publishing.');
    expect(log.textContent).toContain('The clip could not be imported.');
    expect(log.textContent).toContain('Downloaded reference footage');
    expect(target.textContent).not.toContain('Undo change');
    expect(target.textContent).not.toContain('Done');
    log.querySelector<HTMLButtonElement>('[aria-label="Add clip.mp4 to timeline"]')!.click();
    log.querySelector<HTMLButtonElement>('[aria-label="Reveal clip.mp4 in Finder"]')!.click();
    expect(PM.AgentUI.importArtifact).toHaveBeenCalledOnce();
    expect(PM.AgentUI.revealArtifact).toHaveBeenCalledOnce();
    expect(target.querySelector('textarea')?.disabled).toBe(false);

    flushSync(() => setAgentSnapshot(snapshot({
      legacyPhase: 'result', conversation: [{ role: 'assistant', text: 'No changes needed.' }],
      run: { autonomous: true, changed: false, review: { message: 'The agent run completed without changing Powermove source.' } }
    })));
    expect(target.textContent).not.toContain('completed without changing');
  });

  it('preserves the review card for a non-autonomous scene edit', () => {
    renderPanel(snapshot({ legacyPhase: 'result', run: { review: { message: 'Check the timing.' } } }));
    const card = target.querySelector('.agent-footer .agent-card')!;
    expect(card.textContent).toContain('Rendered result');
    const buttons = [...card.querySelectorAll<HTMLButtonElement>('button')];
    buttons.find(button => button.textContent === 'Undo change')!.click();
    buttons.find(button => button.textContent === 'Keep change')!.click();
    expect(PM.AgentUI.undoSceneRun).toHaveBeenCalledOnce();
    expect(PM.AgentUI.keepSceneRun).toHaveBeenCalledOnce();
  });
});

describe('agent bridge', () => {
  it('coalesces active-run updates on a 32 ms trailing timer and flushes final state', () => {
    vi.useFakeTimers();
    try {
      const legacyState = snapshot({ legacyPhase: 'working', activity: 'Starting' });
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
      const initialRevision = agentState.revision;

      legacyState.activity = 'Inspecting';
      registry.AgentUI.update();
      legacyState.activity = 'Editing';
      registry.AgentUI.update();
      expect(agentState.revision).toBe(initialRevision);

      vi.advanceTimersByTime(32);
      expect(agentState.revision).toBe(initialRevision + 1);
      expect(agentState.activity).toBe('Editing');

      legacyState.activity = 'Finishing';
      registry.AgentUI.update();
      legacyState.activity = '';
      legacyState.legacyPhase = 'conversation';
      registry.AgentUI.update({ flush: true });
      expect(agentState.phase).toBe('prompt');
      expect(agentState.activity).toBe('');
      const finalRevision = agentState.revision;
      vi.advanceTimersByTime(32);
      expect(agentState.revision).toBe(finalRevision);
    } finally {
      vi.useRealTimers();
    }
  });

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
    const registry: Record<string, any> = { store, registerPanel: vi.fn(), uid: PM.uid };
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

  it('labels the two persistent agent authorities by what they change', () => {
    const registry: Record<string, any> = {
      store: { get: vi.fn((_key: string, fallback: unknown) => fallback), set: vi.fn() },
      uid: PM.uid,
      registerPanel: vi.fn()
    };
    installSpatial(registry);

    expect(agentState.accessModes.slice(0, 2).map(({ id, label }) => ({ id, label }))).toEqual([
      { id: 'editor', label: 'Edit project' },
      { id: 'project', label: 'Change Powermove (project)' }
    ]);
  });
});
