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
  it('leaves the project and draft alone when another run prevents switching context', async () => {
    const PM = { ...host(), ProjectsScreen: { isOpen: true }, newBlankProject: vi.fn() };
    PM.AgentUI.openGlobal.mockReturnValue(false);
    const connected = vi.fn(async () => true);
    expect(await workspaceImportHandoff(PM, connected).start('after-effects')).toBe(false);
    expect(PM.newBlankProject).not.toHaveBeenCalled();
    expect(PM.AgentUI.newThread).not.toHaveBeenCalled();
    expect(PM.AgentUI.setDraft).not.toHaveBeenCalled();
    expect(connected).not.toHaveBeenCalled();
  });
  it('keeps the selected agent and model when the app conversation has different saved choices', async () => {
    const PM = host();
    Object.assign(PM.AgentUI.state, { provider: 'claude', model: 'opus', reasoningEffort: 'high' });
    PM.AgentUI.openGlobal.mockImplementation(() => {
      Object.assign(PM.AgentUI.state, { provider: 'chatgpt', model: 'gpt-5.6-sol', reasoningEffort: 'low' });
    });
    const setProvider = vi.fn(); const setModel = vi.fn();
    Object.assign(PM.AgentUI, { setProvider, setModel });
    await workspaceImportHandoff(PM, async () => true).start('after-effects');
    expect(setProvider).toHaveBeenCalledExactlyOnceWith('claude');
    expect(setModel).toHaveBeenCalledExactlyOnceWith('opus', 'high');
  });
  it('never sends a changed draft or a request in a different thread', async () => {
    const PM = host(); const handoff = workspaceImportHandoff(PM, async () => false);
    await handoff.start('after-effects');
    PM.AgentUI.state.threadId = 'other'; handoff.resume();
    PM.AgentUI.state.threadId = 'import-thread'; PM.AgentUI.state.composerDraft = 'New request'; handoff.resume();
    expect(PM.AgentUI.submit).not.toHaveBeenCalled();
  });
});
