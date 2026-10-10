import { WORKSPACE_IMPORT_LABEL, type CreativeAppId } from '../../../shared/creative-workspace';

type AgentHost = Record<string, any>;

/** Holds the opt-in until account setup finishes; never sends a changed draft. */
export function workspaceImportHandoff(PM: AgentHost, connected: () => Promise<boolean>) {
  let pendingThread: string | null = null;
  const resume = () => {
    const state = PM.AgentUI?.state;
    if (!pendingThread || state?.threadId !== pendingThread || state.context !== 'app'
      || state.composerDraft !== WORKSPACE_IMPORT_LABEL || state.phase === 'running') return;
    pendingThread = null;
    PM.AgentUI.submit(WORKSPACE_IMPORT_LABEL);
  };
  return {
    async start(appId: CreativeAppId): Promise<boolean> {
      if (appId !== 'after-effects' || !PM.AgentUI?.openGlobal) return false;
      const { provider, model, reasoningEffort } = PM.AgentUI.state ?? {};
      if (PM.AgentUI.openGlobal() === false) return false;
      // Arrange the imported workspace around a project, not the empty home.
      if (PM.ProjectsScreen?.isOpen || PM.isHomeProject?.()) PM.newBlankProject?.();
      PM.AgentUI.newThread();
      // App conversations have their own saved choices; keep the agent the
      // person selected before starting this transfer.
      if (provider) PM.AgentUI.setProvider?.(provider);
      if (model && reasoningEffort) PM.AgentUI.setModel?.(model, reasoningEffort);
      PM.AgentUI.setScope('workspace');
      PM.AgentUI.setDraft(WORKSPACE_IMPORT_LABEL, true);
      pendingThread = PM.AgentUI.state.threadId;
      try { if (await connected()) resume(); } catch { /* Connection gate keeps the queued request visible. */ }
      return true;
    },
    resume
  };
}
