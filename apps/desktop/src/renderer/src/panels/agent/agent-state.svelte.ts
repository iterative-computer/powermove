import type { NoticeKind } from '../../errors/presentation';
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
  /** A user follow-up or trace checkpoint captured while a run was steered. */
  steering?: boolean;
  /** role 'trace': sealed activity steps from a run or steering checkpoint. */
  steps?: TraceStep[];
  /** Elapsed wall time captured when the run's activity is archived. */
  durationMs?: number;
  attachments?: Array<Record<string, any> | string>;
  entering?: boolean;
  /** Run failures render with the error treatment. */
  error?: boolean;
  /** How much apparatus the failure earns: the editor's own error keeps the
      diagnostics, the agent's account of what it could not do is an alert, and
      a turn where nothing broke reads as an ordinary reply. Absent means
      'error', so older messages keep their treatment. */
  notice?: NoticeKind;
  /** Editor mode could not author the requested extension. */
  requiresProject?: boolean;
  fixExtensionId?: string;
  modResult?: AgentModResult;
  focusLabels?: string[];
}

export interface AgentStep {
  id: string;
  title: string;
  status: 'pending' | 'active' | 'complete' | 'error';
}

/* Frozen 2026-09-13 (agent-stream-ux). Mirrors ./activity-rows TraceStep. */
export type TraceStep =
  | { kind: 'thought'; id: string; label: string; live: boolean; startedAt?: number; endedAt?: number }
  | { kind: 'text'; id: string; text: string }
  | {
      kind: 'tool';
      id: string;
      toolName: string;
      label: string;
      status: 'running' | 'done' | 'error' | 'continued';
      /** Primary argument in mono (command, path, query). */
      detail?: string;
      /** Bounded result excerpt, set on tool-end. */
      output?: string;
      startedAt?: number;
      endedAt?: number;
    };

export interface AgentSnapshot {
  threadId?: string;
  threads?: Array<{ id: string; title: string; updatedAt?: number; busy?: boolean }>;
  threadSwitchBlocked?: boolean;
  /** Runs still working in threads other than the one on screen. */
  backgroundRuns?: number;
  threadSaveError?: boolean;
  legacyPhase: string;
  requestToken: number;
  /** When the visible thread's run actually began, across thread switches. */
  runStartedAt?: number | null;
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
  workingStartedAt: number | null;
  workingConversationIndex: number | null;
  focusVersion: number;
  revision: number;
}

