// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentTask } from '../../../../shared/agent-orchestration';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { groupedToolRow } from './activity-rows';
import { taskTrace } from './task-trace';
import ToolActivity from './ToolActivity.svelte';

const task = (patch: Partial<AgentTask> = {}): AgentTask => ({
  taskId: 'task-one', parentThreadId: 'parent', childThreadId: 'child', childRunId: 'child-run', parentTaskId: null,
  requestId: 'root-run', title: 'Review the animation', providerInstanceId: 'claude', model: 'claude-sonnet-5-5',
  status: 'running', workState: 'working', hasPendingChildRuns: false, summary: null, progress: 'Checking timing', startedAt: 1000, waitTimedOut: false, ...patch
});
let instance: Record<string, any> | undefined;
let target: HTMLDivElement | undefined;
afterEach(async () => { if (instance) await unmount(instance); instance = undefined; target?.remove(); resetBridgeForTests(); });
function render(value: AgentTask) {
  target = document.createElement('div'); document.body.append(target);
  instance = mount(ToolActivity, { target, props: { row: groupedToolRow([taskTrace(value)]), live: value.status === 'running' } });
  flushSync();
  return target;
}

describe('subagent activity rows', () => {
  it('shows a simple task, its selected model, progress and a working Stop control', async () => {
    const cancelTask = vi.fn(async () => {});
    installBridgeForTests({ codex: { cancelTask } } as any);
    const element = render(task());
    expect(element.textContent).toContain('1 task');
    expect(element.textContent).toContain('Claude · Sonnet 5.5');
    const stop = element.querySelector('button[aria-label="Stop Review the animation"]') as HTMLButtonElement;
    expect(stop).toBeTruthy();
    stop.click(); flushSync();
    await vi.waitFor(() => expect(cancelTask).toHaveBeenCalledExactlyOnceWith('root-run', 'task-one'));
  });

  it('shows an expanded readable failure and no live Stop control', () => {
    const element = render(task({ status: 'failed', workState: 'result_available', summary: 'The provider is offline.', endedAt: 2000 }));
    expect(element.querySelector('.is-failed')).toBeTruthy();
    expect(element.querySelector('.agent-tool-row')?.getAttribute('aria-expanded')).toBe('true');
    expect(element.textContent).toContain('The provider is offline.');
    expect(element.querySelector('.agent-task-stop')).toBeNull();
  });

  it('keeps stopped tasks quiet and converts structured results into readable text', () => {
    expect(taskTrace(task({ status: 'cancelled' })).status).toBe('done');
    const element = render(task({ status: 'cancelled', summary: 'The task was stopped.' }));
    expect(element.textContent).toContain('Stopped');
    expect(element.querySelector('.is-failed')).toBeNull();
    expect(taskTrace(task({ status: 'completed', summary: JSON.stringify({ summary: 'Timing fixed.', notes: ['Reviewed the preview.'], commands: [] }) })).output).toBe('Timing fixed.\n\nReviewed the preview.');
  });
});
