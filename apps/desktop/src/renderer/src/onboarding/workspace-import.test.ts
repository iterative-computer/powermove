import { describe, expect, it, vi } from 'vitest';
import { WORKSPACE_IMPORT_LABEL } from '../../../shared/creative-workspace';
import { workspaceImportHandoff } from './workspace-import';

function host() {
  const state = { threadId: 'import-thread', context: 'app', phase: 'idle', composerDraft: '' };
  const AgentUI = { state, openGlobal: vi.fn(), newThread: vi.fn(), setScope: vi.fn(),
    setDraft: vi.fn(value => { state.composerDraft = value; }), submit: vi.fn() };
  return { AgentUI };
}

describe('onboarding workspace agent handoff', () => {
  it('starts one app request automatically when connected', async () => {
    const PM = host(); const handoff = workspaceImportHandoff(PM, async () => true);
    expect(await handoff.start('after-effects')).toBe(true);
    expect(PM.AgentUI.newThread).toHaveBeenCalledOnce();
    expect(PM.AgentUI.submit).toHaveBeenCalledExactlyOnceWith(WORKSPACE_IMPORT_LABEL);
    handoff.resume(); expect(PM.AgentUI.submit).toHaveBeenCalledOnce();
  });
  it('waits for sign-in and leaves the original draft available if status fails', async () => {
    const PM = host(); const handoff = workspaceImportHandoff(PM, async () => { throw new Error('Offline'); });
    await handoff.start('after-effects'); expect(PM.AgentUI.submit).not.toHaveBeenCalled();
    expect(PM.AgentUI.state.composerDraft).toBe(WORKSPACE_IMPORT_LABEL);
    handoff.resume(); expect(PM.AgentUI.submit).toHaveBeenCalledOnce();
  });
  it('opens a project from the home screen so the workspace is arranged around it', async () => {
    const PM: Record<string, any> = { ...host(), ProjectsScreen: { isOpen: true }, newBlankProject: vi.fn() };
    await workspaceImportHandoff(PM, async () => true).start('after-effects');
    expect(PM.newBlankProject).toHaveBeenCalledOnce();
    const inProject: Record<string, any> = { ...host(), ProjectsScreen: { isOpen: false }, isHomeProject: () => false, newBlankProject: vi.fn() };
    await workspaceImportHandoff(inProject, async () => true).start('after-effects');
    expect(inProject.newBlankProject).not.toHaveBeenCalled();
  });
  it('never sends a changed draft or a request in a different thread', async () => {
    const PM = host(); const handoff = workspaceImportHandoff(PM, async () => false);
    await handoff.start('after-effects');
    PM.AgentUI.state.threadId = 'other'; handoff.resume();
    PM.AgentUI.state.threadId = 'import-thread'; PM.AgentUI.state.composerDraft = 'New request'; handoff.resume();
    expect(PM.AgentUI.submit).not.toHaveBeenCalled();
  });
});
