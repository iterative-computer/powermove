import { describe, expect, it, vi } from 'vitest';

import type { CodexTraceEvent } from '../shared/ipc';
import { AgentApprovals, ALLOW_ONCE, ALLOW_RUN, DENY } from './agent-approvals';

const request = { title: 'Run a command outside the project sandbox?', detail: 'cp DMSans.ttf ~/Library/Fonts/', reason: 'Install the font.' };

function asked(trace: ReturnType<typeof vi.fn>): Extract<CodexTraceEvent, { kind: 'question' }> {
  const step = trace.mock.calls.at(-1)?.[0] as CodexTraceEvent;
  if (step?.kind !== 'question') throw new Error('No approval card was shown.');
  return step;
}

describe('agent approvals', () => {
  it('shows a blocking card and allows once', async () => {
    const approvals = new AgentApprovals();
    const trace = vi.fn();
    const decision = approvals.requester('run-1', trace)(request);
    const card = asked(trace);
    expect(card).toMatchObject({ transport: 'reply', blocking: true });
    expect(card.questions[0]!.question).toContain('cp DMSans.ttf ~/Library/Fonts/');
    expect(card.questions[0]!.options.map(option => option.label)).toEqual([ALLOW_ONCE, ALLOW_RUN, DENY]);

    expect(approvals.answer({ id: 'run-1', itemId: card.itemId, answers: { decision: [ALLOW_ONCE] } })).toBe(true);
    await expect(decision).resolves.toEqual({ allowed: true });
    // Once: the next action asks again.
    void approvals.requester('run-1', trace)(request);
    expect(asked(trace).itemId).not.toBe(card.itemId);
  });

  it('stops asking for the rest of a run allowed as a whole, and only that run', async () => {
    const approvals = new AgentApprovals();
    const trace = vi.fn();
    const ask = approvals.requester('run-1', trace);
    const first = ask(request);
    approvals.answer({ id: 'run-1', itemId: asked(trace).itemId, answers: { decision: [ALLOW_RUN] } });
    await expect(first).resolves.toEqual({ allowed: true });
    trace.mockClear();
    await expect(ask(request)).resolves.toEqual({ allowed: true });
    expect(trace).not.toHaveBeenCalled();

    approvals.closeRun('run-1');
    void ask(request);
    expect(trace).toHaveBeenCalledOnce();
  });

  it('declines on Deny, Skip or a typed note, and ignores answers for another run', async () => {
    const approvals = new AgentApprovals();
    const trace = vi.fn();
    const ask = approvals.requester('run-1', trace);

    const denied = ask(request);
    const card = asked(trace);
    expect(approvals.answer({ id: 'run-2', itemId: card.itemId, answers: { decision: [ALLOW_ONCE] } })).toBe(false);
    approvals.answer({ id: 'run-1', itemId: card.itemId, answers: { decision: [DENY] } });
    await expect(denied).resolves.toMatchObject({ allowed: false });

    const skipped = ask(request);
    approvals.answer({ id: 'run-1', itemId: asked(trace).itemId, answers: {} });
    await expect(skipped).resolves.toMatchObject({ allowed: false });

    const noted = ask(request);
    approvals.answer({ id: 'run-1', itemId: asked(trace).itemId, answers: { decision: ['Put it in the project instead'] } });
    await expect(noted).resolves.toEqual({ allowed: false, message: 'The user declined and said: Put it in the project instead' });
  });

  it('denies what is still waiting when the run ends or the provider cancels', async () => {
    const approvals = new AgentApprovals();
    const trace = vi.fn();
    const ask = approvals.requester('run-1', trace);

    const ended = ask(request);
    approvals.closeRun('run-1');
    await expect(ended).resolves.toMatchObject({ allowed: false });

    const controller = new AbortController();
    const cancelled = ask(request, controller.signal);
    const card = asked(trace);
    controller.abort();
    await expect(cancelled).resolves.toMatchObject({ allowed: false });
    expect(trace).toHaveBeenLastCalledWith({ kind: 'question-closed', itemId: card.itemId });
    expect(approvals.answer({ id: 'run-1', itemId: card.itemId, answers: { decision: [ALLOW_ONCE] } })).toBe(false);
  });
});
