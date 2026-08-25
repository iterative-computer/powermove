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
  snapshot(): AgentSnapshot;
  submit(value: string): void;
  stop(): void;
  setDraft(value: string): void;
  setStepsExpanded(expanded: boolean): void;
  setModel(model: string, effort: string): void;
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
  PM.AgentUI = {
    state: agentState,
    update(options: AgentUpdateOptions = {}) {
      setAgentSnapshot(bridge.snapshot(), options);
    },
    submit: bridge.submit,
    stop: bridge.stop,
    setDraft(value: string, focus = false) {
      bridge.setDraft(value);
      setAgentComposerDraft(value);
      if (focus) setAgentSnapshot(bridge.snapshot(), { focusComposer: true });
    },
    setStepsExpanded: bridge.setStepsExpanded,
    setModel: bridge.setModel,
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
