import type { AgentModResult } from './mod-result';
import type { UIPlacement } from './ui-placement';

export type AgentPhase = 'idle' | 'prompt' | 'running' | 'preview' | 'result';

export interface AgentOption {
  id: string;
  label: string;
  detail?: string;
}

export interface AgentMessage {
  role: 'user' | 'assistant' | 'trace';
  text?: string;
  /** A user follow-up sent while the current run was still active. */
  steering?: boolean;
  /** role 'trace': the sealed activity steps of a completed run. */
  steps?: TraceStep[];
  attachments?: Array<Record<string, any> | string>;
  entering?: boolean;
  /** Run failures render with the error treatment. */
  error?: boolean;
  fixExtensionId?: string;
  modResult?: AgentModResult;
  focusLabels?: string[];
}

export interface AgentStep {
  id: string;
  title: string;
  status: 'pending' | 'active' | 'complete' | 'error';
}

export type TraceStep =
  | { kind: 'thought'; id: string; label: string; live: boolean }
  | { kind: 'text'; id: string; text: string }
  | { kind: 'tool'; id: string; toolName: string; label: string; status: 'running' | 'done' | 'error' | 'continued' };

export interface AgentSnapshot {
  threadId?: string;
  threads?: Array<{ id: string; title: string; updatedAt?: number }>;
  threadSwitchBlocked?: boolean;
  threadSaveError?: boolean;
  legacyPhase: string;
  requestToken: number;
  conversation: AgentMessage[];
  activity: string;
  uiPlacement?: UIPlacement | null;
  trace: TraceStep[];
  plan: Record<string, any> | null;
  run: Record<string, any> | null;
  panelRun: Record<string, any> | null;
  attachments: Array<Record<string, any>>;
  steps: AgentStep[];
  stepsExpanded: boolean;
  scope: string;
  autoApplyPanels: boolean;
  provider: string;
  model: string;
  reasoningEffort: string;
  accessMode: string;
  composerDraft: string;
  pendingEntering: boolean;
  models: AgentOption[];
  providers: AgentOption[];
  reasoningEfforts: string[];
  accessModes: AgentOption[];
}

export interface AgentUpdateOptions {
  focusComposer?: boolean;
  flush?: boolean;
}

interface AgentViewState extends AgentSnapshot {
  phase: AgentPhase;
  progressLines: string[];
  focusVersion: number;
  revision: number;
}

const EMPTY_SNAPSHOT: AgentSnapshot = {
  threadId: '',
  threads: [],
  threadSwitchBlocked: false,
  threadSaveError: false,
  legacyPhase: 'idle',
  requestToken: 0,
  conversation: [],
  activity: '',
  uiPlacement: null,
  trace: [],
  plan: null,
  run: null,
  panelRun: null,
  attachments: [],
  steps: [],
  stepsExpanded: false,
  scope: 'workspace',
  autoApplyPanels: true,
  provider: 'chatgpt',
  model: 'gpt-5.6-sol',
  reasoningEffort: 'high',
  accessMode: 'editor',
  composerDraft: '',
  pendingEntering: false,
  models: [],
  providers: [],
  reasoningEfforts: [],
  accessModes: []
};

export const agentState: AgentViewState = $state({
  ...EMPTY_SNAPSHOT,
  phase: 'idle',
  progressLines: [],
  focusVersion: 0,
  revision: 0
});

function viewPhase(snapshot: AgentSnapshot): AgentPhase {
  if (snapshot.legacyPhase === 'result' || snapshot.panelRun) return 'result';
  if (snapshot.legacyPhase === 'working' || snapshot.legacyPhase === 'applying') return 'running';
  if (snapshot.plan) return 'preview';
  if (snapshot.legacyPhase === 'idle') return 'idle';
  return 'prompt';
}

