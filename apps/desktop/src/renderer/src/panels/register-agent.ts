import AgentPanel from './AgentPanel.svelte';
import {
  agentState,
  setAgentComposerDraft,
  setAgentSnapshot,
  type AgentSnapshot,
  type AgentUpdateOptions
} from './agent/agent-state.svelte';
import { registerSveltePanel } from './registerSveltePanel';
import { installAgentShell } from './agent-shell';

export interface AgentLegacyBridge {
  openGlobal?(): void;
  importWorkspace?(appId: 'after-effects'): Promise<boolean>;
  resumeWorkspaceImport?(): void;
  repairExtension?(request: { id: string; name?: string; diagnostics?: string[] }): Promise<boolean>;
  newThread?(): void;
  switchThread?(id: string): void;
  deleteThread?(id: string): void;
  flushThreads?(): void;
  snapshot(): AgentSnapshot;
  submit(value: string): void;
  stop(): void;
  setDraft(value: string): void;
  setInlineDraft?(value: string, attachments: any[]): void;
  retry?(messageIndex?: number): void;
  continueWithProject?(messageIndex: number): void;
  setStepsExpanded(expanded: boolean): void;
  setModel(model: string, effort: string): void;
  setProvider?(provider: string): void;
  setAccess(mode: string): void;
  confirmComputerAccess(): void;
  setScope(scope: string): void;
  toggleAutoApplyPanels(): void;
  dismissPlan(): void;
  applyPlan(): void;
  addAttachments(files: File[]): Promise<void>;
  removeAttachment(id: string): void;
  answerQuestion?(id: string, answers: Record<string, string[]>): void;
  importArtifact(artifact: Record<string, any>): Promise<void>;
  revealArtifact(artifact: Record<string, any>): void;
  undoPanelRun(): void;
  keepPanelRun(): void;
  undoSceneRun(): void;
  keepSceneRun(): void;
}

type LegacyPM = Record<string, any>;

export function registerAgentPanel(PM: LegacyPM, bridge: AgentLegacyBridge): void {
  const updateInterval = 16;
  let updateTimer: ReturnType<typeof setTimeout> | null = null;
  let lastUpdateAt: number | null = null;
  let queuedOptions: AgentUpdateOptions = {};

  const commitUpdate = (options: AgentUpdateOptions): void => {
    lastUpdateAt = Date.now();
    setAgentSnapshot(bridge.snapshot(), options);
  };

  PM.AgentUI = {
    openGlobal: bridge.openGlobal,
    importWorkspace: bridge.importWorkspace,
    resumeWorkspaceImport: bridge.resumeWorkspaceImport,
    repairExtension: bridge.repairExtension,
    newThread: bridge.newThread,
    switchThread: bridge.switchThread,
    deleteThread: bridge.deleteThread,
    flushThreads: bridge.flushThreads,
    state: agentState,
    update(options: AgentUpdateOptions = {}) {
      const now = Date.now();
      const mustFlush = options.flush || options.focusComposer;
      const shouldCoalesce = agentState.phase === 'running' && !mustFlush;
      if (!shouldCoalesce || lastUpdateAt === null || now - lastUpdateAt >= updateInterval) {
        if (updateTimer !== null) clearTimeout(updateTimer);
        updateTimer = null;
        const merged = { ...queuedOptions, ...options };
        queuedOptions = {};
        commitUpdate(merged);
        return;
      }

      queuedOptions = { ...queuedOptions, ...options };
      if (updateTimer !== null) return;
      updateTimer = setTimeout(() => {
        updateTimer = null;
        const pending = queuedOptions;
        queuedOptions = {};
        commitUpdate(pending);
      }, Math.max(0, updateInterval - (now - lastUpdateAt)));
    },
    submit: bridge.submit,
    stop: bridge.stop,
    retry: bridge.retry,
    continueWithProject: bridge.continueWithProject,
    setDraft(value: string, focus = false) {
      bridge.setDraft(value);
      setAgentComposerDraft(value);
      if (focus) setAgentSnapshot(bridge.snapshot(), { focusComposer: true });
    },
    setInlineDraft(value: string, attachments: any[]) {
      if (bridge.setInlineDraft) bridge.setInlineDraft(value, attachments);
      else bridge.setDraft(value);
      setAgentComposerDraft(value);
      agentState.attachments = attachments;
    },
    setStepsExpanded: bridge.setStepsExpanded,
    setModel: bridge.setModel,
    setProvider: bridge.setProvider,
    setAccess: bridge.setAccess,
    confirmComputerAccess: bridge.confirmComputerAccess,
    setScope: bridge.setScope,
    toggleAutoApplyPanels: bridge.toggleAutoApplyPanels,
    dismissPlan: bridge.dismissPlan,
    applyPlan: bridge.applyPlan,
    addAttachments: bridge.addAttachments,
    removeAttachment: bridge.removeAttachment,
    answerQuestion: bridge.answerQuestion,
    importArtifact: bridge.importArtifact,
    revealArtifact: bridge.revealArtifact,
    undoPanelRun: bridge.undoPanelRun,
    keepPanelRun: bridge.keepPanelRun,
    undoSceneRun: bridge.undoSceneRun,
    keepSceneRun: bridge.keepSceneRun
  };

  /* Opens from the titlebar launcher as a popover; dragging the launcher onto
     the workspace docks it like any panel (closing it returns to the popover). */
  registerSveltePanel(PM, 'agent', {
    title: 'Powermove agent',
    size: 350,
    min: 240,
    noscroll: true,
    library: false,
    component: AgentPanel
  });
  if (typeof document !== 'undefined' && document.body) installAgentShell(PM);
  PM.AgentUI.update();
}
