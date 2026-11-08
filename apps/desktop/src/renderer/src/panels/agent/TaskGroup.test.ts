// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentTask } from '../../../../shared/agent-orchestration';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { activityRows, threadToolLabel, type TasksRow } from './activity-rows';
import type { AgentMessage, TraceStep } from './agent-state.svelte';
import { taskModel, taskTrace, taskTree } from './task-trace';
import TaskGroup from './TaskGroup.svelte';
import { agentState } from './agent-state.svelte';
import { AgentThreads } from './threads';
import { groupConversation, workLabel } from './work-groups';

const task = (patch: Partial<AgentTask> = {}): AgentTask => ({
  taskId: 'timing', parentThreadId: 'parent', childThreadId: 'child', childRunId: 'child-run', parentTaskId: null,
  requestId: 'root-run', title: 'Tighten the timing', providerInstanceId: 'claude', model: 'claude-sonnet-5-5',
  status: 'running', workState: 'working', hasPendingChildRuns: false, summary: null, progress: 'Moving keyframes', startedAt: 1000, waitTimedOut: false, ...patch
});
const tool = (id: string, toolName = 'shell'): TraceStep => ({ kind: 'tool', id, toolName, label: id, status: 'done' } as TraceStep);

let instance: Record<string, any> | undefined;
let target: HTMLDivElement | undefined;
afterEach(async () => { if (instance) await unmount(instance); instance = undefined; target?.remove(); resetBridgeForTests(); agentState.threads = []; });
function render(tasks: AgentTask[], PM?: Record<string, any>) {
  target = document.createElement('div'); document.body.append(target);
  const row = activityRows(tasks.map(taskTrace))[0] as TasksRow;
  instance = mount(TaskGroup, { target, props: { row, PM } });
  flushSync();
  return target;
}

describe('subagent card', () => {
  it('pulls subagents out of tool groups into one card, nesting late subagents under their parent', () => {
    const rows = activityRows([
      tool('read'),
      taskTrace(task()),
      taskTrace(task({ taskId: 'title', title: 'Rebuild the title' })),
      { kind: 'text', id: 'note', text: 'Meanwhile…' },
      taskTrace(task({ taskId: 'easing', parentTaskId: 'title', title: 'Check easing' }))
    ]);
    expect(rows.map(row => row.kind)).toEqual(['tools', 'tasks', 'text']);
    expect((rows[1] as TasksRow).tasks.map(item => item.taskId)).toEqual(['timing', 'title', 'easing']);
    expect(taskTree((rows[1] as TasksRow).tasks).map(item => [item.task.taskId, item.depth])).toEqual([['timing', 0], ['title', 0], ['easing', 1]]);
  });

  it('says who is doing what, live, and stops one subagent', async () => {
    const cancelTask = vi.fn(async () => {});
    installBridgeForTests({ codex: { cancelTask } } as any);
    const element = render([task(), task({ taskId: 'color', title: 'Review the color', status: 'completed', summary: JSON.stringify({ summary: 'The grade is consistent. Scene 4 is warm.' }), endedAt: 49_000 })]);
    expect(element.querySelector('.agent-subagents-title')?.textContent).toBe('2 subagents');
    expect(element.querySelector('.agent-subagents-summary')?.textContent).toBe('1 working · 1 done');
    expect(element.textContent).toContain('Claude Sonnet 5.5');
    expect(element.textContent).toContain('Moving keyframes');
    expect(element.textContent).toContain('The grade is consistent.');
    expect(element.textContent).toContain('48s');
    (element.querySelector('[aria-label="Stop Tighten the timing"]') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(cancelTask).toHaveBeenCalledExactlyOnceWith('root-run', 'timing'));
    expect(element.querySelector('[aria-label="Stop Review the color"]')).toBeNull();
    const open = element.querySelector('li[data-phase="done"] .agent-subagent-row') as HTMLButtonElement;
    open.click(); flushSync();
    expect(element.querySelector('.agent-subagent-result')?.textContent).toContain('Scene 4 is warm.');
  });

  it('shows why a subagent failed and that a parent waits on its own subagents', () => {
    const element = render([
      task({ taskId: 'title', title: 'Rebuild the title', status: 'failed', summary: 'The connection closed.', endedAt: 5000 }),
      task({ taskId: 'wait', title: 'Plan the cut', status: 'waiting', workState: 'waiting_for_children' })
    ]);
    expect(element.querySelector('li[data-phase="failed"]')?.textContent).toContain('The connection closed.');
    expect(element.querySelector('li[data-phase="waiting"]')?.textContent).toContain('Waiting on its subagents');
    expect(element.querySelector('.agent-subagents-summary')?.textContent).toBe('1 working · 1 failed');
  });

  it('opens a subagent’s sub-thread from its row', () => {
    const switchThread = vi.fn();
    agentState.threads = [{ id: 'parent', title: 'Polish' }, { id: 'sub-timing', title: 'Tighten the timing', parentId: 'parent', kind: 'subagent' }];
    const element = render([task(), task({ taskId: 'old', title: 'Older work', status: 'completed', summary: 'Done. '.repeat(60) })], { AgentUI: { switchThread } });
    (element.querySelector('[aria-label="Open Tighten the timing"]') as HTMLButtonElement).click();
    expect(switchThread).toHaveBeenCalledExactlyOnceWith('sub-timing');
    // Work from before sub-threads existed still unfolds in place.
    const older = element.querySelector('li[data-phase="done"] .agent-subagent-row') as HTMLButtonElement;
    expect(older.getAttribute('aria-expanded')).toBe('false');
  });

  it('never reuses a sub-thread as a blank new thread', () => {
    expect(AgentThreads.isBlank({ id: 'sub-x', title: 'x', updatedAt: 0, conversation: [], composerDraft: '', attachments: [], scope: 'workspace', parentId: 'p' })).toBe(false);
  });

  it('names models a person can read', () => {
    expect(taskModel({ providerInstanceId: 'claude', model: 'claude-haiku-5-5' })).toBe('Haiku 5.5');
    expect(taskModel({ providerInstanceId: 'claude', model: 'claude-sonnet-5-5' })).toBe('Sonnet 5.5');
  });
});

describe('thread and fold wording', () => {
  it('labels thread tools in plain words', () => {
    expect(threadToolLabel('mcp__powermove__thread_launch')).toBe('Started a thread');
    expect(threadToolLabel('thread_send')).toBe('Messaged a thread');
    expect(threadToolLabel('shell')).toBeUndefined();
  });

  it('counts subagents in a folded turn', () => {
    const conversation: AgentMessage[] = [
      { role: 'user', text: 'Polish it' },
      { role: 'trace', durationMs: 90_000, steps: [
        taskTrace(task()), taskTrace(task({ status: 'completed', summary: 'Done.' })),
        taskTrace(task({ taskId: 'title', status: 'failed', summary: 'Offline.' })),
        { kind: 'text', id: 'reply', text: 'All done.' }
      ] }
    ];
    const work = groupConversation(conversation, null).find(entry => entry.kind === 'work')!;
    expect(workLabel(work)).toBe('Worked for 1m 30s · 2 subagents, 1 failed');
    expect(workLabel({ durationMs: 5000 })).toBe('Worked for 5s');
  });
});
