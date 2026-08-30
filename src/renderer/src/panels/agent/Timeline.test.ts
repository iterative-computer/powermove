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
    expect(at(0).textContent).toBe('Reading the composition');
    expect(at(1).className).toContain('agent-trace-tool');
    expect(at(1).textContent).toContain('Ran commands and edited files');
    expect(at(2).className).toContain('agent-trace-text');
    expect(at(2).textContent).toBe('The layout is fixed.');
    expect(target.querySelector('.shimmer-text')).toBeNull();
  });

  it('exposes the collapsed step labels as a title tooltip', () => {
    render(trace([
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'npm test', status: 'done' },
      { kind: 'tool', id: 'x2', toolName: 'edit_file', label: 'Timeline.svelte', status: 'done' }
    ]));

    expect(target.querySelector('.agent-trace-tool')?.getAttribute('title'))
      .toBe('npm test\nTimeline.svelte');
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
    expect(row.querySelector('em')).toBeNull();
  });

  it('pulses exactly one row even when stale live flags remain', () => {
    render(trace([
      { kind: 'thought', id: 't0', label: 'Old plan', live: true },
      { kind: 'tool', id: 'x1', toolName: 'bash', label: 'npm test', status: 'running' }
    ]));

    const pulsing = target.querySelectorAll('.agent-trace .is-pulsing, .agent-trace .shimmer-text');
    expect(pulsing).toHaveLength(1);
    expect(pulsing[0]?.className).toContain('agent-trace-tool');
  });

  it('shimmers a live plain thought but pulses a bold action thought', () => {
    render(trace([{ kind: 'thought', id: 't0', label: 'Thinking it through', live: true }]));
    expect(target.querySelector('.agent-trace-thought')?.className).toContain('shimmer-text');
    flushSync();

    Object.assign(agentState, {
      trace: [{ kind: 'thought', id: 't1', label: 'Now **editing the file**', live: true }]
    });
    flushSync();
    const row = target.querySelector('.agent-trace-thought')!;
    expect(row.className).toContain('is-action');
    expect(row.className).toContain('is-pulsing');
    expect(row.className).not.toContain('shimmer-text');
    expect(row.textContent).toBe('Now editing the file');
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

    Object.assign(agentState, { phase: 'preview' });
    flushSync();
    expect(target.querySelector('.agent-step')?.textContent).toContain('Inspect composition');
  });
});
