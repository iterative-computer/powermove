// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import Timeline from './Timeline.svelte';
import { agentState, resetAgentState, type AgentSnapshot } from './agent-state.svelte';
import type { TraceStep } from './activity-rows';

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

function baseSnapshot(overrides: Record<string, any> = {}): AgentSnapshot {
  return {
    legacyPhase: 'working',
    requestToken: 1,
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
    model: 'gpt-5.6-sol',
    provider: 'chatgpt',
    reasoningEffort: 'high',
    accessMode: 'editor',
    composerDraft: '',
    pendingEntering: false,
    models: [],
    providers: [],
    reasoningEfforts: [],
    accessModes: [],
    ...overrides
  } as AgentSnapshot;
}

function render(overrides: Record<string, any> = {}): void {
  // `trace` rides along on the snapshot; agent-state copies unknown keys through.
  Object.assign(agentState, baseSnapshot(overrides), {
    phase: overrides.legacyPhase === 'working' || !overrides.legacyPhase ? 'running' : 'preview'
  });
  instance = mount(Timeline, { target, props: { PM: {} } });
  flushSync();
}

const trace = (steps: TraceStep[]): Record<string, any> => ({ trace: steps });

beforeEach(() => {
  target = document.createElement('div');
  document.body.append(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
  resetAgentState();
  delete (agentState as any).trace;
});

describe('Timeline', () => {
  it('falls back to a single shimmer activity line before the trace arrives', () => {
    render({ activity: 'Inspecting the composition…' });
    const line = target.querySelector('.agent-trace .shimmer-text')!;
    expect(line.textContent).toBe('Inspecting the composition…');
    expect(target.querySelectorAll('.agent-pixel-loader > i')).toHaveLength(9);
    expect(target.querySelectorAll('.agent-trace-tool')).toHaveLength(0);
    // The old per-word stagger is gone.
    expect(target.querySelector('.agent-thinking-word')).toBeNull();
  });

  it('renders thought, text, and grouped tool rows in order', () => {
    render({
      activity: 'ignored once the trace exists',
      ...trace([
        { kind: 'thought', id: 't0', label: 'Reading the composition', live: false },
        { kind: 'tool', id: 'x1', toolName: 'bash', label: 'npm test', status: 'done' },
        { kind: 'tool', id: 'x2', toolName: 'edit_file', label: 'Timeline.svelte', status: 'done' },
        { kind: 'text', id: 'm0', text: 'The layout is fixed.' }
      ])
    });

    const rows = [...target.querySelectorAll('.agent-trace > *')] as HTMLElement[];
    const at = (index: number): HTMLElement => rows[index]!;
    expect(rows).toHaveLength(3);
    expect(at(0).className).toContain('agent-trace-thought');
    // A settled thought collapses to its header; the prose stays one click away.
    expect((at(0) as HTMLDetailsElement).open).toBe(false);
    expect(at(0).querySelector('summary')?.textContent?.trim()).toBe('Thought');
    expect(at(0).querySelector('.agent-thinking-body')?.textContent).toBe('Reading the composition');
    expect(at(1).matches('.agent-trace-tool')).toBe(true);
    expect(at(1).textContent).toContain('Ran commands and edited files');
    expect(at(2).className).toContain('agent-trace-text');
    expect(at(2).textContent).toBe('The layout is fixed.');
    expect(target.querySelector('.shimmer-text')).toBeNull();
  });

  it('streams the newest text row with a caret and drops it once work moves on', () => {
    render(trace([{ kind: 'text', id: 'm0', text: 'Looking at the' }]));
    const text = target.querySelector('.agent-trace-text')!;
    expect(text.className).toContain('is-streaming');
    // The caret is a pseudo-element: every real child stays an inline word span.
    expect([...text.children].every((child) => child.tagName === 'SPAN' && !child.className)).toBe(true);
    expect(text.textContent).toBe('Looking at the');

    Object.assign(agentState, { trace: [
      { kind: 'text', id: 'm0', text: 'Looking at the layout.' },
      { kind: 'tool', id: 'x1', toolName: 'read', label: 'Read', detail: 'Timeline.svelte', status: 'running' }
    ] });
    flushSync();
    expect(target.querySelector('.agent-trace-text')?.className).not.toContain('is-streaming');
  });

  it('titles a settled thought with how long it took', () => {
    render(trace([{ kind: 'thought', id: 't0', label: 'Weighing options', live: false, startedAt: 10_000, endedAt: 14_200 }]));
    expect(target.querySelector('.agent-trace-thought summary')?.textContent?.trim()).toBe('Thought for 4s');
  });

  it('shows each call as icon, label, and a mono argument chip, and expands its output', () => {
    render(trace([
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'Run', detail: 'npm test', output: '✓ 34 checks passed', status: 'done', startedAt: 0, endedAt: 1_200 },
      { kind: 'tool', id: 'x2', toolName: 'edit', label: 'Edit', detail: 'Timeline.svelte', status: 'done' }
    ]));
    const rows = [...target.querySelectorAll('.agent-tool-details > div')];
    expect(rows).toHaveLength(2);
    const first = rows[0]!;
    expect(first.querySelector('.agent-tool-glyph')?.getAttribute('data-family')).toBe('run');
    expect(first.querySelector('span')?.textContent).toBe('Run');
    expect(first.querySelector('.agent-tool-chip')?.textContent).toBe('npm test');
    expect(first.querySelector('em.is-time')?.textContent).toBe('1s');
    const toggle = first.querySelector<HTMLButtonElement>('button.agent-tool-row')!;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    toggle.click();
    flushSync();
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(first.querySelector('.agent-tool-output')?.className).toContain('is-open');
    expect(first.querySelector('.agent-tool-output pre')?.textContent).toBe('✓ 34 checks passed');
    // A call without output is not a button: nothing to disclose.
    expect(rows[1]!.querySelector('button')).toBeNull();
    expect(rows[1]!.querySelector('.agent-tool-glyph')?.getAttribute('data-family')).toBe('edit');
    // Settled groups read their size and span in the header.
    expect(target.querySelector('.agent-tool-activity summary small')?.textContent).toBe('2 calls');
  });

  it('exposes the collapsed step labels as a title tooltip', () => {
    render(trace([
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'npm test', status: 'done' },
      { kind: 'tool', id: 'x2', toolName: 'edit_file', label: 'Timeline.svelte', status: 'done' }
    ]));

    expect(target.querySelector('.agent-trace-tool')?.getAttribute('title'))
      .toBe('npm test\nTimeline.svelte');
  });

  it('shows the real active action and its live work log while running', () => {
    render(trace([
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'bash · npm test', status: 'done' },
      { kind: 'tool', id: 'x2', toolName: 'edit_file', label: 'edit · Timeline.svelte', status: 'running' }
    ]));

    const row = target.querySelector('details.agent-trace-tool')!;
    expect(row.hasAttribute('open')).toBe(true);
    expect(row.className).toContain('is-pulsing');
    expect(row.className).toContain('is-running');
    // The open rows name the active call; the header only says work is underway.
    expect(row.querySelector('summary')?.textContent?.trim()).toBe('Running tools');
    expect(row.querySelector('.agent-tool-details .is-running span')?.textContent).toBe('edit · Timeline.svelte');
    expect(row.querySelector('.agent-tool-details .is-running .agent-tool-spinner')).toBeTruthy();
    expect([...row.querySelectorAll('.agent-tool-details span')].map((item) => item.textContent))
      .toEqual(['bash · npm test', 'edit · Timeline.svelte']);
  });

  it('makes completed work primary when one action failed', () => {
    render(trace([
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'bash · npm test', status: 'done' },
      { kind: 'tool', id: 'x2', toolName: 'web_search', label: 'search · motion references', status: 'error' }
    ]));

    const row = target.querySelector('details.agent-trace-tool')!;
    expect(row.querySelector('summary')?.textContent).toContain('1 completed');
    expect(row.querySelector('summary')?.textContent).not.toContain('1 of 2 failed');
    row.setAttribute('open', '');
    expect(row.querySelector('.is-failed')?.textContent).toContain('search · motion references');
    expect(row.querySelector('.is-failed')?.textContent).toContain('Failed');
  });

  it('marks a failed tool group with an error class and a Failed chip', () => {
    render(trace([{ kind: 'tool', id: 'x1', toolName: 'edit', label: 'patch', status: 'error' }]));

    const row = target.querySelector('.agent-trace-tool')!;
    expect(row.className).toContain('is-error');
    expect(row.querySelector('em')?.textContent).toBe('Failed');
  });

  it('renders a continued tool call exactly like a done one', () => {
    render(trace([{ kind: 'tool', id: 'x1', toolName: 'bash', label: 'npm test', status: 'continued' }]));

    const row = target.querySelector('.agent-trace-tool')!;
    expect(row.className).not.toContain('is-error');
    expect(row.className).not.toContain('is-pulsing');
    expect(row.querySelector('summary > em')).toBeNull();
  });

  it('pulses exactly one row even when stale live flags remain', () => {
    render(trace([
      { kind: 'thought', id: 't0', label: 'Old plan', live: true },
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'npm test', status: 'running' }
    ]));

    const pulsing = target.querySelectorAll('.agent-trace .is-pulsing, .agent-trace .shimmer-text');
    expect(pulsing).toHaveLength(1);
    expect(pulsing[0]?.className).toContain('agent-trace-tool');
    // The stale thought stays open (it is still flagged live) but does not shimmer.
    expect(target.querySelector('.agent-thinking-heading')?.className).not.toContain('shimmer-text');
  });

  it('keeps a live thought open under a shimmering Thinking header', () => {
    render(trace([{ kind: 'thought', id: 't0', label: 'Thinking it through', live: true }]));
    const row = target.querySelector<HTMLDetailsElement>('.agent-trace-thought')!;
    expect(row.open).toBe(true);
    expect(row.className).toContain('is-live');
    expect(row.querySelector('.agent-thinking-heading')?.textContent).toBe('Thinking');
    expect(row.querySelector('.agent-thinking-heading')?.className).toContain('shimmer-text');
    expect(row.querySelector('.agent-thinking-body')?.textContent).toBe('Thinking it through');

    Object.assign(agentState, {
      trace: [{ kind: 'thought', id: 't1', label: 'Now **editing the file**', live: true }]
    });
    flushSync();
    const action = target.querySelector('.agent-trace-thought')!;
    expect(action.className).toContain('is-action');
    expect(action.querySelector('.agent-thinking-body')?.textContent).toBe('Now editing the file');
  });

  it('renders inline code spans as code chips', () => {
    render(trace([{ kind: 'text', id: 'm0', text: 'Run `npm test` first.' }]));

    const code = target.querySelectorAll('.agent-trace-code');
    expect([...code].map((el) => el.textContent).join('')).toBe('npm test');
    expect(target.querySelector('.agent-trace-text')?.textContent).toBe('Run npm test first.');
  });

  it('drops the plan step list while running and keeps it in preview', () => {
    render({
      steps: [{ id: 's0', title: 'Inspect composition', status: 'active' }],
      ...trace([{ kind: 'thought', id: 't0', label: 'Working', live: true }])
    });
    expect(target.querySelector('.agent-step')).toBeNull();
    expect(target.querySelector('.agent-trace-thought')).toBeTruthy();
    expect(target.querySelector('.agent-trace')?.className).toContain('is-live');

    Object.assign(agentState, { phase: 'preview' });
    flushSync();
    expect(target.querySelector('.agent-step')?.textContent).toContain('Inspect composition');
  });
});
