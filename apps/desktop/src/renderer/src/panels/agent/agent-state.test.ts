import { afterEach, describe, expect, it } from 'vitest';

import {
  agentState,
  resetAgentState,
  setAgentSnapshot,
  type AgentSnapshot,
  type TraceStep
} from './agent-state.svelte';

function snapshot(trace: TraceStep[]): AgentSnapshot {
  return {
    legacyPhase: 'working',
    requestToken: 1,
    conversation: [],
    activity: '',
    trace,
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
    permission: 'editor',
    composerDraft: '',
    pendingEntering: false,
    models: [],
    providers: [],
    reasoningEfforts: [],
    accessModes: []
  };
}

afterEach(() => resetAgentState());

it('reconciles visual artifacts across assistant rows and preserves unchanged frame inputs', () => {
  const artifact = { version: 1 as const, id: 'visual-first', runId: 'run', title: 'First', html: '<h1>First</h1>', height: 300 };
  const first = { ...snapshot([]), conversation: [{ role: 'assistant' as const, text: '', visualReplies: [artifact] }] };
  setAgentSnapshot(first);
  const inputs = agentState.conversation[0]!.visualReplies;
  setAgentSnapshot({ ...first, conversation: [{ ...first.conversation[0]!, visualReplies: [{ ...artifact }] }] });
  expect(agentState.conversation[0]!.visualReplies).toBe(inputs);
  const existing = inputs![0];
  setAgentSnapshot({ ...first, conversation: [{ ...first.conversation[0]!, visualReplies: [artifact, { ...artifact, id: 'visual-added' }] }] });
  expect(agentState.conversation[0]!.visualReplies?.[0]).toBe(existing);
  expect(agentState.conversation[0]!.visualReplies).toHaveLength(2);
  setAgentSnapshot({ ...first, conversation: [{ role: 'assistant', text: '', visualReplies: [{ ...artifact, id: 'visual-second', title: 'Second' }] }] });
  expect(agentState.conversation[0]!.visualReplies?.[0]?.id).toBe('visual-second');
  setAgentSnapshot({ ...first, conversation: [{ role: 'assistant', text: 'Plain reply' }] });
  expect(agentState.conversation[0]!.visualReplies).toBeUndefined();
});

describe('setAgentSnapshot trace reconciliation', () => {
  it('preserves an approval answer through live status changes, archival, and replay', () => {
    const question: TraceStep = { kind: 'question', id: 'approval-1', transport: 'reply', blocking: true,
      requestId: 'run-1', status: 'open', questions: [{ id: 'decision', header: 'Approval needed',
        question: 'Run a harmless fixture command?', options: [{ label: 'Allow once', description: '' }],
        allowOther: true, secret: false }] };
    setAgentSnapshot(snapshot([question]));
    const row = agentState.trace[0];
    const answered: TraceStep = { ...question, status: 'answered', answers: { decision: 'Allow once' } };
    setAgentSnapshot(snapshot([answered, { kind: 'tool', id: 'tool', toolName: 'fixture', label: 'Fixture completed', status: 'done' }]));
    expect(agentState.trace[0]).toBe(row);
    expect(agentState.trace[0]).toMatchObject({ status: 'answered', answers: { decision: 'Allow once' } });
    const archived = { ...snapshot([]), conversation: [{ role: 'trace' as const, steps: [answered] }] };
    setAgentSnapshot(JSON.parse(JSON.stringify(archived)));
    expect(agentState.conversation[0]?.steps?.[0]).toMatchObject({ answers: { decision: 'Allow once' } });
    resetAgentState();
    setAgentSnapshot(JSON.parse(JSON.stringify(archived)));
    expect(agentState.conversation[0]?.steps?.[0]).toMatchObject({ answers: { decision: 'Allow once' } });
  });

  it('clears absent answers when a question is closed instead of retaining stale approval text', () => {
    const question: TraceStep = { kind: 'question', id: 'approval-1', transport: 'reply', blocking: true,
      status: 'answered', questions: [], answers: { decision: 'Allow once' } };
    setAgentSnapshot(snapshot([question]));
    const { answers: _answers, ...closed } = question;
    setAgentSnapshot(snapshot([{ ...closed, status: 'closed' }]));
    expect(agentState.trace[0]).toMatchObject({ status: 'closed' });
    expect((agentState.trace[0] as any).answers).toBeUndefined();
  });


  it('preserves existing row identity when only the last text row grows', () => {
    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false },
      { kind: 'text', id: 'text-1', text: 'Reading ' }
    ]));
    const firstRow = agentState.trace[0];

    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false },
      { kind: 'text', id: 'text-1', text: 'Reading the source' }
    ]));

    expect(agentState.trace[0]).toBe(firstRow);
    expect(agentState.trace.at(-1)).toMatchObject({ text: 'Reading the source' });
  });

  it('truncates rows missing from a shorter snapshot', () => {
    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false },
      { kind: 'text', id: 'text-1', text: 'Done' }
    ]));

    setAgentSnapshot(snapshot([
      { kind: 'thought', id: 'thought-1', label: 'Planning', live: false }
    ]));

    expect(agentState.trace).toHaveLength(1);
    expect(agentState.trace[0]?.id).toBe('thought-1');
  });

  it('replaces a row when its id changes at the same index', () => {
    setAgentSnapshot(snapshot([
      { kind: 'text', id: 'text-1', text: 'First' }
    ]));
    const originalRow = agentState.trace[0];

    setAgentSnapshot(snapshot([
      { kind: 'text', id: 'text-2', text: 'Second' }
    ]));

    expect(agentState.trace[0]).not.toBe(originalRow);
    expect(agentState.trace[0]).toEqual({ kind: 'text', id: 'text-2', text: 'Second' });
  });
});
