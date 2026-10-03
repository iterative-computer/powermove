import { describe, expect, it } from 'vitest';
import { currentStepLabel, elapsedLabel, launcherStatus, replyLead, type LauncherInput } from './launcher-status';

const idle: LauncherInput = { phase: 'idle', trace: [], activity: '', conversation: [] };

describe('agent launcher status', () => {
  it('invites a request when nothing is happening', () => {
    expect(launcherStatus(idle, false)).toEqual({ tone: 'idle', label: 'Ask the agent', startedAt: null, tasks: 0 });
  });

  it('names the step the run is on and counts every running task', () => {
    const status = launcherStatus({
      ...idle, phase: 'running', runStartedAt: 1000, backgroundRuns: 2,
      trace: [
        { kind: 'tool', id: 'a', toolName: 'get_panel_layout', label: 'Get panel layout', status: 'done' },
        { kind: 'tool', id: 'b', toolName: 'write_file', label: 'Write file', status: 'running' }
      ]
    }, false);
    expect(status).toEqual({ tone: 'working', label: 'Write file', startedAt: 1000, tasks: 3 });
  });

  it('falls back from live thoughts to the run activity', () => {
    expect(currentStepLabel([{ kind: 'thought', id: 't', label: 'Planning the layout. Then more.', live: true }])).toBe('Planning the layout.');
    expect(launcherStatus({ ...idle, phase: 'running', activity: 'Arranging your workspace with the new panels…' }, false).label)
      .toBe('Arranging your workspace with the new panels');
    expect(launcherStatus({ ...idle, phase: 'running', threadTitle: 'Bring my After Effects workspace' }, false).label).toBe('Bring my After Effects workspace');
    expect(launcherStatus({ ...idle, phase: 'running' }, false).label).toBe('Working');
  });

  it('asks for attention when a blocking question is open', () => {
    const status = launcherStatus({ ...idle, phase: 'running', trace: [
      { kind: 'question', id: 'q', questions: [], transport: 'reply', blocking: true, status: 'open' }
    ] }, false);
    expect(status.tone).toBe('attention');
    expect(status.label).toBe('Needs your answer');
  });

  it('asks for review of proposed or applied edits', () => {
    expect(launcherStatus({ ...idle, phase: 'preview' }, false).tone).toBe('review');
    expect(launcherStatus({ ...idle, phase: 'result', run: { autonomous: false } }, false).label).toBe('Review the changes');
    expect(launcherStatus({ ...idle, phase: 'result', run: { autonomous: true } }, false).tone).toBe('idle');
  });

  it('reports an unread reply or failure until it is opened', () => {
    const conversation = [{ role: 'assistant' as const, text: '**Arranged** your After Effects workspace.\n\nSandbox check passed.' }];
    expect(launcherStatus({ ...idle, conversation }, true)).toMatchObject({ tone: 'done', label: 'Arranged your After Effects workspace.' });
    expect(launcherStatus({ ...idle, conversation }, false).tone).toBe('idle');
    const failed = [{ role: 'assistant' as const, text: 'The model could not be reached.', error: true }];
    expect(launcherStatus({ ...idle, conversation: failed }, true).tone).toBe('error');
    const plain = [{ role: 'assistant' as const, text: 'Nothing needed changing.', error: true, notice: 'plain' as const }];
    expect(launcherStatus({ ...idle, conversation: plain }, true).tone).toBe('done');
  });

  it('keeps background work visible while this thread is idle', () => {
    expect(launcherStatus({ ...idle, backgroundRuns: 1 }, false)).toMatchObject({ tone: 'working', label: '1 task running', tasks: 1 });
  });

  it('formats compact labels', () => {
    expect(replyLead('x'.repeat(100))).toHaveLength(72);
    expect(elapsedLabel(9_400)).toBe('9s');
    expect(elapsedLabel(184_000)).toBe('3:04');
    expect(elapsedLabel(3_729_000)).toBe('1:02:09');
  });
});