export function setAgentSnapshot(snapshot: AgentSnapshot, options: AgentUpdateOptions = {}): void {
  const phase = viewPhase(snapshot);
  const tokenChanged = snapshot.requestToken !== agentState.requestToken;
  const enteringRun = phase === 'running' && agentState.phase !== 'running';
  let progressLines = tokenChanged || enteringRun ? [] : [...agentState.progressLines];
  if (phase === 'running' && snapshot.activity && progressLines.at(-1) !== snapshot.activity) {
    progressLines = [...progressLines, snapshot.activity].slice(-20);
  }
  if ((phase === 'idle' || phase === 'prompt') && !snapshot.activity) progressLines = [];

  Object.assign(agentState, snapshot, {
    threadId: snapshot.threadId || '',
    threads: snapshot.threads?.map(thread => ({ ...thread })) || [],
    threadSwitchBlocked: snapshot.threadSwitchBlocked || false,
    threadSaveError: snapshot.threadSaveError || false,
    phase,
    uiPlacement: phase === 'running' && snapshot.uiPlacement ? { ...snapshot.uiPlacement } : null,
    plan: snapshot.plan ? { ...snapshot.plan } : null,
    run: snapshot.run ? {
      ...snapshot.run,
      artifacts: snapshot.run.artifacts?.map((artifact: Record<string, any>) => ({ ...artifact })),
      externalActions: snapshot.run.externalActions ? [...snapshot.run.externalActions] : snapshot.run.externalActions,
      frames: snapshot.run.frames ? {
        ...snapshot.run.frames,
        images: snapshot.run.frames.images ? [...snapshot.run.frames.images] : snapshot.run.frames.images,
        times: snapshot.run.frames.times ? [...snapshot.run.frames.times] : snapshot.run.frames.times
      } : snapshot.run.frames
    } : null,
    panelRun: snapshot.panelRun ? {
      ...snapshot.panelRun,
      actions: snapshot.panelRun.actions?.map((action: Record<string, any>) => ({ ...action }))
    } : null,
    conversation: snapshot.conversation.map((message) => ({
      ...message,
      modResult: message.modResult ? { ...message.modResult } : undefined,
      focusLabels: message.focusLabels ? [...message.focusLabels] : undefined,
      attachments: message.attachments?.map((attachment) =>
        typeof attachment === 'string' ? attachment : { ...attachment })
    })),
    attachments: snapshot.attachments.map((attachment) => ({ ...attachment })),
    trace: snapshot.trace.map((step) => ({ ...step })),
    steps: snapshot.steps.map((step) => ({ ...step })),
    models: snapshot.models.map((model) => ({ ...model })),
    providers: snapshot.providers.map((provider) => ({ ...provider })),
    reasoningEfforts: [...snapshot.reasoningEfforts],
    accessModes: snapshot.accessModes.map((mode) => ({ ...mode })),
    progressLines,
    focusVersion: agentState.focusVersion + (options.focusComposer ? 1 : 0),
    revision: agentState.revision + 1
  });
}

export function setAgentComposerDraft(value: string): void {
  agentState.composerDraft = value;
}

export function resetAgentState(): void {
  Object.assign(agentState, EMPTY_SNAPSHOT, {
    phase: 'idle',
    conversation: [],
    attachments: [],
    trace: [],
    steps: [],
    models: [],
    providers: [],
    reasoningEfforts: [],
    accessModes: [],
    progressLines: [],
    focusVersion: 0,
    revision: 0
  });
}

export function composerMode(legacyPhase: string): {
  working: boolean;
  disabled: boolean;
  placeholder: string;
  sendLabel: string;
} {
  const working = legacyPhase === 'working';
  return {
    working,
    disabled: legacyPhase === 'applying',
    placeholder: working ? 'Add direction…' : 'Describe a change…',
    sendLabel: working ? 'Steer current run' : 'Send message'
  };
}

export function describePanelAction(PM: Record<string, any>, action: Record<string, any>): string {
  const title = PM.PANELS[action.panelId]?.title || action.panelId || action.dockId || 'Dock';
  if (action.type === 'add') return `Open ${title}${action.dockId ? ` in ${action.dockId}` : ''}`;
  if (action.type === 'restore') return `Restore ${title}`;
  if (action.type === 'hide') return `Hide ${title}`;
  if (action.type === 'move') return `Move ${title} to ${action.dockId}${Number.isInteger(action.position) ? ` · position ${action.position + 1}` : ''}`;
  if (action.type === 'reorder') return `Place ${title} at position ${(action.position ?? 0) + 1}`;
  if (action.type === 'resize') return `Resize ${title} to ${Math.round(action.size || 0)} px`;
  if (action.type === 'resizeDock') return `Resize ${action.dockId} dock to ${Math.round(action.size || 0)} px`;
  if (action.type === 'rename') return `Rename ${title} to ${action.title}`;
  if (action.type === 'collapse') return `Collapse ${title}`;
  if (action.type === 'expand') return `Expand ${title}`;
  return `${action.type} ${title}`;
}
