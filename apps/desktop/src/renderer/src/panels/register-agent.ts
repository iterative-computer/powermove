import AgentPanel from './AgentPanel.svelte';
import {
  agentState,
  setAgentComposerDraft,
  setAgentSnapshot,
  type AgentSnapshot,
  type AgentUpdateOptions
} from './agent/agent-state.svelte';
import { registerSveltePanel } from './registerSveltePanel';

export interface AgentLegacyBridge {
  newThread?(): void;
  switchThread?(id: string): void;
  deleteThread?(id: string): void;
  flushThreads?(): void;
  snapshot(): AgentSnapshot;
  submit(value: string): void;
  stop(): void;
  setDraft(value: string): void;
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
    importArtifact: bridge.importArtifact,
    revealArtifact: bridge.revealArtifact,
    undoPanelRun: bridge.undoPanelRun,
    keepPanelRun: bridge.keepPanelRun,
    undoSceneRun: bridge.undoSceneRun,
    keepSceneRun: bridge.keepSceneRun
  };

  registerSveltePanel(PM, 'agent', {
    title: 'Powermove agent',
    size: 350,
    min: 240,
    noscroll: true,
    component: AgentPanel
  });
  PM.AgentUI.update();
}