const EMPTY_SNAPSHOT: AgentSnapshot = {
  threadId: '',
  threads: [],
  threadSwitchBlocked: false,
  backgroundRuns: 0,
  threadSaveError: false,
  legacyPhase: 'idle',
  requestToken: 0,
  runStartedAt: null,
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
  workingStartedAt: null,
  workingConversationIndex: null,
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

function jsonEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** Attachment bodies can be megabytes; compare immutable fields without encoding them. */
function attachmentsEqual(left: Array<Record<string, any> | string> | undefined, right: Array<Record<string, any> | string> | undefined): boolean {
  if (left === right) return true;
  if (!left || !right || left.length !== right.length) return false;
  return left.every((item, index) => {
    const other = right[index];
    if (item === other) return true;
    if (typeof item !== 'object' || !item || typeof other !== 'object' || !other) return false;
    const keys = Object.keys(item);
    return keys.length === Object.keys(other).length && keys.every(key => item[key] === other[key]);
  });
}

function assignChangedFields(
  current: Record<string, any>,
  next: Record<string, any>,
  fields: readonly string[]
): void {
  for (const field of fields) {
    if (current[field] !== next[field]) current[field] = next[field];
  }
}

function reconcileTrace(next: TraceStep[]): void {
  for (let index = 0; index < next.length; index += 1) {
    const nextStep = next[index]!;
    const currentStep = agentState.trace[index];
    if (!currentStep || currentStep.kind !== nextStep.kind || currentStep.id !== nextStep.id) {
      agentState.trace[index] = { ...nextStep };
      continue;
    }
    if (nextStep.kind === 'thought') {
      assignChangedFields(currentStep, nextStep, ['label', 'live', 'startedAt', 'endedAt']);
    } else if (nextStep.kind === 'text') {
      assignChangedFields(currentStep, nextStep, ['text']);
    } else {
      assignChangedFields(currentStep, nextStep, [
        'toolName', 'label', 'status', 'detail', 'output', 'startedAt', 'endedAt'
      ]);
    }
  }
  if (agentState.trace.length !== next.length) agentState.trace.length = next.length;
}

function reconcileSteps(next: AgentStep[]): void {
  for (let index = 0; index < next.length; index += 1) {
    const nextStep = next[index]!;
    const currentStep = agentState.steps[index];
    if (!currentStep || currentStep.id !== nextStep.id) {
      agentState.steps[index] = { ...nextStep };
      continue;
    }
    assignChangedFields(currentStep, nextStep, ['title', 'status']);
  }
  if (agentState.steps.length !== next.length) agentState.steps.length = next.length;
}

function cloneMessage(message: AgentMessage): AgentMessage {
  return {
    ...message,
    modResult: message.modResult ? { ...message.modResult } : undefined,
    steps: message.steps?.map((step) => ({ ...step })),
    focusLabels: message.focusLabels ? [...message.focusLabels] : undefined,
    attachments: message.attachments?.map((attachment) =>
      typeof attachment === 'string' ? attachment : { ...attachment }
    )
  };
}

function reconcileConversation(next: AgentMessage[]): void {
  for (let index = 0; index < next.length; index += 1) {
    const nextMessage = next[index]!;
    const currentMessage = agentState.conversation[index];
    if (!currentMessage || currentMessage.role !== nextMessage.role) {
      agentState.conversation[index] = cloneMessage(nextMessage);
      continue;
    }
    assignChangedFields(currentMessage, nextMessage, [
      'text', 'steering', 'entering', 'error', 'notice', 'fixExtensionId', 'requiresProject', 'durationMs'
    ]);
    if (!jsonEqual(currentMessage.modResult, nextMessage.modResult)) {
      currentMessage.modResult = nextMessage.modResult ? { ...nextMessage.modResult } : undefined;
    }
    if (!jsonEqual(currentMessage.steps, nextMessage.steps)) {
      currentMessage.steps = nextMessage.steps?.map((step) => ({ ...step }));
    }
    if (!attachmentsEqual(currentMessage.attachments, nextMessage.attachments)) {
      currentMessage.attachments = nextMessage.attachments?.map((attachment) =>
        typeof attachment === 'string' ? attachment : { ...attachment }
      );
    }
    if (!jsonEqual(currentMessage.focusLabels, nextMessage.focusLabels)) {
      currentMessage.focusLabels = nextMessage.focusLabels ? [...nextMessage.focusLabels] : undefined;
    }
  }
  if (agentState.conversation.length !== next.length) agentState.conversation.length = next.length;
}

export function setAgentSnapshot(snapshot: AgentSnapshot, options: AgentUpdateOptions = {}): void {
  const phase = viewPhase(snapshot);
  const tokenChanged = snapshot.requestToken !== agentState.requestToken;
  const enteringRun = phase === 'running' && agentState.phase !== 'running';
  const threadChanged = (snapshot.threadId || '') !== agentState.threadId;
  let progressLines = tokenChanged || enteringRun || threadChanged ? [] : [...agentState.progressLines];
  if (phase === 'running' && snapshot.activity && progressLines.at(-1) !== snapshot.activity) {
    progressLines = [...progressLines, snapshot.activity].slice(-20);
  }
  if ((phase === 'idle' || phase === 'prompt') && !snapshot.activity) progressLines = [];

  const {
    conversation,
    trace,
    steps,
    models,
    providers,
    reasoningEfforts,
    accessModes,
    threads = [],
    threadId,
    threadSwitchBlocked,
    backgroundRuns,
    threadSaveError,
    attachments,
    uiPlacement,
    plan,
    run,
    panelRun,
    ...scalarSnapshot
  } = snapshot;

  reconcileConversation(conversation);
  reconcileTrace(trace);
  reconcileSteps(steps);
  if (!jsonEqual(agentState.models, models)) agentState.models = models.map((model) => ({ ...model }));
  if (!jsonEqual(agentState.providers, providers)) agentState.providers = providers.map((provider) => ({ ...provider }));
  if (!jsonEqual(agentState.reasoningEfforts, reasoningEfforts)) agentState.reasoningEfforts = [...reasoningEfforts];
  if (!jsonEqual(agentState.accessModes, accessModes)) agentState.accessModes = accessModes.map((mode) => ({ ...mode }));
  if (!jsonEqual(agentState.threads, threads)) agentState.threads = threads.map((thread) => ({ ...thread }));
  if (!attachmentsEqual(agentState.attachments, attachments)) {
    agentState.attachments = attachments.map((attachment) => ({ ...attachment }));
  }

  Object.assign(agentState, scalarSnapshot, {
    threadId: threadId || '',
    threadSwitchBlocked: threadSwitchBlocked || false,
    backgroundRuns: backgroundRuns || 0,
    threadSaveError: threadSaveError || false,
    phase,
    workingStartedAt: phase === 'running'
      ? (enteringRun || tokenChanged || threadChanged
        ? (snapshot.runStartedAt ?? Date.now()) : agentState.workingStartedAt)
      : null,
    workingConversationIndex: phase === 'running'
      ? (enteringRun || tokenChanged || threadChanged
        ? conversation.length : agentState.workingConversationIndex)
      : null,
    uiPlacement: phase === 'running' && uiPlacement ? { ...uiPlacement } : null,
    plan: plan ? { ...plan } : null,
    run: run ? {
      ...run,
      artifacts: run.artifacts?.map((artifact: Record<string, any>) => ({ ...artifact })),
      externalActions: run.externalActions ? [...run.externalActions] : run.externalActions,
      frames: run.frames ? {
        ...run.frames,
        images: run.frames.images ? [...run.frames.images] : run.frames.images,
        times: run.frames.times ? [...run.frames.times] : run.frames.times
      } : run.frames
    } : null,
    panelRun: panelRun ? {
      ...panelRun,
      actions: panelRun.actions?.map((action: Record<string, any>) => ({ ...action }))
    } : null,
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
    workingStartedAt: null,
    workingConversationIndex: null,
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
