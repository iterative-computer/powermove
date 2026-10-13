import { afterEach, describe, expect, it, vi } from 'vitest';
import { manageProject } from './project-tools';
import { makePM } from '../__tests__/make-pm';
afterEach(() => vi.restoreAllMocks());

function editor() {
  const PM = makePM('core/model', 'core/projects', 'core/history', 'core/editing', 'assistant/harness');
  PM.proj = PM.mkProject({ name: 'Original' });
  PM.Projects.put(PM.proj);
  PM.store.flush = vi.fn(async () => {});
  PM.openProjectHere = vi.fn(async (id: string) => { PM.proj = PM.Projects.get(id); PM.hist.clear(); return true; });
  return PM;
}

describe('project lifecycle tools', () => {
  it('creates, renames, duplicates, saves, trashes and restores independent project records', async () => {
    const PM = editor();
    const original = PM.proj.id;
    await manageProject(PM, { operation: 'create', name: 'Film', w: 640, h: 360, fps: 24, dur: 12 });
    expect(PM.proj).toMatchObject({ name: 'Film', w: 640, h: 360, fps: 24, dur: 12 });
    const film = PM.proj.id;
    await manageProject(PM, { operation: 'rename', name: 'Final film' });
    const copy: any = await manageProject(PM, { operation: 'duplicate', name: 'Copy' });
    expect(copy.projectId).not.toBe(film);
    expect(PM.Projects.get(original).name).toBe('Original');
    await manageProject(PM, { operation: 'trash', projectId: copy.projectId });
    expect(PM.Projects.list().some((p: any) => p.id === copy.projectId)).toBe(false);
    await manageProject(PM, { operation: 'restore', projectId: copy.projectId });
    await manageProject(PM, { operation: 'save' });
    expect(PM.Projects.get(film).name).toBe('Final film');
    expect(PM.store.flush).toHaveBeenCalled();
    await expect(manageProject(PM, { operation: 'trash', projectId: film })).rejects.toThrow('Close');
    await expect(manageProject(PM, { operation: 'create', name: 'Bad', fps: NaN })).rejects.toThrow('fps');
    await expect(manageProject(PM, { operation: 'create', name: 'Bad', w: 1.5 })).rejects.toThrow();
  });

});

it('keeps project threads pinned while allowing app threads to open projects', async () => {
  const PM = editor();
  const other = PM.mkProject({ name: 'Other' }); PM.Projects.put(other);
  const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId: 'library-run', callId: tool, tool, arguments: args });
  PM.SpatialAssistant = { canSwitchRunProject: () => false };
  await expect(call('manage_project', { operation: 'open', projectId: other.id })).rejects.toThrow('app thread');
  expect(PM.openProjectHere).not.toHaveBeenCalled();
  PM.SpatialAssistant.canSwitchRunProject = () => true;
  await call('manage_project', { operation: 'open', projectId: other.id });
  expect(PM.proj.id).toBe(other.id);
  expect((await call('__finish_run', { commit: true })).changed).toBe(true);
});

it('preserves durable library changes and refuses rollback across that boundary', async () => {
  const PM = editor();
  const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId: 'rename-run', callId: tool, tool, arguments: args });
  await call('manage_project', { operation: 'rename', name: 'Final' });
  await call('rollback_changes');
  expect(PM.proj.name).toBe('Final');
  expect(PM.Projects.get(PM.proj.id).name).toBe('Final');
  expect((await call('__finish_run', { commit: true })).changed).toBe(true);
});
