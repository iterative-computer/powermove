/* Ported from js/assistant/spatial.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import type { CodexTraceEvent } from '../../../../shared/ipc';
import { addPanel, findPanel, hidePanel, movePanel, restorePanel } from '../../layout/model';
import { composerMode, type AgentSnapshot } from '../../panels/agent/agent-state.svelte';
import { registerAgentPanel } from '../../panels/register-agent';
import { isUIPlacementMessage, parseUIPlacement, uiPlacementInstructions } from '../../panels/agent/ui-placement';
import { flushSync, mount, unmount } from 'svelte';
import AgentOptions from '../../panels/agent/AgentOptions.svelte';
import { mountPromptAttachments, readPromptAttachment, requestFileAttachments } from '../../panels/agent/attachments';
import { intersectingPanels, NATIVE_PANEL_DESIGN, panelFocusContext, panelFocusPrompt, panelScope, type PanelFocusContext } from '../../panels/agent/panel-focus';
import { AgentThreads, threadTitle } from '../../panels/agent/threads';
import { AGENT_TESTING_INSTRUCTIONS } from '../../../../shared/agent-testing';
import RippleCanvas from './RippleCanvas.svelte';

export function install(PM: PMRegistry): void {
const h: any = PM.h;

/* The native Codex client owns ChatGPT authentication. This bridge only moves a
   prompt and a strict JSON schema across WKWebView; no account secret enters JS. */
const pending: any = new Map();
function codexAbortError(message: any = 'The previous agent run was replaced') {
  const error: any = new Error(message); error.name = 'AbortError'; return error;
}
PM.CodexBridge = {
  request(prompt: any, schema: any, images: any = [], options: any = {}) {
    const bridge: any = (window as any).webkit?.messageHandlers?.pmCodex;
    if (!bridge) return Promise.reject(new Error('The coding agent is available in the Powermove macOS app'));
    const id: any = PM.uid('spatial-codex-');
    return new Promise((resolve: any, reject: any) => {
      const signal: any = options.signal;
      const stopNative: any = () => (window as any).webkit?.messageHandlers?.pmCodexCancel?.postMessage({ id });
      const settle: any = (error: any) => {
        const job: any = pending.get(id); if (!job) return;
        pending.delete(id); window.clearTimeout(job.timer);
        job.signal?.removeEventListener('abort', job.abort);
        stopNative(); reject(error);
      };
      const abort: any = () => settle(codexAbortError());
      const timeout: any = Math.max(30_000, Math.min(Number(options.timeoutMs) || 120_000, 3_600_000));
      const timer: any = window.setTimeout(() => settle(new Error('The coding agent took too long to respond')), timeout);
      pending.set(id, { resolve, reject, timer, signal, abort, onProgress: options.onProgress, onTrace: options.onTrace, mode: options.mode });
      if (signal?.aborted) { abort(); return; }
      signal?.addEventListener('abort', abort, { once: true });
      bridge.postMessage({
        id, prompt, schema, images: images.slice(0, 6),
        provider: options.provider || 'chatgpt',
        threadId: options.threadId || '',
        model: options.model || '', reasoningEffort: options.reasoningEffort || '',
        mode: options.mode || 'editor', access: options.access || 'editor',
        projectId: options.projectId || '', projectName: options.projectName || '',
        projectJSON: options.projectJSON || '', attachments: (options.attachments || []).slice(0, 6),
      });
    });
  },
  resolve(id: any, result: any) {
    const job: any = pending.get(id); if (!job) return;
    pending.delete(id); window.clearTimeout(job.timer);
    job.signal?.removeEventListener('abort', job.abort);
    let text: any = '';
    try {
      const bytes: any = Uint8Array.from(window.atob(result.dataBase64 || ''), (c: any) => c.charCodeAt(0));
      text = new window.TextDecoder().decode(bytes);
    } catch { text = 'The coding-agent response could not be decoded'; }
    if (result.ok) {
      job.resolve(job.mode === 'autonomous' ? {
        text,
        extensions: result.extensions,
        extensionChangeSetId: result.extensionChangeSetId
      } : text);
    } else job.reject(new Error(text));
  },
  progress(id: any, result: any) {
    const job: any = pending.get(id); if (!job || typeof job.onProgress !== 'function') return;
    try {
      const bytes: any = Uint8Array.from(window.atob(result.dataBase64 || ''), (c: any) => c.charCodeAt(0));
      const summary: any = new window.TextDecoder().decode(bytes).replace(/\s+/g, ' ').trim().slice(0, 320);
      if (summary) job.onProgress(summary);
    } catch { /* Ignore malformed progress without interrupting the real run. */ }
  },
  trace(id: any, step: any) {
    const job: any = pending.get(id); if (!job || typeof job.onTrace !== 'function') return;
    job.onTrace(step);
  },
};

const artifactPending: any = new Map();
PM.AgentArtifacts = {
  load(artifact: any) {
    const bridge: any = (window as any).webkit?.messageHandlers?.pmAgentArtifact;
    if (!bridge) return Promise.reject(new Error('Agent artifacts are available in the Powermove macOS app'));
    const id: any = PM.uid('agent-artifact-');
    return new Promise((resolve: any, reject: any) => {
      const timer: any = window.setTimeout(() => { artifactPending.delete(id); reject(new Error('The artifact took too long to load')); }, 120_000);
      artifactPending.set(id, { resolve, reject, timer });
      bridge.postMessage({ id, projectId: artifact.projectId, path: artifact.path });
    });
  },
  resolve(id: any, result: any) {
    const job: any = artifactPending.get(id); if (!job) return;
    artifactPending.delete(id); window.clearTimeout(job.timer);
    if (!result?.ok) { job.reject(new Error(result?.message || 'The artifact could not be loaded')); return; }
    try {
      const bytes: any = Uint8Array.from(window.atob(result.dataBase64 || ''), (character: any) => character.charCodeAt(0));
      job.resolve(new window.File([bytes], result.name || 'agent-artifact', { type: result.mime || 'application/octet-stream' }));
    } catch { job.reject(new Error('The artifact could not be decoded')); }
  },
  reveal(artifact: any) {
    const bridge: any = (window as any).webkit?.messageHandlers?.pmAgentReveal;
    if (!bridge) return PM.toast('Agent artifacts are available in the Powermove macOS app');
    bridge.postMessage({ projectId: artifact.projectId, path: artifact.path || '' });
  },
};

/* WebGPU can only displace pixels it can sample. The native shell captures the
   WKWebView before the overlay mounts, then this bridge decodes that frame into
   an ImageBitmap suitable for copyExternalImageToTexture(). */
const pendingCaptures: any = new Map();
PM.WindowCapture = {
  request() {
    const bridge: any = (window as any).webkit?.messageHandlers?.pmCaptureWindow;
    if (!bridge) return Promise.resolve(null);
    const id: any = PM.uid('window-capture-');
    return new Promise((resolve: any) => {
      const timer: any = window.setTimeout(() => { pendingCaptures.delete(id); resolve(null); }, 700);
      pendingCaptures.set(id, { resolve, timer });
      bridge.postMessage({ id });
    });
  },
  async resolve(id: any, result: any) {
    const job: any = pendingCaptures.get(id); if (!job) return;
    pendingCaptures.delete(id); window.clearTimeout(job.timer);
    if (!result?.ok || !result.dataBase64) { job.resolve(null); return; }
    try {
      const bytes: any = Uint8Array.from(window.atob(result.dataBase64), (c: any) => c.charCodeAt(0));
      job.resolve(await window.createImageBitmap(new window.Blob([bytes], { type: 'image/png' })));
    } catch (error: any) { window.console.warn('Could not decode the interface snapshot', error); job.resolve(null); }
  },
};

/* Broad access is the default: the agent can use project tools and the web
   without a picker. Computer access still requires its per-run confirmation. */
const storedAccessMode: any = PM.store?.get?.('agentAccessMode', 'project');
const storedProvider: any = PM.store?.get?.('agentProvider', 'chatgpt');
const initialProvider: any = ['chatgpt', 'claude'].includes(storedProvider) ? storedProvider : 'chatgpt';
const S: any = {
  initialized: false, active: false, pressed: false, phase: 'idle',
  samples: [], points: [], lastTrigger: 0, origin: { x: 0, y: 0 },
  root: null, ink: null, path: null, shadePath: null, hint: null, card: null, outline: null,
  region: null, context: null, plan: null, renderStop: null, requestToken: 0,
  requestText: '', run: null, conversation: [], activity: '', trace: [], activeRequest: null, composerDraft: '',
  uiPlacement: null,
  focusPicker: null,
  rippleWarmup: null, sceneCache: null, sceneCacheAt: 0, cachePending: null,
  sceneFrame: null, regionImage: null,
  hintFrame: 0, hintPoint: null,
  attachments: [], requestAttachments: [], steps: [], stepsExpanded: false,
  pendingEntering: false,
  panelRun: null, scope: PM.store?.get?.('agentScope', 'workspace') || 'workspace',
  autoApplyPanels: PM.store?.get?.('agentAutoApplyPanels', true) !== false,
  provider: initialProvider,
  model: PM.store?.get?.(`agentModel.${initialProvider}`, initialProvider === 'claude' ? 'sonnet' : 'gpt-5.6-sol') || (initialProvider === 'claude' ? 'sonnet' : 'gpt-5.6-sol'),
  reasoningEffort: PM.store?.get?.('agentReasoningEffort', 'high') || 'high',
  accessMode: ['editor', 'project'].includes(storedAccessMode) ? storedAccessMode : 'project',
};

const AGENT_PROVIDERS: any = [
  { id: 'chatgpt', label: 'ChatGPT' },
  { id: 'claude', label: 'Claude' },
];
const AGENT_MODELS: any = {
  chatgpt: [
    { id: 'gpt-5.6-sol', label: '5.6 Sol' },
    { id: 'gpt-5.6-terra', label: '5.6 Terra' },
    { id: 'gpt-5.6-luna', label: '5.6 Luna' },
  ],
  claude: [
    { id: 'sonnet', label: 'Sonnet' },
    { id: 'opus', label: 'Opus' },
    { id: 'fable', label: 'Fable' },
  ],
};
const REASONING_EFFORTS: any = ['low', 'medium', 'high', 'xhigh', 'max'];
const AGENT_ACCESS_MODES: any = [
  { id: 'editor', label: 'Edit project', detail: 'Edit the current composition' },
  { id: 'project', label: 'Change Powermove (project)', detail: 'Create or edit mods with files, shell, web, and integrations' },
  { id: 'computer', label: 'Computer', detail: 'Full Mac access for one explicitly approved run' },
];
const SEND_TRANSITION_MS: any = 240;

const threads = new AgentThreads({
  get: (key, fallback) => PM.store?.get?.(key, fallback) ?? fallback,
  set: (key, value) => PM.store?.set?.(key, value),
}, () => PM.uid('thread-'));
threads.load(PM.proj?.id || '');
let threadSaveError = false;
let threadSaveTimer: ReturnType<typeof setTimeout> | undefined;
let changingThreadProject = false;
let lastThreadWrite = '';
const threadResults = new Map<string, any>();

function captureThread() {
  const thread = threads.active;
  Object.assign(thread, {
    conversation: S.conversation, composerDraft: S.composerDraft,
    attachments: S.attachments, scope: S.scope, title: threadTitle(S.conversation),
  });
}

function persistThreads() {
  clearTimeout(threadSaveTimer); threadSaveTimer = undefined;
  captureThread();
  const serialized = JSON.stringify([threads.projectId, threads.activeId, threads.threads]);
  if (serialized === lastThreadWrite) return;
  threadSaveError = !threads.save();
  if (!threadSaveError) lastThreadWrite = serialized;
}

function restoreThread() {
  const thread = threads.active;
  const saved = threadResults.get(`${threads.projectId}/${thread.id}`);
  // Checkpoints and unapplied plans are only safe while the document is unchanged.
  const result = saved?.revision === Number(PM.proj?.revision || 0) ? saved : null;
  Object.assign(S, {
    conversation: thread.conversation, composerDraft: thread.composerDraft, attachments: thread.attachments,
    scope: thread.scope, phase: result?.phase || (thread.conversation.length ? 'conversation' : 'idle'),
    plan: result?.plan || null, run: result?.run || null, panelRun: result?.panelRun || null,
    steps: result?.steps || [], trace: [], activity: '', uiPlacement: null,
    context: null, region: null, regionImage: null, requestAttachments: [], requestText: '',
    stepsExpanded: false, pendingEntering: false,
  });
}

function ensureThreadProject(): boolean {
  const projectId = PM.proj?.id || '';
  if (changingThreadProject || projectId === threads.projectId) return false;
  changingThreadProject = true;
  if (S.activeRequest || S.phase === 'working') stopActiveRequest();
  persistThreads();
  ++S.requestToken;
  threadResults.clear();
  threads.load(projectId);
  restoreThread();
  changingThreadProject = false;
  return true;
}

function switchThread(id?: string) {
  ensureThreadProject();
  if (S.activeRequest || S.phase === 'working' || S.phase === 'applying') {
    PM.toast?.('Finish or stop the current run before switching threads.'); return;
  }
  if (id === threads.activeId || (id && !threads.threads.some(t => t.id === id))) return;
  persistThreads();
  threadResults.set(`${threads.projectId}/${threads.activeId}`, {
    revision: Number(PM.proj?.revision || 0), phase: S.phase,
    plan: S.plan, run: S.run, panelRun: S.panelRun, steps: S.steps,
  });
  // Close only the spatial prompt; the editor window and project stay intact.
  if (S.active) dismissOverlay(true);
  ++S.requestToken;
  if (id) threads.select(id); else threads.create();
  restoreThread();
  persistThreads();
  PM.AgentUI?.update({ flush: true, focusComposer: true });
}

restoreThread();
PM.bus?.on?.('project', () => { ensureThreadProject(); PM.AgentUI?.update({ flush: true }); });
PM.bus?.on?.('storage:error', (event: any) => {
  if (event?.key === threads.key) { threadSaveError = true; PM.AgentUI?.update({ flush: true }); }
});
if (typeof window !== 'undefined') window.addEventListener('beforeunload', persistThreads);

const Spatial: any = {
  init,
  activate,
  open: openAgentPanel,
  requestFix,
  cancel,
  get active() { return S.active; },
  /* Small pure seams are exposed for deterministic regression tests. */
  math: { motionProfile, shakeReady, shakeIntent, selectionRect, bitmapCropRect, isClickGesture, overlayPointerAction, pointInPolygon, sanitizePlan, sanitizePanelEdit, applyPanelEdit, applyChromeEdit, hintPosition, clampFloatingPosition, textareaLayout, composerMode, normalizeAutonomousResult },
  lifecycle: { applyExtensionChanges, reduceTrace, sealTrace },
};
PM.SpatialAssistant = Spatial;
PM.requestExtensionFix = requestFix;

function agentUISnapshot(): AgentSnapshot {
  ensureThreadProject();
  captureThread();
  // Coalesce drafts/activity; never serialize the entire archive on each token.
  if (!threadSaveTimer) threadSaveTimer = setTimeout(() => {
    const previousError = threadSaveError;
    persistThreads();
    if (threadSaveError !== previousError) PM.AgentUI?.update({ flush: true });
  }, 500);
  S.attachmentUI?.refresh();
  const snapshot: AgentSnapshot = {
    threadId: threads.activeId,
    threads: threads.threads.map(({ id, title }) => ({ id, title })),
    threadSwitchBlocked: !!S.activeRequest || S.phase === 'working' || S.phase === 'applying',
    threadSaveError,
    legacyPhase: S.phase,
    requestToken: S.requestToken,
    conversation: S.conversation.map((message: any) => ({ ...message })),
    activity: S.activity,
    uiPlacement: S.uiPlacement,
    trace: S.trace,
    plan: S.plan,
    run: S.run,
    panelRun: S.panelRun,
    attachments: S.attachments,
    steps: S.steps,
    stepsExpanded: S.stepsExpanded,
    scope: S.scope,
    autoApplyPanels: S.autoApplyPanels,
    provider: S.provider,
    model: S.model,
    reasoningEffort: S.reasoningEffort,
    accessMode: S.accessMode,
    composerDraft: S.composerDraft,
    pendingEntering: S.pendingEntering,
    models: AGENT_MODELS[S.provider],
    providers: AGENT_PROVIDERS,
    reasoningEfforts: REASONING_EFFORTS,
    accessModes: AGENT_ACCESS_MODES,
  };
  S.conversation.forEach((message: any) => { message.entering = false; });
  S.pendingEntering = false;
  return snapshot;
}

registerAgentPanel(PM, {
  flushThreads: persistThreads,
  newThread: () => switchThread(),
  switchThread,
  snapshot: agentUISnapshot,
  submit: (value: string) => { void sendRequest({ value }); },
  stop: stopActiveRequest,
  setDraft: (value: string) => {
    const projectChanged = ensureThreadProject();
    S.composerDraft = value; captureThread();
    clearTimeout(threadSaveTimer); threadSaveTimer = setTimeout(persistThreads, 300);
    if (projectChanged) PM.AgentUI?.update({ flush: true });
  },
  setStepsExpanded: (expanded: boolean) => { S.stepsExpanded = expanded; PM.AgentUI?.update(); },
  setModel: (model: string, effort: string) => {
    if (!AGENT_MODELS[S.provider].some((item: any) => item.id === model) || !REASONING_EFFORTS.includes(effort)) return;
    S.model = model; S.reasoningEffort = effort;
    PM.store.set(`agentModel.${S.provider}`, model); PM.store.set('agentReasoningEffort', effort);
    PM.AgentUI?.update({ focusComposer: true });
  },
  setProvider: (provider: string) => {
    if (!AGENT_PROVIDERS.some((item: any) => item.id === provider) || S.activeRequest) return;
    S.provider = provider;
    S.model = PM.store?.get?.(`agentModel.${provider}`, provider === 'claude' ? 'sonnet' : 'gpt-5.6-sol') || (provider === 'claude' ? 'sonnet' : 'gpt-5.6-sol');
    PM.store.set('agentProvider', provider);
    PM.AgentUI?.update({ focusComposer: true });
  },
  setAccess: setAgentAccessMode,
  confirmComputerAccess: () => {
    S.accessMode = 'computer';
    PM.AgentUI?.update({ focusComposer: true });
  },
  setScope: (scope: string) => {
    S.scope = panelFocusContext(scope || 'workspace', PM.WS?.current, PM.PANELS || {}).scope;
    PM.store.set('agentScope', S.scope);
    PM.AgentUI?.update({ flush: true });
  },
  toggleAutoApplyPanels: () => {
    S.autoApplyPanels = !S.autoApplyPanels;
    PM.store.set('agentAutoApplyPanels', S.autoApplyPanels);
    PM.AgentUI?.update({ focusComposer: true });
  },
  dismissPlan: () => { S.plan = null; PM.AgentUI?.update({ focusComposer: true }); },
  applyPlan: () => { void applyPlan(); },
  addAttachments: addAttachmentFiles,
  removeAttachment,
  importArtifact: (artifact: any) => importAutonomousArtifact(
    S.run?.artifacts?.find((item: any) => item.path === artifact.path) || artifact,
  ),
  revealArtifact: (artifact: any) => PM.AgentArtifacts.reveal(artifact),
  undoPanelRun,
  keepPanelRun,
  undoSceneRun,
  keepSceneRun,
});

function init() {
  if (S.initialized) return;
  S.initialized = true;
  // The agent installs before app.ts chooses the boot project. Rebind its
  // placeholder thread as soon as the first app frame initializes the agent.
  ensureThreadProject();
  PM.AgentUI?.update({ flush: true });
  window.addEventListener('pointerdown', () => { S.pressed = true; }, true);
  window.addEventListener('pointerup', () => { S.pressed = false; }, true);
  window.addEventListener('pointercancel', () => { S.pressed = false; }, true);
  window.addEventListener('pointermove', watchShake, true);
  refreshSceneCache();
}

function openAgentPanel() {
  if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide();
  if (PM.LibraryUI?.isOpen) PM.LibraryUI.close?.();
  const workspace: any = PM.WS?.current;
  if (!workspace) return;
  const visible: any = PM.Layout.hasPanel(workspace, 'agent');
  const hidden: any = (workspace.hiddenPanels || []).some((item: any) => item.id === 'agent');
  if (!visible) {
    PM.WS.mutate((draft: any) => {
      if (hidden) PM.Layout.restorePanel(draft, 'agent');
      else PM.Layout.addPanel(draft, 'agent', 'right');
    });
  }
  window.requestAnimationFrame(() => {
    PM.AgentUI?.update({ focusComposer: true });
    PM.panelInst.agent?.el?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  });
}

function extensionRecords(): any[] {
  const candidates: any[] = [
    PM.Kernel?.loader?.records?.(),
    PM.Kernel?.extensions?.records?.(),
    PM.Kernel?.deps?.extensions?.list?.(),
  ];
  return candidates.find(Array.isArray) || [];
}

function extensionRecord(id: any, records: any[] = extensionRecords()) {
  return records.find((record: any) => record?.id === id);
}

function extensionHealthError(record: any) {
  const error: any = record?.health?.error;
  return typeof error === 'string' && error.trim() ? error.trim() : '';
}

async function requestFix(id: any) {
  const native: any = (window as any).powermove;
  if (typeof native?.extensions?.readSource !== 'function' || typeof native?.codex?.fixPrompt !== 'function') {
    window.console.warn(`[agent] Fix it is unavailable for extension "${String(id)}"`);
    return;
  }
  try {
    const record: any = extensionRecord(id);
    const error: any = extensionHealthError(record);
    const files: any = await native.extensions.readSource({ id });
    const prompt: any = await native.codex.fixPrompt({ id, error, files });
    openAgentPanel();
    setAgentAccessMode('project');
    PM.AgentUI?.setDraft?.(String(prompt), true);
    PM.AgentUI?.submit?.(String(prompt));
  } catch (error: any) {
    window.console.warn(`[agent] Could not prepare Fix it for extension "${String(id)}"`, error);
  }
}

function watchShake(event: any) {
  /* Once summoned, keep the short-lived distortion centered on the live
     pointer. S.origin is the same object read by the WebGPU render loop. */
  if (S.active) {
    const live: any = event.getCoalescedEvents?.().at(-1) || event;
    S.origin.x = live.clientX; S.origin.y = live.clientY;
    scheduleHint(live.clientX, live.clientY);
    return;
  }
  if (!isEditorPointer(event)) { S.samples = []; return; }
  if (S.pressed || event.buttons || window.performance.now() - S.lastTrigger < 1600) return;
  if (!S.cachePending && window.performance.now() - S.sceneCacheAt > 1100) refreshSceneCache();
  const events: any = event.getCoalescedEvents?.().length ? event.getCoalescedEvents() : [event];
  for (const e of events) {
    /* event.timeStamp preserves the real spacing of coalesced samples. Using
       performance.now() for every point made fast mice look like zero-time
       teleports and slow event streams look artificially weak. */
    const eventTime: any = Number.isFinite(e.timeStamp) ? e.timeStamp : window.performance.now();
    const sample: any = { x: e.clientX, y: e.clientY, t: eventTime };
    const last: any = S.samples[S.samples.length - 1];
    if (!last || sample.t > last.t && Math.hypot(sample.x - last.x, sample.y - last.y) >= 1.5) S.samples.push(sample);
  }
  const newest: any = S.samples.at(-1)?.t ?? window.performance.now();
  const cutoff: any = newest - 900;
  S.samples = S.samples.filter((p: any) => p.t >= cutoff).slice(-160);
  if (shakeIntent(S.samples) && !S.rippleWarmup) warmRipple();
  if (shakeReady(S.samples)) {
    const p: any = S.samples[S.samples.length - 1];
    S.samples = []; S.lastTrigger = window.performance.now();
    const warmup: any = S.rippleWarmup;
    if (warmup) warmup.claimed = true;
    S.rippleWarmup = null;
    activate(p.x, p.y, warmup);
  }
}

function isEditorPointer(event: any) {
  const target: any = event?.target;
  return window.opener == null
    && !(PM.ProjectsScreen && PM.ProjectsScreen.isOpen)
    && !(PM.LibraryUI && PM.LibraryUI.isOpen)
    && !window.document.querySelector('#scrim.on,.modal')
    && !!target?.closest?.('#body');
}

function motionProfile(points: any) {
  if (!Array.isArray(points) || points.length < 3) return { duration: 0, path: 0, span: 0, net: 0, reversals: 0, oscillation: 0, peakSpeed: 0, energy: 0 };
  let path: any = 0, reversals: any = 0, oscillation: any = 0, peakSpeed: any = 0, energy: any = 0;
  let minX: any = points[0].x, maxX: any = minX, minY: any = points[0].y, maxY: any = minY;
  let priorVelocity: any = null, distanceSinceTurn: any = 0;
  for (let i: any = 1; i < points.length; i++) {
    const dt: any = points[i].t - points[i - 1].t;
    if (!(dt > 0) || dt > 140) { priorVelocity = null; distanceSinceTurn = 0; continue; }
    const dx: any = points[i].x - points[i - 1].x;
    const dy: any = points[i].y - points[i - 1].y;
    const distance: any = Math.hypot(dx, dy);
    if (distance < .5) continue;
    const seconds: any = Math.max(dt, 4) / 1000;
    const velocity: any = { x: dx / seconds, y: dy / seconds };
    const speed: any = Math.hypot(velocity.x, velocity.y);
    path += distance; distanceSinceTurn += distance;
    peakSpeed = Math.max(peakSpeed, speed);
    energy += speed * speed * seconds;
    if (priorVelocity) {
      const priorSpeed: any = Math.hypot(priorVelocity.x, priorVelocity.y);
      const alignment: any = (velocity.x * priorVelocity.x + velocity.y * priorVelocity.y) / (speed * priorSpeed);
      /* A reversal needs real momentum and travel on both sides. This rejects
         hand tremor/high-frequency sensor jitter without penalizing event rate. */
      if (speed >= 260 && priorSpeed >= 260 && alignment < -.35 && distanceSinceTurn >= 18) {
        reversals++; oscillation += distanceSinceTurn; distanceSinceTurn = 0;
      }
    }
    if (speed >= 120) priorVelocity = velocity;
    minX = Math.min(minX, points[i].x); maxX = Math.max(maxX, points[i].x);
    minY = Math.min(minY, points[i].y); maxY = Math.max(maxY, points[i].y);
  }
  const duration: any = Math.max(0, points.at(-1).t - points[0].t);
  const net: any = Math.hypot(points.at(-1).x - points[0].x, points.at(-1).y - points[0].y);
  return { duration, path, span: Math.max(maxX - minX, maxY - minY), net, reversals, oscillation, peakSpeed, energy: duration ? energy / (duration / 1000) : 0 };
}

function shakeIntent(points: any) {
  const m: any = motionProfile(points);
  return m.duration <= 900 && m.path >= 72 && m.span >= 32 && m.peakSpeed >= 420 && m.reversals >= 1;
}

function warmRipple() {
  const warmup: any = {
    claimed: false,
    /* A selected-region attachment must represent this gesture, not an older
       idle cache. Start a fresh native snapshot as soon as shake intent is clear. */
    capture: PM.WindowCapture.request(),
  };
  S.rippleWarmup = warmup;
  window.setTimeout(() => {
    if (warmup.claimed || S.rippleWarmup !== warmup) return;
    S.rippleWarmup = null;
    warmup.capture?.then((bitmap: any) => bitmap?.close?.());
  }, 1000);
}

function refreshSceneCache() {
  if (S.active || S.cachePending) return;
  const request: any = PM.WindowCapture.request();
  S.cachePending = request;
  request.then((bitmap: any) => {
    if (S.cachePending !== request) { bitmap?.close?.(); return; }
    S.cachePending = null;
    if (!bitmap || S.active) { bitmap?.close?.(); return; }
    S.sceneCache?.close?.();
    S.sceneCache = bitmap; S.sceneCacheAt = window.performance.now();
  });
}

function shakeReady(points: any) {
  const m: any = motionProfile(points);
  /* Three momentum reversals over about 180 CSS pixels is a short intentional
     shake. CSS pixels make the gesture consistent across Retina scale factors;
     timestamp-normalized speed makes it consistent across mouse event rates. */
  return m.duration >= 120 && m.duration <= 900
    && m.path >= 180 && m.span >= 44 && m.oscillation >= 108
    && m.peakSpeed >= 430 && m.reversals >= 3 && m.net < m.path * .62;
}

function hintPosition(x: any, y: any, width: any, height: any, viewportWidth: any, viewportHeight: any, offset: any = 18) {
  const pad: any = 12;
  const left: any = PM.clamp(x + offset, pad, Math.max(pad, viewportWidth - width - pad));
  const below: any = y + offset;
  const top: any = below + height + pad <= viewportHeight
    ? below
    : PM.clamp(y - height - offset, pad, Math.max(pad, viewportHeight - height - pad));
  return { x: Math.round(left), y: Math.round(top) };
}

function scheduleHint(x: any, y: any) {
  S.hintPoint = { x, y };
  if (!S.hint || S.hintFrame) return;
  S.hintFrame = window.requestAnimationFrame(() => {
    S.hintFrame = 0;
    if (!S.hint || !S.hintPoint) return;
    /* The prompt replaces the instruction pill while the assistant is waiting
       for a gesture. Keep the editable surface attached to the live cursor. */
    if (S.card && !S.context?.targetPanelId && ['arming', 'selecting', 'composing'].includes(S.phase)) {
      S.hint.style.display = 'none';
      const p: any = hintPosition(
        S.hintPoint.x, S.hintPoint.y,
        S.card.offsetWidth || 390, S.card.offsetHeight || 52,
        window.innerWidth, window.innerHeight,
      );
      Object.assign(S.card.style, { left: p.x + 'px', top: p.y + 'px' });
      return;
    }
    const p: any = hintPosition(
      S.hintPoint.x, S.hintPoint.y,
      S.hint.offsetWidth || 250, S.hint.offsetHeight || 38,
      window.innerWidth, window.innerHeight,
    );
    S.hint.style.transform = `translate3d(${p.x}px,${p.y}px,0)`;
  });
}

function activate(x: any, y: any, warmup: any = null) {
  if (S.active) return;
  // Load the current project's thread before creating a selection. The first
  // composer publish must not reset this new overlay's region or arming phase.
  ensureThreadProject();
  S.active = true; S.phase = 'arming'; S.origin = { x, y }; S.points = [];
  const rippleHost: any = h('div.spatial-ripple-host', { 'aria-hidden': 'true' });
  S.shadePath = window.document.createElementNS('http://www.w3.org/2000/svg', 'path');
  S.shadePath.classList.add('spatial-shade');
  S.path = window.document.createElementNS('http://www.w3.org/2000/svg', 'path');
  S.ink = window.document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  S.ink.classList.add('spatial-ink'); S.ink.append(S.shadePath, S.path);
  S.hint = h('div.spatial-hint', h('span', 'Type a prompt or drag to select an area'));
  S.root = h('div#spatial-assistant', { role: 'dialog', 'aria-label': 'Spatial coding assistant' },
    rippleHost, h('div.spatial-wash'), S.ink, S.hint);
  window.addEventListener('keydown', onKey, true);
  /* Capture first, while the overlay is not in the DOM. This is the texture the
     WGSL pass genuinely displaces instead of merely painting over the UI. */
  const cachedScene: any = S.sceneCache;
  if (cachedScene) { S.sceneCache = null; S.sceneCacheAt = 0; }
  const sceneRequest: any = warmup?.capture
    ? warmup.capture.then((fresh: any) => {
      if (fresh) { cachedScene?.close?.(); return fresh; }
      return cachedScene;
    })
    : cachedScene ? Promise.resolve(cachedScene) : PM.WindowCapture.request();
  sceneRequest.then((sceneBitmap: any) => {
    if (!S.active) { sceneBitmap?.close?.(); return; }
    /* Preserve clean pre-overlay pixels for the eventual selected-region
       attachment before Motion GPU takes ownership of the ImageBitmap. */
    S.sceneFrame = snapshotScene(sceneBitmap);
    S.root.addEventListener('pointerdown', onOverlayPointerDown);
    window.document.body.appendChild(S.root);
    S.region = { x, y, width: 1, height: 1 };
    S.context = {
      targetPanelId: '', targetTitle: 'full composition and workspace',
      rect: { x: Math.round(x), y: Math.round(y), width: 1, height: 1 }, panels: [], elements: [],
    };
    showComposer();
    scheduleHint(x, y);
    S.renderStop = startRipple(rippleHost, S.origin, sceneBitmap);
    window.setTimeout(() => { if (S.active && S.phase === 'arming') S.phase = 'selecting'; }, 340);
  });
}

function overlayPointerAction(phase: any, button: any, insideComposer: any) {
  if (button !== 0 || insideComposer) return 'ignore';
  if (phase === 'selecting') return 'select';
  if (phase === 'arming' || phase === 'composing') return 'cancel';
  return 'ignore';
}

function onOverlayPointerDown(event: any) {
  const action: any = overlayPointerAction(S.phase, event.button, !!event.target?.closest?.('.spatial-compose'));
  if (action === 'cancel') {
    event.preventDefault();
    cancel();
    return;
  }
  if (action === 'select') beginSelection(event);
}

function beginSelection(event: any) {
  event.preventDefault();
  S.phase = 'drawing'; S.points = [{ x: event.clientX, y: event.clientY }];
  showOutline({ x: event.clientX, y: event.clientY, width: 1, height: 1 }, true);
  S.root.setPointerCapture?.(event.pointerId);
  const move: any = (e: any) => {
    const p: any = { x: e.clientX, y: e.clientY };
    S.points[1] = p;
    updateOutline(selectionRect(S.points[0], p));
  };
  const finish: any = (e: any) => {
    S.root.removeEventListener('pointermove', move);
    S.root.removeEventListener('pointerup', finish);
    S.root.removeEventListener('pointercancel', abort);
    S.root.releasePointerCapture?.(event.pointerId);
    finishSelection(e);
  };
  const abort: any = () => {
    S.root.removeEventListener('pointermove', move);
    S.root.removeEventListener('pointerup', finish);
    S.root.removeEventListener('pointercancel', abort);
    S.outline?.remove(); S.outline = null;
    resetSelection('Drag across any part of the interface');
  };
  S.root.addEventListener('pointermove', move);
  S.root.addEventListener('pointerup', finish);
  S.root.addEventListener('pointercancel', abort);
}

function finishSelection(event: any) {
  if (event) S.points[1] = { x: event.clientX, y: event.clientY };
  if (isClickGesture(S.points)) { cancel(); return; }
  const rect: any = selectionRect(S.points[0], S.points[1]);
  if (rect.width < 34 || rect.height < 34) {
    S.outline?.remove(); S.outline = null;
    resetSelection('Drag across a larger interface area');
    return;
  }
  S.points = rectanglePolygon(rect);
  S.shadePath.setAttribute('fill-rule', 'evenodd');
  S.shadePath.setAttribute('d', `M 0 0 H ${window.innerWidth} V ${window.innerHeight} H 0 Z M ${rect.x} ${rect.y} H ${rect.x + rect.width} V ${rect.y + rect.height} H ${rect.x} Z`);
  S.root.classList.add('target-selected');
  const draft: any = S.card?.querySelector('textarea')?.value || '';
  S.region = rect;
  S.context = inspectRegion(S.points, S.region);
  S.scope = panelScope(S.context.panels.map((panel: any) => panel.id));
  PM.store.set('agentScope', S.scope);
  const regionCapture: any = captureRegionImage(S.sceneFrame, S.region);
  S.regionImage = regionCapture?.dataUrl || null;
  S.context.visualReference = regionCapture ? {
    attachment: 'image-1', width: regionCapture.width, height: regionCapture.height,
    sourceRect: regionCapture.sourceRect,
  } : { attachment: '', unavailable: true };
  S.phase = 'composing';
  S.hint.style.display = 'none';
  S.outline?.classList.remove('selecting'); S.outline?.classList.add('confirmed');
  showComposer(draft);
}

function isClickGesture(points: any) {
  if (!Array.isArray(points) || !points.length) return false;
  const start: any = points[0];
  return points.every((p: any) => Math.hypot(p.x - start.x, p.y - start.y) <= 7);
}

function selectionRect(start: any, end: any) {
  if (!start || !end) return { x: 0, y: 0, width: 0, height: 0 };
  return {
    x: Math.round(Math.min(start.x, end.x)), y: Math.round(Math.min(start.y, end.y)),
    width: Math.round(Math.abs(end.x - start.x)), height: Math.round(Math.abs(end.y - start.y)),
  };
}

function bitmapCropRect(rect: any, bitmapWidth: any, bitmapHeight: any, viewportWidth: any, viewportHeight: any, maxEdge: any = 1280) {
  const scaleX: any = bitmapWidth / Math.max(1, viewportWidth);
  const scaleY: any = bitmapHeight / Math.max(1, viewportHeight);
  const sx: any = Math.max(0, Math.min(bitmapWidth - 1, Math.round(rect.x * scaleX)));
  const sy: any = Math.max(0, Math.min(bitmapHeight - 1, Math.round(rect.y * scaleY)));
  const sw: any = Math.max(1, Math.min(bitmapWidth - sx, Math.round(rect.width * scaleX)));
  const sh: any = Math.max(1, Math.min(bitmapHeight - sy, Math.round(rect.height * scaleY)));
  const outputScale: any = Math.min(1, maxEdge / Math.max(sw, sh));
  return {
    sx, sy, sw, sh,
    width: Math.max(1, Math.round(sw * outputScale)),
    height: Math.max(1, Math.round(sh * outputScale)),
  };
}

function snapshotScene(bitmap: any) {
  if (!bitmap?.width || !bitmap?.height) return null;
  try {
    const scale: any = Math.min(1, 2400 / Math.max(bitmap.width, bitmap.height));
    const canvas: any = window.document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(bitmap.width * scale));
    canvas.height = Math.max(1, Math.round(bitmap.height * scale));
    const context: any = canvas.getContext('2d', { alpha: false });
    if (!context) return null;
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    return canvas;
  } catch (error: any) {
    window.console.warn('Could not retain the interface snapshot for selected-region context', error);
    return null;
  }
}

function captureRegionImage(sceneFrame: any, rect: any) {
  if (!sceneFrame?.width || !sceneFrame?.height || !rect?.width || !rect?.height) return null;
  try {
    const crop: any = bitmapCropRect(rect, sceneFrame.width, sceneFrame.height, window.innerWidth, window.innerHeight);
    const canvas: any = window.document.createElement('canvas');
    canvas.width = crop.width; canvas.height = crop.height;
    const context: any = canvas.getContext('2d', { alpha: false });
    if (!context) return null;
    context.drawImage(
      sceneFrame, crop.sx, crop.sy, crop.sw, crop.sh, 0, 0, crop.width, crop.height,
    );
    return {
      dataUrl: canvas.toDataURL('image/jpeg', .92), width: crop.width, height: crop.height,
      sourceRect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
    };
  } catch (error: any) {
    window.console.warn('Could not crop the selected interface region', error);
    return null;
  }
}

function rectanglePolygon(rect: any) {
  return [
    { x: rect.x, y: rect.y }, { x: rect.x + rect.width, y: rect.y },
    { x: rect.x + rect.width, y: rect.y + rect.height }, { x: rect.x, y: rect.y + rect.height },
  ];
}

function resetSelection(message: any) {
  S.phase = 'selecting'; S.points = []; S.path.setAttribute('d', '');
  S.regionImage = null;
  S.shadePath?.setAttribute('d', ''); S.root?.classList.remove('target-selected');
  if (S.card && !S.context?.targetPanelId) {
    const input: any = S.card.querySelector('textarea');
    if (input) {
      const original: any = 'Full composition';
      input.placeholder = message;
      window.setTimeout(() => { if (S.active && S.phase === 'selecting' && input) input.placeholder = original; }, 1400);
    }
    S.hint.style.display = 'none';
    scheduleHint(S.hintPoint?.x ?? S.origin.x, S.hintPoint?.y ?? S.origin.y);
    return;
  }
  S.hint.style.display = ''; S.hint.querySelector('span').textContent = message;
  window.setTimeout(() => {
    if (S.active && S.phase === 'selecting') S.hint.querySelector('span').textContent = 'Type a prompt or drag to select an area';
  }, 1400);
}

function pointInPolygon(point: any, polygon: any) {
  let inside: any = false;
  for (let i: any = 0, j: any = polygon.length - 1; i < polygon.length; j = i++) {
    const a: any = polygon[i], b: any = polygon[j];
    const hit: any = ((a.y > point.y) !== (b.y > point.y))
      && point.x < (b.x - a.x) * (point.y - a.y) / ((b.y - a.y) || .00001) + a.x;
    if (hit) inside = !inside;
  }
  return inside;
}

function inspectRegion(polygon: any, rect: any) {
  const seen: any = new Map();
  S.root.style.pointerEvents = 'none';
  for (let gy: any = 0; gy < 5; gy++) for (let gx: any = 0; gx < 5; gx++) {
    const p: any = { x: rect.x + rect.width * (gx + .5) / 5, y: rect.y + rect.height * (gy + .5) / 5 };
    if (!pointInPolygon(p, polygon)) continue;
    window.document.elementsFromPoint(p.x, p.y).forEach((el: any) => {
      if (el === window.document.body || el === window.document.documentElement || el.closest('#spatial-assistant')) return;
      const key: any = el.id || `${el.tagName}.${[...el.classList].slice(0, 3).join('.')}`;
      if (!seen.has(key) && seen.size < 18) seen.set(key, describeElement(el));
    });
  }
  S.root.style.pointerEvents = '';
  // Exact rectangle intersections also catch narrow panels missed by the sample grid.
  const ranked = intersectingPanels(rect, Array.from(window.document.querySelectorAll<HTMLElement>('#body .panel[data-panel]')).map(panel => {
    const bounds = panel.getBoundingClientRect();
    return { id: panel.dataset.panel || '', rect: { x: bounds.left, y: bounds.top, width: bounds.width, height: bounds.height } };
  }));
  const targetPanelId: any = ranked[0] || '';
  const target: any = targetPanelId ? PM.$(`#panel-${window.CSS.escape(targetPanelId)}`) : null;
  return {
    targetPanelId,
    targetTitle: target?.querySelector('.ptitle')?.textContent?.trim() || (targetPanelId ? PM.PANELS[targetPanelId]?.title : '') || 'interface area',
    rect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
    panels: ranked.map(id => ({ id, title: PM.PANELS[id]?.title || id })).slice(0, 32),
    elements: [...seen.values()],
  };
}

function describeElement(el: any) {
  return {
    tag: el.tagName.toLowerCase(), id: el.id || '',
    classes: [...el.classList].slice(0, 4),
    label: (el.getAttribute('aria-label') || el.getAttribute('title') || '').slice(0, 100),
    text: String(el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 180),
  };
}

function updateOutline(rect: any) {
  if (!S.outline) return;
  Object.assign(S.outline.style, {
    left: Math.max(4, rect.x) + 'px', top: Math.max(4, rect.y) + 'px',
    width: Math.max(1, Math.min(window.innerWidth - Math.max(4, rect.x) - 4, rect.width)) + 'px',
    height: Math.max(1, Math.min(window.innerHeight - Math.max(4, rect.y) - 4, rect.height)) + 'px',
  });
}

function showOutline(rect: any, selecting: any = false) {
  S.outline?.remove();
  S.outline = h('div.spatial-outline', { style: {
    left: Math.max(4, rect.x) + 'px', top: Math.max(4, rect.y) + 'px',
    width: Math.max(1, rect.width) + 'px', height: Math.max(1, rect.height) + 'px',
  } });
  if (selecting) S.outline.classList.add('selecting');
  S.root.appendChild(S.outline);
}

function cardPosition(card: any, rect: any) {
  const left: any = PM.clamp(rect.x + rect.width + 14, 14, window.innerWidth - Math.min(420, window.innerWidth - 28) - 14);
  let top: any = PM.clamp(rect.y, 60, window.innerHeight - card.offsetHeight - 14);
  if (left < rect.x + rect.width && rect.y + rect.height + 14 + card.offsetHeight < window.innerHeight) top = rect.y + rect.height + 14;
  Object.assign(card.style, { left: Math.round(left) + 'px', top: Math.round(top) + 'px' });
}

function composerPosition(card: any) {
  /* Full-composition prompts belong to the visible canvas, not the pointer's
     activation point. Selected-area edits remain spatially anchored to their region. */
  if (S.context?.targetPanelId) { cardPosition(card, S.region); return; }
  const canvas: any = window.document.querySelector('#stage-inner')?.getBoundingClientRect();
  if (!canvas || !canvas.width || !canvas.height) { cardPosition(card, S.region); return; }
  const width: any = card.offsetWidth || 390;
  const p: any = clampFloatingPosition(canvas.left + 22, canvas.top + 18, width, card.offsetHeight || 92, window.innerWidth, window.innerHeight, 14);
  Object.assign(card.style, { left: p.x + 'px', top: p.y + 'px' });
}

function clampFloatingPosition(x: any, y: any, width: any, height: any, viewportWidth: any, viewportHeight: any, pad: any = 12) {
  return {
    x: Math.round(PM.clamp(x, pad, Math.max(pad, viewportWidth - width - pad))),
    y: Math.round(PM.clamp(y, pad, Math.max(pad, viewportHeight - height - pad))),
  };
}

function moveCardTo(card: any, x: any, y: any) {
  const p: any = clampFloatingPosition(x, y, card.offsetWidth || 420, card.offsetHeight || 180, window.innerWidth, window.innerHeight);
  Object.assign(card.style, { left: p.x + 'px', top: p.y + 'px' });
  return p;
}

function textareaLayout(scrollHeight: any, minHeight: any, maxHeight: any) {
  const contentHeight: any = Math.max(0, Math.ceil(Number(scrollHeight) || 0));
  const min: any = Number.isFinite(minHeight) ? Math.max(0, minHeight) : 0;
  const max: any = Number.isFinite(maxHeight) ? Math.max(min, maxHeight) : Number.POSITIVE_INFINITY;
  const height: any = Math.min(max, Math.max(min, contentHeight));
  return { height, overflowY: contentHeight > height + 1 ? 'auto' : 'hidden' };
}

function autosizeTextarea(input: any, onResize?: any) {
  const resize: any = () => {
    input.style.height = 'auto';
    const style: any = window.getComputedStyle(input);
    const layout: any = textareaLayout(input.scrollHeight, Number.parseFloat(style.minHeight), Number.parseFloat(style.maxHeight));
    input.style.height = `${layout.height}px`;
    input.style.overflowY = layout.overflowY;
    onResize?.();
  };
  input.addEventListener('input', resize);
  resize();
}

function cardCoordinates(card: any) {
  const left: any = Number.parseFloat(card.style.left), top: any = Number.parseFloat(card.style.top);
  if (Number.isFinite(left) && Number.isFinite(top)) return { left, top };
  return card.getBoundingClientRect();
}

function makeCardMovable(card: any, handle: any) {
  handle.tabIndex = 0; handle.setAttribute('role', 'button');
  handle.setAttribute('aria-label', 'Move assistant result');
  handle.addEventListener('pointerdown', (event: any) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    const rect: any = cardCoordinates(card);
    /* The conversation's parent is deliberately click-through. Use the shared
       window-level drag helper so movement continues after leaving the header. */
    PM.drag(event, {
      cursor: 'grabbing',
      move: (dx: any, dy: any) => moveCardTo(card, rect.left + dx, rect.top + dy),
    });
  });
  handle.addEventListener('keydown', (event: any) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const rect: any = card.getBoundingClientRect(), step: any = event.shiftKey ? 24 : 8;
    const dx: any = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0;
    const dy: any = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0;
    moveCardTo(card, rect.left + dx, rect.top + dy);
  });
}

function showComposer(draft: any = '') {
  S.attachmentUI?.dispose();
  if (S.focusPicker) { void unmount(S.focusPicker); S.focusPicker = null; }
  S.card?.remove();
  const selectedContext: any = !!S.context.targetPanelId;
  const input: any = h('textarea', {
    placeholder: selectedContext ? 'How should this area change?' : 'Full composition',
    rows: '1', 'data-autosize': 'true',
  });
  input.value = draft;
  const status: any = h('span.spatial-status', 'Enter to send');
  const cancelBtn: any = h('button.spatial-action', { onclick: cancel }, 'Cancel');
  const sendBtn: any = h('button.spatial-action.pri.spatial-send', { 'aria-label': 'Press Enter to send', title: 'Press Enter to send', onclick: () => sendRequest(input) }, PM.icon('return'));
  input.addEventListener('keydown', (e: any) => {
    e.stopPropagation();
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendRequest(input); }
  });
  const targetLabel: any = selectedContext ? `Selected · ${panelFocusContext(S.scope, PM.WS?.current, PM.PANELS || {}).label}` : 'Powermove agent · full composition';
  const handle: any = h('div.spatial-target', { title: targetLabel }, targetLabel);
  const focusHost = h('div.spatial-focus-host');
  S.card = h('div.spatial-compose.compact.full',
    handle,
    h('div.spatial-input-row', input, sendBtn),
    focusHost,
    h('div.spatial-actions', status, cancelBtn));
  S.root.appendChild(S.card);
  S.attachmentUI = mountPromptAttachments(PM, S.card, () => S.attachments);
  PM.AgentUI?.update({ flush: true });
  S.focusPicker = mount(AgentOptions, { target: focusHost, props: { PM } });
  flushSync();
  autosizeTextarea(input, () => {
    if (!S.card || !input.isConnected) return;
    const position: any = cardCoordinates(S.card);
    moveCardTo(S.card, position.left, position.top);
  });
  /* Place and focus immediately as well as on the next frame. A cached window
     capture can make repeat activations complete inside the click dispatch;
     waiting only for rAF left the card at an unpositioned static location. */
  composerPosition(S.card);
  input.focus();
  window.requestAnimationFrame(() => {
    if (!S.active || !S.card) return;
    composerPosition(S.card);
    input.focus();
  });
}

function conversationReply(plan: any) {
  if (plan.kind === 'scene') return plan.sceneEdit.summary || plan.message || 'I prepared an editable scene change.';
  if (plan.kind === 'panels') return plan.message || `I prepared ${plan.panelEdit.actions.length} panel changes.`;
  if (plan.kind === 'workspace') return plan.message || `I prepared the ${plan.workspaceEdit.name} workspace.`;
  if (plan.kind === 'interface') return plan.message || 'I prepared a structured Timeline redesign.';
  if (plan.kind === 'chrome') return plan.message || 'I prepared an interface change.';
  return plan.message || `I prepared the ${plan.section.title} section.`;
}

function promoteToConversation() {
  dismissOverlay(true);
  openAgentPanel();
}

function setAgentAccessMode(mode: any) {
  if (mode === 'computer' || !AGENT_ACCESS_MODES.some((item: any) => item.id === mode)) return;
  S.accessMode = mode;
  PM.store.set('agentAccessMode', mode);
  PM.AgentUI?.update({ focusComposer: true });
}

let attachmentQueue = Promise.resolve();
async function addAttachmentFiles(files: any) {
  const thread = threads.active;
  const pendingFiles = [...files];
  attachmentQueue = attachmentQueue.then(async () => {
    for (const file of pendingFiles) {
      if (thread.attachments.length >= 6) { PM.toast('You can attach up to 6 files per message.'); break; }
      try { thread.attachments.push(await readPromptAttachment(file, PM.uid('attachment-'))); }
      catch (error) { PM.toast(error instanceof Error ? error.message : String(error), 6000); }
    }
    threads.save();
    PM.AgentUI?.update({ focusComposer: true });
  });
  await attachmentQueue;
}

async function importAutonomousArtifact(artifact: any) {
  if (!artifact || artifact.importing || artifact.imported) return;
  artifact.importing = true; PM.AgentUI?.update();
  try {
    const file: any = await PM.AgentArtifacts.load(artifact);
    if (!PM.assetKind(file)) throw new Error('This artifact is not a supported image, video, or audio file');
    await PM.importFiles([file]);
    artifact.imported = true;
    PM.toast(`Added ${artifact.name} to the timeline`);
  } catch (error: any) {
    PM.toast(String(error.message || error), 6000);
  } finally {
    artifact.importing = false; PM.AgentUI?.update();
  }
}

function removeAttachment(id: any) {
  S.attachments = S.attachments.filter((item: any) => item.id !== id);
  PM.AgentUI?.update({ focusComposer: true });
}

function stopActiveRequest() {
  if (S.phase !== 'working') return;
  const active: any = S.activeRequest;
  S.activeRequest = null;
  ++S.requestToken;
  S.uiPlacement = null;
  active?.abort();
  sealTrace();
  S.steps = []; S.activity = ''; S.plan = null; S.phase = 'conversation';
  archiveTrace();
  S.conversation.push({ role: 'assistant', text: 'Stopped. Add direction whenever you are ready.' });
  PM.AgentUI?.update({ focusComposer: true });
}

const TRACE_STEP_LIMIT: any = 200;
const TRACE_TEXT_LIMIT: any = 2_000;

function finishTraceThought() {
  const last: any = S.trace.at(-1);
  if (last?.kind === 'thought' && last.live) last.live = false;
}

function trimTrace() {
  while (S.trace.length > TRACE_STEP_LIMIT) {
    const removable: any = S.trace.findIndex((step: any) => step.kind !== 'text');
    S.trace.splice(removable >= 0 ? removable : 0, 1);
  }
}

function reduceTrace(step: CodexTraceEvent) {
  if (!step || typeof step !== 'object') return;
  if ((step.kind === 'answer' || step.kind === 'thought') && isUIPlacementMessage(step.text)) {
    // Only explicit public commentary can place a ghost, never reasoning or tool output.
    if (step.kind === 'answer' && S.phase === 'working') {
      const placement = parseUIPlacement(step.text, PM.WS?.current);
      if (placement) {
        S.uiPlacement = placement;
        S.activity = `Working on ${placement.label}…`;
      }
    }
    return;
  }
  if (step.kind === 'thought') {
    if (!step.text) return;
    let thought: any = S.trace.at(-1);
    if (thought?.kind !== 'thought' || !thought.live) {
      thought = { kind: 'thought', id: PM.uid('trace-thought-'), label: '', live: true };
      S.trace.push(thought);
    }
    thought.label = `${thought.label}${step.text}`.slice(0, TRACE_TEXT_LIMIT);
  } else if (step.kind === 'answer') {
    if (!step.text) return;
    finishTraceThought();
    let text: any = S.trace.at(-1);
    if (text?.kind !== 'text') {
      text = { kind: 'text', id: PM.uid('trace-text-'), text: '' };
      S.trace.push(text);
    }
    text.text = `${text.text}${step.text}`.slice(0, TRACE_TEXT_LIMIT);
  } else if (step.kind === 'tool-start') {
    finishTraceThought();
    S.trace.push({
      kind: 'tool', id: step.itemId, toolName: step.toolName,
      label: step.label, status: 'running',
    });
  } else if (step.kind === 'tool-end') {
    const tool: any = [...S.trace].reverse().find((entry: any) => entry.kind === 'tool' && entry.id === step.itemId);
    if (tool) tool.status = step.isError ? 'error' : 'done';
  }
  trimTrace();
}

function sealTrace() {
  finishTraceThought();
  S.trace.forEach((step: any) => {
    if (step.kind === 'tool' && step.status === 'running') step.status = 'continued';
  });
}

/* Move the finished run's trace into the conversation so the activity trail
   stays visible above its summary (supermove keeps per-message steps). Text
   rows are dropped — the final answer lands in its own conversation turn. */
function archiveTrace() {
  sealTrace();
  const steps: any = S.trace.filter((step: any) => step.kind !== 'text');
  S.trace = [];
  if (steps.length) S.conversation.push({ role: 'trace', steps });
}

function normalizeAutonomousResult(raw: any, rawExtensions: any) {
  const text: any = (value: any) => String(value || '').replace(/\s+/g, ' ').trim();
  const projectId: any = text(raw?.projectId || PM.proj?.id).slice(0, 160);
  const extensionActions: any = new Set(['created', 'updated', 'removed']);
  const extensions: any = (Array.isArray(rawExtensions) ? rawExtensions : []).filter((item: any) => {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return false;
    if (typeof item.id !== 'string' || !/^[a-z0-9][a-z0-9-]{1,63}$/.test(item.id)) return false;
    if (!extensionActions.has(item.action)) return false;
    return item.summary === undefined || (typeof item.summary === 'string' && item.summary.length <= 200);
  }).slice(0, 32).map((item: any) => ({
    id: item.id,
    action: item.action,
    ...(item.summary === undefined ? {} : { summary: item.summary }),
  }));
  return {
    summary: text(raw?.summary || 'The autonomous agent finished its run.').slice(0, 800),
    commands: (Array.isArray(raw?.commands) ? raw.commands : [])
      .slice(0, 80).map(PM.AgentHarness.cleanCommand).filter(Boolean),
    artifacts: (Array.isArray(raw?.artifacts) ? raw.artifacts : []).slice(0, 80).map((item: any) => ({
      projectId,
      path: text(item?.path).slice(0, 600),
      name: text(item?.name || String(item?.path || '').split('/').pop() || 'Agent artifact').slice(0, 240),
      mime: text(item?.mime || 'application/octet-stream').slice(0, 120),
      size: Math.max(0, Number(item?.size) || 0),
      importToTimeline: item?.importToTimeline === true,
      imported: false,
    })).filter((item: any) => item.path),
    externalActions: (Array.isArray(raw?.externalActions) ? raw.externalActions : []).slice(0, 80).map(text).filter(Boolean),
    notes: (Array.isArray(raw?.notes) ? raw.notes : []).slice(0, 80).map(text).filter(Boolean),
    extensions,
  };
}

async function applyExtensionChanges(extensions: any) {
  const turns: any[] = [];
  for (const change of Array.isArray(extensions) ? extensions : []) {
    let loadError: any = '';
    let record: any;
    if (change.action === 'created' || change.action === 'updated') {
      const loader: any = PM.Kernel?.loader;
      try {
        if (typeof loader?.reload === 'function') await loader.reload(change.id);
      } catch (error: any) {
        loadError = String(error?.message || error || 'Unknown extension error');
      }
      /* A freshly created mod reaches the loader through main's file watcher
         (debounced + compiled), so the record may not exist yet: drain the
         loader queue and wait briefly for it before judging health. */
      const deadline: any = Date.now() + 4_000;
      for (;;) {
        if (typeof loader?.whenIdle === 'function') { try { await loader.whenIdle(); } catch { /* ignore */ } }
        record = extensionRecord(change.id, extensionRecords());
        if (record || Date.now() >= deadline) break;
        await new Promise(r => setTimeout(r, 150));
      }
      if (!record && !loadError) loadError = 'Powermove could not find this mod after the agent finished.';
    } else {
      record = extensionRecord(change.id, extensionRecords());
    }
    const name: any = record?.manifest?.name || change.id;
    const verb: any = change.action === 'created' ? 'Added' : change.action === 'updated' ? 'Updated' : 'Removed';
    turns.push({ role: 'assistant', text: `${verb} mod ${change.action === 'removed' ? change.id : name}` });

    const error: any = loadError || extensionHealthError(record);
    if ((change.action === 'created' || change.action === 'updated') && error) {
      const firstLine: any = (String(error).split(/\r?\n/, 1)[0] || '').slice(0, 400);
      turns.push({ role: 'assistant', text: `${name} didn't load: ${firstLine}`, fixExtensionId: change.id });
    }
  }
  return turns;
}

async function runAutonomousRequest({ request, token, controller, access, focus, context, threadId }: any) {
  const baseRevision: any = Number(PM.proj.revision) || 0;
  const checkpoint: any = {
    id: PM.uid('agent-checkpoint'),
    label: `Before autonomous agent · ${request.slice(0, 42)}`,
    json: JSON.stringify(PM.proj),
  };
  try { checkpoint.takeId = PM.takes?.save(`Before autonomous agent · ${request.slice(0, 42)}`)?.id || null; }
  catch { checkpoint.takeId = null; }
  const historyMark: any = PM.hist.mark();
  const observationPromise: any = PM.AgentHarness ? PM.AgentHarness.observe() : Promise.resolve({ state: {}, times: [], images: [] });
  const [observation]: any = await Promise.all([
    observationPromise,
    new Promise((resolve: any) => window.setTimeout(resolve, SEND_TRANSITION_MS)),
  ]);
  if (token !== S.requestToken) return;
  S.steps[0].status = 'complete'; S.steps[1].status = 'active';
  S.activity = access === 'computer' ? 'Working across Powermove and this Mac…' : 'Researching and working inside the project workspace…';
  PM.AgentUI?.update();
  const userImages: any = S.requestAttachments.filter((item: any) => item.dataUrl).map((item: any) => item.dataUrl);
  const attachedImages: any = [...userImages, ...(S.regionImage ? [S.regionImage] : []), ...observation.images].slice(0, 6);
  const raw: any = await PM.CodexBridge.request(`${request}\n\n${AGENT_TESTING_INSTRUCTIONS}\n\nCONVERSATION IN THIS THREAD\n${JSON.stringify(S.conversation.filter((m: any) => m.role !== 'trace').slice(0, -1).slice(-12).map((m: any) => ({ role: m.role, text: m.text })))}\n\n${panelFocusPrompt(focus)}\n\nSELECTED REGION REFERENCE\n${JSON.stringify(context)}\n\n${NATIVE_PANEL_DESIGN}\n\n${uiPlacementInstructions(PM.WS?.current)}`, null, attachedImages, {
    mode: 'autonomous', access,
    threadId,
    projectId: PM.proj.id, projectName: PM.proj.name || 'Untitled',
    projectJSON: JSON.stringify(PM.proj),
    attachments: requestFileAttachments(S.requestAttachments),
    provider: S.provider,
    model: S.model, reasoningEffort: S.reasoningEffort, signal: controller.signal,
    timeoutMs: 3_600_000,
    onProgress: (summary: any) => {
      if (token !== S.requestToken || !summary || isUIPlacementMessage(summary)) return;
      S.activity = summary; PM.AgentUI?.update();
    },
    onTrace: (step: CodexTraceEvent) => {
      if (token !== S.requestToken) return;
      reduceTrace(step); PM.AgentUI?.update();
    },
  });
  if (token !== S.requestToken) return;
  const responseText: any = typeof raw === 'string' ? raw : raw?.text;
  let decoded: any;
  try { decoded = JSON.parse(responseText); }
  catch { throw new Error('The autonomous agent returned an invalid result'); }
  const result: any = normalizeAutonomousResult(decoded, typeof raw === 'object' ? raw?.extensions : []);
  S.steps[1].status = 'complete'; S.steps[2].status = 'active';
  S.activity = 'Bringing the result back into Powermove…'; PM.AgentUI?.update();
  let changed: any = false;
  let reviewError: any = '';
  if ((Number(PM.proj.revision) || 0) !== baseRevision) {
    reviewError = 'Project changed while the autonomous agent was working, so its proposed source edits and automatic imports were left unapplied.';
  } else {
    const commands: any = result.commands;
    if (commands.length) {
      const applied: any = PM.Edit.apply(commands, {
        label: 'Autonomous agent', origin: 'agent', baseRevision,
      });
      if (!applied.ok) throw new Error(applied.message);
      changed = true;
    }
    for (const artifact of result.artifacts.filter((item: any) => item.importToTimeline)) {
      try {
        const file: any = await PM.AgentArtifacts.load(artifact);
        if (!PM.assetKind(file)) throw new Error(`${artifact.name} is not supported project media`);
        await PM.importFiles([file]);
        artifact.imported = true; changed = true;
      } catch (error: any) {
        reviewError += `${reviewError ? ' ' : ''}${String(error.message || error)}`;
      }
    }
  }
  const extensionTurns: any = await applyExtensionChanges(result.extensions);
  if (token !== S.requestToken) return;
  checkpoint.historyId = changed ? PM.hist.squash(historyMark, 'Autonomous agent') : null;
  const finalFrames: any = changed && PM.AgentHarness ? await PM.AgentHarness.observe() : observation;
  finishSteps();
  archiveTrace();
  S.conversation.push({ role: 'assistant', text: result.summary });
  S.conversation.push(...extensionTurns);
  S.run = {
    autonomous: true, summary: result.summary, checkpoint,
    extensionChangeSetId: typeof raw === 'object' ? raw?.extensionChangeSetId : undefined,
    projectId: PM.proj.id,
    applied: result.commands, changed, artifacts: result.artifacts,
    externalActions: result.externalActions, extensions: result.extensions,
    review: { message: result.notes.join(' ') || (changed ? 'The editable Powermove result is ready to review.' : 'The agent run completed without changing Powermove source.') },
    frames: finalFrames, reviewError,
  };
  /* Deliberate Svelte deviation from HEAD: result transitions restore the
     persistent composer's focus instead of relying on a DOM rebuild. */
  S.activity = ''; S.phase = 'result'; PM.AgentUI?.update({ focusComposer: true });
}

async function sendRequest(input: any) {
  const typedRequest: any = input.value.trim();
  ensureThreadProject();
  if ((!typedRequest && !S.attachments.length) || S.phase === 'applying') return;
  const threadIdAtStart: any = threads.activeId;
  const request: any = typedRequest || 'Review the attached files and make the relevant editable change.';
  const focus = panelFocusContext(S.scope, PM.WS?.current, PM.PANELS || {});
  const context = S.context ? JSON.parse(JSON.stringify(S.context)) : null;
  const steering: any = S.phase === 'working';
  const previousRequest: any = S.activeRequest;
  const token: any = ++S.requestToken;
  const controller: any = new window.AbortController();
  S.activeRequest = controller;
  previousRequest?.abort();
  S.requestText = request;
  S.trace = [];
  S.uiPlacement = null;
  S.requestAttachments = S.attachments.splice(0);
  S.composerDraft = '';
  S.conversation.push({
    role: 'user', text: typedRequest || `Attached ${S.requestAttachments.length} file${S.requestAttachments.length === 1 ? '' : 's'}`,
    focusLabels: focus.panels.map(panel => panel.title),
    attachments: S.requestAttachments.map((item: any) => ({ name: item.name, type: item.type, dataUrl: item.dataUrl })), entering: true,
  });
  threads.active.updatedAt = Date.now();
  persistThreads();
  const accessAtStart: any = S.accessMode;
  const autonomous: any = accessAtStart !== 'editor';
  updateSteps(autonomous ? [
    'Understand the request and project',
    'Research and operate the required tools',
    'Bring artifacts and edits into Powermove',
    'Review the result and external activity',
  ] : steering ? [
    'Review the new direction',
    'Revise the editable change',
    'Prepare the updated source edits',
    'Review the visible result',
  ] : [
    'Understand the request and context',
    'Design an editable change',
    'Prepare the source edits',
    'Review the visible result',
  ], 0);
  S.stepsExpanded = false;
  S.plan = null; S.panelRun = null; S.phase = 'working';
  S.activity = steering ? 'Updating the run with your direction…' : 'Looking at the composition and workspace…';
  S.pendingEntering = true;
  promoteToConversation(); PM.AgentUI?.update({ focusComposer: true });
  try {
    if (autonomous) {
      await runAutonomousRequest({ request, token, controller, access: accessAtStart, focus, context, threadId: threadIdAtStart });
      return;
    }
    /* Let the send handoff finish before the next progress render replaces the
       message DOM. Observation still runs immediately, so the beat adds only
       the portion of the 240 ms transition that useful work did not consume. */
    const observationPromise: any = PM.AgentHarness ? PM.AgentHarness.observe() : Promise.resolve({ state: {}, times: [], images: [] });
    const [observation]: any = await Promise.all([
      observationPromise,
      new Promise((resolve: any) => window.setTimeout(resolve, SEND_TRANSITION_MS)),
    ]);
    if (token !== S.requestToken) return;
    S.steps[0].status = 'complete'; S.steps[1].status = 'active';
    S.activity = steering ? 'Reworking the editable change…' : 'Designing an editable change…'; PM.AgentUI?.update({ focusComposer: true });
    const userImages: any = S.requestAttachments.filter((item: any) => item.dataUrl).map((item: any) => item.dataUrl);
    const attachedImages: any = [...userImages, ...(S.regionImage ? [S.regionImage] : []), ...observation.images].slice(0, 6);
    const raw: any = await PM.CodexBridge.request(
      agentPrompt(request, observation, steering, focus, context), responseSchema(), attachedImages,
      {
        threadId: threadIdAtStart,
        attachments: requestFileAttachments(S.requestAttachments),
        provider: S.provider,
        model: S.model, reasoningEffort: S.reasoningEffort, signal: controller.signal,
        onProgress: (summary: any) => {
          if (token !== S.requestToken || !summary || isUIPlacementMessage(summary)) return;
          S.activity = summary; PM.AgentUI?.update();
        },
        onTrace: (step: CodexTraceEvent) => {
          if (token !== S.requestToken) return;
          reduceTrace(step); PM.AgentUI?.update();
        },
      },
    );
    if (token !== S.requestToken) return;
    let decoded: any;
    try { decoded = JSON.parse(raw); } catch { throw new Error('The coding agent returned an invalid section'); }
    const plan: any = sanitizePlan(decoded, { ...context, targetPanelId: focus.panels[0]?.id || context?.targetPanelId || '' }, request);
    if (plan.operation === 'noop') throw new Error(plan.message || 'I could not turn that into an editable change yet');
    if (plan.kind === 'scene' && !plan.sceneEdit.commands.length) throw new Error(plan.message || 'I could not prepare the composition edit');
    if (plan.kind === 'section' && !plan.section.controls.length) throw new Error('The generated section had no controls connected to editable source');
    if (plan.kind === 'workspace' && !plan.workspaceEdit) throw new Error('The generated workspace was not safe or complete enough to preview');
    if (plan.kind === 'panels' && !plan.panelEdit.actions.length) throw new Error('I could not find a valid panel action to perform');
    updateSteps(plan.steps);
    archiveTrace();
    S.stepsExpanded = S.steps.length > 1;
    S.conversation.push({ role: 'assistant', text: conversationReply(plan) });
    S.activity = ''; S.plan = plan; S.phase = 'conversation';
    if (plan.kind === 'panels' && S.autoApplyPanels) await applyPlan();
    else showPreview();
  } catch (error: any) {
    if (token !== S.requestToken) return;
    if (error?.name === 'AbortError') return;
    const current: any = S.steps.find((step: any) => step.status === 'active'); if (current) current.status = 'error';
    archiveTrace();
    S.activity = ''; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', error: true, text: String(error.message || error).slice(0, 300) });
    PM.AgentUI?.update({ focusComposer: true });
  } finally {
    if (token === S.requestToken) {
      S.uiPlacement = null;
      sealTrace();
      if (S.activeRequest === controller) S.activeRequest = null;
      PM.AgentUI?.update({ flush: true });
    }
    if (accessAtStart === 'computer' && token === S.requestToken) {
      S.accessMode = 'project';
      if (token === S.requestToken && S.phase !== 'working') PM.AgentUI?.update();
    }
  }
}

function responseSchema() {
  return {
    type: 'object', additionalProperties: false,
    required: ['kind', 'operation', 'targetPanelId', 'dockId', 'placement', 'message', 'steps', 'chromeEdit', 'interfaceEdit', 'section', 'sceneEdit', 'workspaceEdit', 'panelEdit'],
    properties: {
      kind: { type: 'string', enum: ['section', 'chrome', 'interface', 'scene', 'workspace', 'panels'] },
      operation: { type: 'string', enum: ['create', 'modify', 'noop'] },
      targetPanelId: { type: 'string' }, dockId: { type: 'string' },
      placement: { type: 'string', enum: ['before', 'after', 'replace'] },
      message: { type: 'string' },
      steps: { type: 'array', minItems: 1, maxItems: 6, items: { type: 'string' } },
      chromeEdit: {
        type: 'object', additionalProperties: false, required: ['target', 'value'],
        properties: {
          target: { type: 'string', enum: ['preview.cornerRadius', 'timeline.surfaceOrder'] },
          value: { type: 'string', enum: ['square', 'rounded', 'normal', 'reversed'] },
        },
      },
      interfaceEdit: { type: 'string' },
      section: {
        type: 'object', additionalProperties: false, required: ['id', 'title', 'icon', 'size', 'note', 'tool', 'controls'],
        properties: {
          id: { type: 'string' }, title: { type: 'string' }, icon: { type: 'string' }, size: { type: 'number' }, note: { type: 'string' }, tool: { type: 'string' },
          controls: { type: 'array', maxItems: 64, items: {
            type: 'object', additionalProperties: false,
            required: ['type', 'label', 'parameter', 'defaultValue', 'min', 'max', 'step', 'options', 'target', 'path', 'command', 'stateKey', 'source', 'action', 'primary'],
            properties: {
              type: { type: 'string', enum: ['slider', 'text', 'color', 'fill', 'toggle', 'select', 'button', 'readout', 'curve'] }, label: { type: 'string' },
              parameter: { type: 'string' }, defaultValue: { anyOf: [{ type: 'string' }, { type: 'number' }, { type: 'boolean' }, { type: 'null' }] }, min: { type: 'number' }, max: { type: 'number' }, step: { type: 'number' },
              options: { type: 'array', items: { type: 'string' } }, target: { type: 'string' }, path: { type: 'string' }, command: { type: 'string' },
              stateKey: { type: 'string' }, source: { type: 'string' }, action: { type: 'string' }, primary: { type: 'boolean' },
            },
          } },
        },
      },
      sceneEdit: PM.AgentHarness.sceneSchema(),
      workspaceEdit: { type: 'string' },
      panelEdit: { type: 'string' },
    },
  };
}

function agentPrompt(request: any, observation: any, steering: any = false, focus: PanelFocusContext = panelFocusContext(S.scope, PM.WS?.current, PM.PANELS || {}), context: any = S.context) {
  const workspace: any = PM.WS.current;
  const commands: any = Object.values(PM.commands || {}).map((c: any) => ({ id: c.id, label: c.label }));
  const attachedFiles: any = S.requestAttachments.slice(0, 6).map((item: any) => ({
    name: item.name, type: item.type, content: item.content ? item.content.slice(0, 30_000) : undefined,
    image: !!item.dataUrl,
  }));
  const editableSource: any = PM.Edit?.sourceCatalog?.() || observation?.state?.editableSource || {};
  const capabilities: any = PM.Capabilities?.catalog?.() || {};
  const conversation: any = S.conversation.slice(0, -1).slice(-12).map((message: any) => ({
    role: message.role, text: message.text,
    attachments: (message.attachments || []).map((item: any) => typeof item === 'string' ? item : item.name),
  }));
  return `You are the action-oriented visual editing agent inside Powermove. Turn the user's request into the strongest editable change supported by the available source operations. Your final response must be only the requested JSON object.

${AGENT_TESTING_INSTRUCTIONS}

${uiPlacementInstructions(workspace)}

${panelFocusPrompt(focus)}

${NATIVE_PANEL_DESIGN}

RULES
- ${steering ? 'This is steering for an active run. Replace the unfinished plan with one updated plan that honors the earlier request and the newest direction.' : 'This is a new run. Build one complete editable plan for the latest request.'}
- Act on clear change requests. Prefer a useful executable interpretation over explaining limitations. Use noop only when none of the available scene, section, workspace, or chrome operations can produce a meaningful result.
- Do not answer with a limitation when the requested target appears in EDITABLE SOURCE CATALOG, AVAILABLE PANELS, AVAILABLE COMMANDS, availableOperations, or the supported chrome targets. Build the executable change.
- Return 2–6 short steps that describe the actual work you will carry out. Each step must start with a verb and be specific enough to show in the interface as a to-do item.
- While working, emit concise user-visible reasoning summaries about what you are inspecting, deciding, or validating. Do not expose private chain-of-thought.
- kind=scene for changes to layers, content, motion, timing, effects, or composition settings. Each sceneEdit.commands item must be one JSON-encoded source-edit object using only availableOperations. Use stable explicit ids for new layers that later commands target. Never output JavaScript, shell commands, or whole-project JSON.
- For scene requests, inspect the live source and attached rendered frames. Preserve locked layers, hand-edited channels, and unrelated work. Set reviewTimes to the most revealing moments. Return neutral section and chromeEdit fields.
- kind=panels for direct changes to the current panel layout. panelEdit must be one JSON-encoded object shaped like {"actions":[{"type":"add|restore|hide|move|reorder|resize|resizeDock|rename|collapse|expand","panelId":"PANEL_ID","dockId":"left|center|right|EXISTING_DOCK","position":0,"size":300,"title":"New title"}]}. You may return up to 16 ordered actions. Use add for an available panel that is not present, restore for a hidden panel, move for a different dock, reorder for an exact zero-based position, resize for panel height, resizeDock for dock width, rename for its visible title, and collapse/expand for its collapsed state. Never hide or collapse viewer. Prefer a direct panels plan over rebuilding the whole workspace when the user asks to rearrange existing panels.
- kind=workspace when the user asks for a complete workspace, layout, editing environment, or a coordinated group of panels. workspaceEdit must be one JSON-encoded manifest shaped like {"name":"...","density":"compact|normal|comfy","accent":"#RRGGBB","docks":[{"id":"left|center|right","size":number,"flex":boolean,"panels":[{"id":"viewer|timeline|inspector|assets|fxbrowser|takes|notes|CUSTOM_ID","size":number,"flex":boolean}]}],"sections":[SECTION_OBJECTS]}. Include viewer, keep all panels reachable, and make every generated section control source-connected under the same rules below.
- For non-panel requests return panelEdit="{\"actions\":[]}". For non-workspace requests return workspaceEdit="{}". For non-interface requests return interfaceEdit="{}". For non-scene requests return an empty neutral sceneEdit. For non-section requests return a neutral empty section with tool="".
- kind=chrome for a supported app-interface style change. Supported targets are preview.cornerRadius with square|rounded and timeline.surfaceOrder with normal|reversed. Use operation=modify, and return a neutral empty section object.
- kind=interface for a structured Timeline redesign. interfaceEdit must be one JSON-encoded manifest shaped like {"target":"timeline","patch":{"rowHeight":22..48,"gutterWidth":160..360,"rulerHeight":20..42,"clipRadius":0..12,"keyframeSize":4..12,"showLayerNumbers":boolean,"showTypeBadges":boolean,"toolbarDensity":"compact|normal","surfaceOrder":"normal|reversed"}}. Include only fields requested or clearly useful. This edits the Timeline view over the existing source; never rewrite layers or keyframes for an interface request.
- kind=section for editable panels/controls. For a known reusable generated tool, set section.tool to an id from GENERATED TOOL CAPABILITIES and leave controls empty. The tool recipe supplies validated selection state, settings, Preview, Apply, and Undo controls. Return a neutral chromeEdit of {"target":"preview.cornerRadius","value":"square"}.
- operation=create when adding a section; operation=modify when replacing or changing an existing source surface.
- A section is a compact native Powermove panel made from slider, text, color, fill, toggle, select, button, readout, and visual curve controls.
- Every non-button control that edits project data must bind to real editable source. Use target="$selection" for the selected layer, an exact layer id from EDITABLE SOURCE CATALOG, or target="$composition" for composition paths. A visual tool control may instead use stateKey only when a validated source-action button consumes that state.
- The EDITABLE SOURCE CATALOG below is authoritative and complete for the current project. If a requested field is listed, create the working control; never claim it is unavailable. Choose each control type and range from its catalog entry.
- Do not create decorative or disconnected scene parameters. If a requested control has no source yet, prefer a scene or workspace action that creates useful editable source rather than refusing the whole request.
- Buttons may use only one of the listed command ids. Never invent commands.
- Advanced generated tools may use local settings with stateKey plus buttons whose action is a JSON-encoded safe transform action. A transform action is {"type":"transform","mode":"preview|apply","transform":{"label":"...","selector":{"scope":"selection|all|visible","types":[]},"order":"stack|reverseStack|selection|reverseSelection|start|reverseStart|name|random","edits":[{"path":"layer.from|layer.duration|layer.*|properties.*|content.*","value":EXPRESSION}]}}. Expressions are constants or objects using state, ref, aggregate, and bounded math ops from GENERATED TOOL CAPABILITIES. Prefer this declarative form when it can express the tool.
- For a Flow-style easing tool, prefer section.tool="easing-flow". To author a custom version, use a curve control with stateKey="curve", defaultValue="[0.62,0.05,0,1]", min=-1, max=2, and preset names in options. Pair it with a JSON-encoded easing button action shaped like {"type":"easing","mode":"preview|apply","scope":"selected-keyframes","curveState":"curve","defaultCurve":[0.62,0.05,0,1]}. Curve handles are visual, draggable, keyboard-accessible tool state; the action applies them to real selected keyframes through one undoable source transaction.
- When a generated tool genuinely needs loops, branching, computed layer counts, or create-many behavior, a button may instead use a JSON-encoded sandboxed script action: {"type":"script","mode":"preview|apply","label":"...","requiredTypes":["text"],"code":"JAVASCRIPT_FUNCTION_BODY"}. The function body receives a frozen PM SDK with project, composition, input, layers, selectedLayers, uid, clone, assert, emit, and typed command builders. It must return one command or an array of commands. It has no app DOM, storage, network, native bridge, or direct mutation access; its output is validated and applied atomically. Use PM.input for stateKey values. Pair an apply button with a preview button using the same code. Never attempt to escape the sandbox or access unavailable globals.
- Include every control the user explicitly requests. For open-ended requests, prefer a focused set unless the user asks for all or everything, in which case include the complete relevant catalog. Use a short title and useful one-sentence note; avoid decorative filler.
- For unused control fields, still return schema-safe neutral values: empty string/array, 0, or false.
- App chrome is a valid editable source target when it is listed above. In particular, requests to reverse the Timeline's grey surface order map to timeline.surfaceOrder=reversed.
- Keep the current interface reachable. Do not remove unrelated docks or panels. Never alter rendered composition shapes or export geometry for a chrome request.
- The active scope is a strong hint, not a restriction. Carry out dependent panel actions elsewhere when needed to satisfy the request.

VISUAL REFERENCES
${attachedFiles.some((file: any) => file.image) ? '- User-attached images come first and are direct visual references.' : '- No user image attachment was provided.'}
${S.regionImage ? '- After user images, the next attached image is an exact screenshot of the selected editor region before the Ripple overlay appeared.' : '- No editor region was selected.'}
- Remaining attached images are rendered composition frames at the times listed below.

ATTACHED FILES
${JSON.stringify(attachedFiles)}

SELECTED REGION SEMANTICS
${JSON.stringify(context)}

ACTIVE PROMPT SCOPE
${JSON.stringify(focus)}

CONVERSATION SO FAR
${JSON.stringify(conversation)}

SEMANTIC WORKSPACE MAP
${JSON.stringify(workspaceSemanticContext(workspace))}

CURRENT WORKSPACE MANIFEST
${JSON.stringify(workspace)}

AVAILABLE PANELS
${JSON.stringify(panelCatalog(workspace))}

AVAILABLE COMMANDS
${JSON.stringify(commands)}

EDITABLE SOURCE CATALOG
${JSON.stringify(editableSource)}

GENERATED TOOL CAPABILITIES
${JSON.stringify(capabilities)}

${PM.AgentHarness.promptContext(observation)}

USER REQUEST
${request}`;
}

function workspaceSemanticContext(workspace: any) {
  const custom: any = new Map((workspace?.custom || []).map((section: any) => [section.id, section]));
  return {
    id: workspace?.id || '', name: workspace?.name || '', density: workspace?.density || '',
    theme: workspace?.theme || {}, chrome: workspace?.chrome || {},
    docks: (workspace?.layout?.docks || []).map((dock: any) => ({
      id: dock.id, size: dock.size, flex: !!dock.flex,
      panels: (dock.panels || []).map((spec: any) => {
        const section: any = custom.get(spec.id);
        return {
          id: spec.id, title: section?.title || PM.PANELS?.[spec.id]?.title || spec.id,
          role: section ? 'generated editable section' : 'native editor panel',
          size: spec.size, flex: !!spec.flex,
          bindings: (section?.controls || []).map((control: any) => ({
            label: control.label, type: control.type, target: control.target || '',
            path: control.path || '', command: control.cmd || control.command || '',
          })),
        };
      }),
    })),
  };
}

function panelCatalog(workspace: any) {
  const visible: any = new Map((workspace?.layout?.docks || []).flatMap((dock: any) => (dock.panels || []).map((spec: any, index: any) => [spec.id, {
    state: 'visible', dockId: dock.id, position: index, size: spec.size || null,
    collapsed: !!spec.collapsed,
  }])));
  const hidden: any = new Map((workspace?.hiddenPanels || []).map((item: any) => [item.id, {
    state: 'hidden', dockId: item.dockId || '', position: item.index ?? 0, size: item.spec?.size || null,
  }]));
  return Object.values(PM.PANELS || {}).filter((panel: any) => panel.id !== 'toolbar').map((panel: any) => ({
    id: panel.id, title: panel.title, ...(visible.get(panel.id) || hidden.get(panel.id) || { state: 'available' }),
    canHide: panel.id !== 'viewer',
  }));
}

function controlConnection(target: any, path: any, controlType: any) {
  const catalog: any = PM.Edit?.sourceCatalog?.();
  if (target === '$composition' || target === 'composition') {
    const fallback: any = {
      'composition.name': 'text', 'composition.width': 'slider', 'composition.height': 'slider',
      'composition.fps': 'slider', 'composition.duration': 'slider', 'composition.shutter': 'slider',
      'composition.workArea.start': 'slider', 'composition.workArea.end': 'slider',
      'composition.backgroundFill': 'fill', 'composition.background': 'color',
      'composition.background.type': 'select', 'composition.background.startColor': 'color',
      'composition.background.endColor': 'color', 'composition.background.angle': 'slider',
      'composition.background.midpoint': 'slider',
    };
    const spec: any = catalog?.composition?.find((item: any) => item.path === path);
    const control: any = spec?.control || fallback[path]; if (!control) return null;
    return { ...spec, target: '$composition', path, control, connection: path.startsWith('composition.background.') ? 'Composition background' : 'Composition' };
  }
  const layer: any = target === '$selection' || target === 'selection' ? PM.firstSel() : (PM.L(target) || PM.byName(target));
  if (!layer) return null;
  const spec: any = catalog?.layers?.find((item: any) => item.id === layer.id)?.controls?.find((item: any) => item.path === path);
  if (spec) return { ...spec, target: target === '$selection' || target === 'selection' ? '$selection' : layer.id, path, connection: `Layer · ${layer.name}` };
  let control: any = controlType;
  if (path.startsWith('properties.')) {
    const channel: any = path.slice('properties.'.length);
    const prop: any = PM.findProp(layer, channel); if (!prop) return null;
    const value: any = PM.evP ? PM.evP(layer, prop, PM.time, channel) : prop.v;
    control = typeof value === 'number' ? 'slider' : typeof value === 'boolean' ? 'toggle' : /^#[0-9a-f]{6}$/i.test(value) ? 'color' : 'text';
  } else if (path.startsWith('content.')) {
    const key: any = path.slice('content.'.length);
    const current: any = layer.d?.[key];
    if (!['string', 'number', 'boolean'].includes(typeof current)) return null;
    control = typeof current === 'number' ? 'slider' : typeof current === 'boolean' ? 'toggle'
      : /^#[0-9a-f]{6}$/i.test(current) ? 'color' : 'text';
  } else if (path.startsWith('layer.')) {
    const key: any = path.slice('layer.'.length);
    if (layer.type === 'audio' && ['motionBlur', 'blend', 'parent'].includes(key)) return null;
    control = ['visible', 'locked', 'shy', 'motionBlur', 'collapsed'].includes(key) ? 'toggle'
      : ['duration', 'from'].includes(key) ? 'slider'
        : ['blend', 'parent'].includes(key) ? 'select'
          : key === 'color' ? 'color' : key === 'name' ? 'text' : '';
    if (!control) return null;
  } else return null;
  return { target: target === '$selection' || target === 'selection' ? '$selection' : layer.id, path, control, connection: `Layer · ${layer.name}` };
}

const PANEL_ACTION_TYPES: any = new Set(['add', 'restore', 'hide', 'move', 'reorder', 'resize', 'resizeDock', 'rename', 'collapse', 'expand']);

function sanitizePanelEdit(encoded: any) {
  let raw: any;
  try {
    if (typeof encoded !== 'string' || encoded.length > 80_000) return { actions: [] };
    raw = JSON.parse(encoded);
  } catch { return { actions: [] }; }
  const known: any = new Set(Object.keys(PM.PANELS || {}).filter((id: any) => id !== 'toolbar'));
  const text: any = (value: any, max: any = 100) => typeof value === 'string' ? value.trim().slice(0, max) : '';
  const actions: any = (Array.isArray(raw?.actions) ? raw.actions : []).slice(0, 16).map((value: any) => {
    const action: any = value && typeof value === 'object' ? value : {};
    const type: any = text(action.type, 20), panelId: any = text(action.panelId, 100), dockId: any = text(action.dockId, 80);
    if (!PANEL_ACTION_TYPES.has(type)) return null;
    if (type !== 'resizeDock' && (!known.has(panelId) || panelId === 'toolbar')) return null;
    if (['hide', 'collapse'].includes(type) && panelId === 'viewer') return null;
    const out: any = { type, panelId, dockId };
    if (Number.isFinite(action.position)) out.position = PM.clamp(Math.floor(action.position), 0, 20);
    if (Number.isFinite(action.size)) out.size = PM.clamp(action.size, type === 'resizeDock' ? 200 : 56, type === 'resizeDock' ? 760 : 1600);
    const title: any = text(action.title, 70); if (title) out.title = title;
    return out;
  }).filter(Boolean);
  return { actions };
}

function applyPanelEdit(workspace: any, edit: any) {
  const applied: any = [], runtime: any = [];
  const visible: any = (id: any) => findPanel(workspace, id);
  const show: any = (id: any, dockId: any = 'right') => {
    if (visible(id)) return visible(id);
    const hidden: any = (workspace.hiddenPanels || []).some((item: any) => item.id === id);
    if (hidden) restorePanel(workspace, id);
    else addPanel(workspace, id, dockId || 'right');
    return visible(id);
  };
  const place: any = (id: any, position: any) => {
    const found: any = visible(id); if (!found || !Number.isInteger(position)) return false;
    const from: any = found.dock.panels.indexOf(found.spec);
    const to: any = Math.max(0, Math.min(position, found.dock.panels.length - 1));
    if (from === to) return false;
    found.dock.panels.splice(from, 1); found.dock.panels.splice(to, 0, found.spec); return true;
  };
  for (const action of edit?.actions || []) {
    let changed: any = false;
    if (action.type === 'add') {
      const existed: any = !!visible(action.panelId);
      const found: any = show(action.panelId, action.dockId || 'right');
      if (action.dockId && found?.dock.id !== action.dockId) movePanel(workspace, action.panelId, action.dockId);
      changed = !existed || !!action.dockId;
      if (Number.isInteger(action.position)) changed = place(action.panelId, action.position) || changed;
    } else if (action.type === 'restore') {
      changed = !!show(action.panelId, action.dockId || 'right');
      if (action.dockId && visible(action.panelId)?.dock.id !== action.dockId) changed = movePanel(workspace, action.panelId, action.dockId) || changed;
      if (Number.isInteger(action.position)) changed = place(action.panelId, action.position) || changed;
    } else if (action.type === 'hide') changed = hidePanel(workspace, action.panelId);
    else if (action.type === 'move') {
      show(action.panelId, action.dockId || 'right');
      changed = action.dockId ? movePanel(workspace, action.panelId, action.dockId) : false;
      if (Number.isInteger(action.position)) changed = place(action.panelId, action.position) || changed;
    } else if (action.type === 'reorder') changed = place(action.panelId, action.position);
    else if (action.type === 'resize') {
      const found: any = show(action.panelId, action.dockId || 'right');
      if (found && Number.isFinite(action.size)) { found.spec.size = action.size; delete found.spec.flex; changed = true; }
    } else if (action.type === 'resizeDock') {
      const dock: any = (workspace.layout?.docks || []).find((item: any) => item.id === action.dockId);
      if (dock && Number.isFinite(action.size)) { dock.size = action.size; if (dock.id !== 'center') delete dock.flex; changed = true; }
    } else if (action.type === 'rename') {
      const found: any = show(action.panelId, action.dockId || 'right');
      if (found && action.title) { found.spec.title = action.title; changed = true; }
    } else runtime.push(action);
    if (changed) applied.push(action);
  }
  return { applied, runtime };
}

function sanitizePlan(raw: any, context: any, request: any = '') {
  const operation: any = ['create', 'modify', 'noop'].includes(raw?.operation) ? raw.operation : 'noop';
  const chromeTarget: any = ['preview.cornerRadius', 'timeline.surfaceOrder'].includes(raw?.chromeEdit?.target) ? raw.chromeEdit.target : '';
  const allowedChromeValues: any = chromeTarget === 'preview.cornerRadius' ? ['square', 'rounded']
    : chromeTarget === 'timeline.surfaceOrder' ? ['normal', 'reversed'] : [];
  const chromeValue: any = allowedChromeValues.includes(raw?.chromeEdit?.value) ? raw.chromeEdit.value : '';
  const interfaceEdit: any = PM.WS?.sanitizeInterfaceEdit?.(raw?.interfaceEdit) || null;
  const requestedKind: any = raw?.kind;
  const panelEdit: any = sanitizePanelEdit(raw?.panelEdit);
  const kind: any = requestedKind === 'scene' ? 'scene'
    : requestedKind === 'workspace' ? 'workspace'
    : requestedKind === 'panels' ? 'panels'
    : requestedKind === 'interface' && interfaceEdit ? 'interface'
    : requestedKind === 'chrome' && chromeTarget && chromeValue ? 'chrome' : 'section';
  const cleanText: any = (v: any, fallback: any = '', max: any = 100) => typeof v === 'string' && v.trim() ? v.trim().slice(0, max) : fallback;
  const source: any = raw?.section && typeof raw.section === 'object' ? raw.section : {};
  const recipe: any = PM.Capabilities?.panelRecipe?.(cleanText(source.tool, '', 80)) || null;
  const sectionSource: any = recipe ? {
    ...recipe,
    id: cleanText(source.id, recipe.id, 80), title: cleanText(source.title, recipe.title, 70),
    size: Number(source.size) || recipe.size, note: cleanText(source.note, recipe.note, 240),
  } : source;
  const steps: any = (Array.isArray(raw?.steps) ? raw.steps : []).filter((step: any) => typeof step === 'string' && step.trim())
    .map((step: any) => step.trim().slice(0, 100)).slice(0, 6);
  const baseId: any = slug(cleanText(sectionSource.id, cleanText(sectionSource.title, 'Generated section')));
  const controls: any = (Array.isArray(sectionSource.controls) ? sectionSource.controls : []).slice(0, 64).map((control: any, index: any) => {
    const c: any = control && typeof control === 'object' ? control : {};
    let type: any = ['slider', 'text', 'color', 'fill', 'toggle', 'select', 'button', 'readout', 'curve'].includes(c.type) ? c.type : 'slider';
    const label: any = cleanText(c.label, `Control ${index + 1}`, 60);
    const out: any = { type, label };
    if (type === 'readout') {
      out.source = ['selection.summary', 'selection.count', 'keyframes.summary', 'keyframes.count'].includes(c.source) ? c.source : 'selection.summary';
      out.connection = out.source.startsWith('keyframes.') ? 'Live keyframes' : 'Live selection';
      return out;
    }
    if (type === 'button') {
      if (c.command && PM.commands?.[c.command]) out.cmd = c.command;
      else if (c.cmd && PM.commands?.[c.cmd]) out.cmd = c.cmd;
      const action: any = PM.Capabilities?.sanitizeControlAction?.(c.action);
      if (action) out.action = action;
      out.primary = c.primary === true;
      out.connection = action ? 'Source action' : 'Command';
      return out;
    }
    const localKey: any = cleanText(c.stateKey, '', 80);
    if (localKey && /^[a-z][a-z0-9_.-]{0,79}$/i.test(localKey)) {
      out.stateKey = localKey; out.connection = 'Tool setting';
      const sourceValue: any = c.def !== undefined ? c.def : c.defaultValue;
      if (type === 'curve') {
        out.def = PM.Capabilities?.sanitizeCurve?.(sourceValue, [.62, .05, 0, 1]) || [.62, .05, 0, 1];
        out.minY = PM.clamp(Number.isFinite(c.minY) ? c.minY : Number.isFinite(c.min) ? c.min : -1, -4, 0);
        out.maxY = PM.clamp(Number.isFinite(c.maxY) ? c.maxY : Number.isFinite(c.max) ? c.max : 2, 1, 4);
        out.presets = (Array.isArray(c.presets) ? c.presets : Array.isArray(c.options) ? c.options : [])
          .filter((name: any) => typeof name === 'string' && PM.Ease?.PRESETS?.[name]).slice(0, 16);
      } else if (type === 'text') out.def = sourceValue == null ? '' : String(sourceValue).slice(0, 500);
      else if (type === 'color') out.def = /^#[0-9a-f]{6}$/i.test(sourceValue) ? sourceValue.toUpperCase() : '#FF6B1A';
      else if (type === 'toggle') out.def = !!sourceValue;
      else if (type === 'select') {
        out.options = (Array.isArray(c.options) ? c.options : [])
          .filter((v: any) => typeof v === 'string' || (v && typeof v === 'object' && ['string', 'number', 'boolean'].includes(typeof v.v) && typeof v.label === 'string'))
          .map((v: any) => typeof v === 'string' ? v : ({ v: v.v, label: v.label.slice(0, 80) })).slice(0, 80);
        const values: any = out.options.map((option: any) => typeof option === 'string' ? option : option.v);
        out.def = values.includes(sourceValue) ? sourceValue : (values[0] ?? 'Default');
      } else {
        out.min = Number.isFinite(c.min) ? c.min : 0; out.max = Number.isFinite(c.max) ? c.max : 100;
        if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
        out.step = Number.isFinite(c.step) && c.step > 0 ? c.step : Math.max((out.max - out.min) / 100, .01);
        out.unit = cleanText(c.unit, '', 12);
        out.def = Number.isFinite(sourceValue) ? PM.clamp(sourceValue, out.min, out.max) : out.min;
      }
      return out;
    }
    const target: any = cleanText(c.target, '', 120), path: any = cleanText(c.path, '', 120);
    const connection: any = controlConnection(target, path, type);
    if (!connection) return null;
    type = connection.control || type; out.type = type;
    Object.assign(out, connection);
    const sourceValue: any = connection.value !== undefined ? connection.value : c.defaultValue;
    if (type === 'text') out.def = sourceValue == null ? '' : String(sourceValue);
    else if (type === 'color') out.def = /^#[0-9a-f]{6}$/i.test(sourceValue) ? sourceValue : '#FF6B1A';
    else if (type === 'fill') out.def = sourceValue && typeof sourceValue === 'object' ? sourceValue : null;
    else if (type === 'toggle') out.def = !!sourceValue;
    else if (type === 'select') {
      out.options = (Array.isArray(connection.options) ? connection.options : Array.isArray(c.options) ? c.options : [])
        .filter((v: any) => typeof v === 'string' || (v && typeof v === 'object' && 'v' in v && typeof v.label === 'string')).slice(0, 160);
      const values: any = out.options.map((option: any) => typeof option === 'string' ? option : option.v);
      out.def = values.includes(sourceValue) ? sourceValue : (values[0] ?? 'Default');
      if (!out.options.length) out.options = [out.def];
    } else {
      out.min = Number.isFinite(connection.min) ? connection.min : Number.isFinite(c.min) ? c.min : 0;
      out.max = Number.isFinite(connection.max) ? connection.max : Number.isFinite(c.max) ? c.max : 100;
      if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
      out.step = Number.isFinite(connection.step) && connection.step > 0 ? connection.step : Number.isFinite(c.step) && c.step > 0 ? c.step : Math.max((out.max - out.min) / 100, .01);
      out.unit = connection.unit || '';
      out.def = Number.isFinite(sourceValue) ? PM.clamp(sourceValue, out.min, out.max) : out.min;
    }
    return out;
  }).filter((c: any) => c && (c.type !== 'button' || c.cmd || c.action));
  const state: any = { ...(recipe?.state || {}) };
  controls.filter((control: any) => control.stateKey).forEach((control: any) => {
    if (state[control.stateKey] === undefined) state[control.stateKey] = control.def;
  });
  return {
    kind,
    operation,
    targetPanelId: cleanText(raw?.targetPanelId, context?.targetPanelId || '', 100),
    dockId: cleanText(raw?.dockId, '', 100),
    placement: ['before', 'after', 'replace'].includes(raw?.placement) ? raw.placement : (operation === 'modify' ? 'replace' : 'after'),
    message: cleanText(raw?.message, operation === 'modify' ? 'The redesigned section is ready.' : 'The new section is ready.', 220),
    steps: steps.length ? steps : ['Prepare the editable change', 'Review the visible result'],
    chromeEdit: kind === 'chrome' ? { target: chromeTarget, value: chromeValue } : null,
    interfaceEdit: kind === 'interface' ? interfaceEdit : null,
    section: {
      id: baseId, title: cleanText(sectionSource.title, 'Generated section', 70),
      icon: typeof sectionSource.icon === 'string' && PM.ICONS?.[sectionSource.icon] ? sectionSource.icon : undefined,
      size: PM.clamp(Number(sectionSource.size) || 220, 120, 700), note: cleanText(sectionSource.note, '', 240),
      tool: recipe?.id || '', state, controls,
    },
    sceneEdit: kind === 'scene' ? PM.AgentHarness.sanitizeProposal(raw?.sceneEdit, request) : null,
    workspaceEdit: kind === 'workspace' ? sanitizeWorkspaceEdit(raw?.workspaceEdit, context) : null,
    panelEdit: kind === 'panels' ? panelEdit : { actions: [] },
  };
}

function sanitizeWorkspaceEdit(encoded: any, context: any) {
  let raw: any;
  try {
    if (typeof encoded !== 'string' || encoded.length > 120_000) return null;
    raw = JSON.parse(encoded);
  } catch { return null; }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const clean: any = (value: any, fallback: any, max: any = 80) => typeof value === 'string' && value.trim() ? value.trim().slice(0, max) : fallback;
  const sections: any = (Array.isArray(raw.sections) ? raw.sections : []).slice(0, 6).map((section: any) => {
    const plan: any = sanitizePlan({
      kind: 'section', operation: 'create', targetPanelId: '', dockId: '', placement: 'after', message: '',
      chromeEdit: { target: 'preview.cornerRadius', value: 'square' }, section,
      sceneEdit: { label: '', summary: '', commands: [], reviewTimes: [] }, workspaceEdit: '{}',
    }, context);
    return plan.section.controls.length ? plan.section : null;
  }).filter(Boolean);
  const customIds: any = new Set(sections.map((section: any) => section.id));
  const known: any = new Set([...Object.keys(PM.PANELS || {}), ...customIds]);
  const usedDockIds: any = new Set();
  const docks: any = (Array.isArray(raw.docks) ? raw.docks : []).slice(0, 4).map((dock: any, index: any) => {
    const baseId: any = slug(clean(dock?.id, index === 0 ? 'center' : `dock-${index + 1}`));
    let id: any = baseId, suffix: any = 2; while (usedDockIds.has(id)) id = `${baseId}-${suffix++}`;
    usedDockIds.add(id);
    const panels: any = (Array.isArray(dock?.panels) ? dock.panels : []).slice(0, 10).map((panel: any) => {
      const spec: any = typeof panel === 'string' ? { id: panel } : panel;
      const panelId: any = clean(spec?.id, '', 80);
      if (!known.has(panelId)) return null;
      const out: any = { id: panelId };
      if (Number.isFinite(spec?.size)) out.size = PM.clamp(spec.size, 100, 900);
      if (spec?.flex === true) out.flex = true;
      return out;
    }).filter(Boolean);
    return { id, size: Number.isFinite(dock?.size) ? PM.clamp(dock.size, 180, 700) : undefined, flex: dock?.flex === true, panels };
  }).filter((dock: any) => dock.panels.length);
  if (!docks.some((dock: any) => dock.panels.some((panel: any) => panel.id === 'viewer'))) {
    let center: any = docks.find((dock: any) => dock.id === 'center');
    if (!center) {
      center = { id: 'center', flex: true, panels: [] };
      if (docks.length >= 4) docks[docks.length - 1] = center;
      else docks.push(center);
    }
    center.panels.unshift({ id: 'viewer', flex: true });
  }
  const placed: any = new Set(docks.flatMap((dock: any) => dock.panels.map((panel: any) => panel.id)));
  const unplaced: any = sections.filter((section: any) => !placed.has(section.id));
  if (unplaced.length) {
    let side: any = docks.find((dock: any) => dock.id === 'left' || dock.id === 'right');
    if (!side) side = docks.find((dock: any) => dock.id !== 'center');
    if (!side) {
      side = { id: 'right', size: 300, flex: false, panels: [] };
      if (docks.length >= 4) docks[docks.length - 1] = side;
      else docks.push(side);
    }
    unplaced.forEach((section: any) => side.panels.push({ id: section.id, size: section.size }));
  }
  return {
    name: clean(raw.name, 'Generated workspace'),
    density: ['compact', 'normal', 'comfy'].includes(raw.density) ? raw.density : 'normal',
    accent: /^#[0-9a-f]{6}$/i.test(raw.accent) ? raw.accent.toUpperCase() : '#FF6B1A',
    docks: docks.slice(0, 4), sections,
  };
}

/* Apply only explicitly supported app-chrome edits to the versioned workspace
   manifest. This is a pure mutation seam used inside WS.mutate, never a CSS or
   project-canvas write from model output. */
function applyChromeEdit(workspace: any, edit: any) {
  if (!workspace || !edit) return false;
  workspace.chrome = workspace.chrome && typeof workspace.chrome === 'object' ? workspace.chrome : {};
  if (edit.target === 'preview.cornerRadius' && ['square', 'rounded'].includes(edit.value)) {
    workspace.chrome.previewCornerRadius = edit.value; return true;
  }
  if (edit.target === 'timeline.surfaceOrder' && ['normal', 'reversed'].includes(edit.value)) {
    workspace.chrome.timelineSurfaceOrder = edit.value; return true;
  }
  return false;
}

function slug(value: any) {
  const out: any = String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 48);
  return out || 'generated-section';
}

function showPreview() {
  S.phase = 'conversation';
  PM.AgentUI?.update({ focusComposer: true });
}

function setStepProgress(index: any) {
  if (!S.steps.length) return;
  const active: any = Math.max(0, Math.min(index, S.steps.length - 1));
  S.steps.forEach((step: any, i: any) => { step.status = i < active ? 'complete' : i === active ? 'active' : 'pending'; });
}

function updateSteps(titles: any, active: any = -1) {
  S.steps = (Array.isArray(titles) ? titles : []).map((title: any, index: any) => ({
    id: `${S.requestToken}-${index}`, title: String(title || '').trim(),
    status: index < active ? 'complete' : index === active ? 'active' : 'pending',
  })).filter((step: any) => step.title);
}

function finishSteps() {
  S.steps.forEach((step: any) => { step.status = 'complete'; });
}

function locatePanel(workspace: any, panelId: any) {
  for (const dock of workspace.layout?.docks || []) {
    const index: any = (dock.panels || []).findIndex((p: any) => p.id === panelId);
    if (index >= 0) return { dock, index };
  }
  return null;
}

function uniqueSectionId(workspace: any, requested: any, keepId: any = '') {
  const occupied: any = new Set([
    ...Object.keys(PM.PANELS || {}),
    ...(workspace.custom || []).map((p: any) => p.id),
  ]);
  if (requested === keepId || !occupied.has(requested)) return requested;
  let n: any = 2; while (occupied.has(`${requested}-${n}`)) n++;
  return `${requested}-${n}`;
}

function finishWorkspaceRun(checkpoint: any, summary: any, actions: any = []) {
  const after: any = PM.WS.historySnapshot();
  const historyId: any = PM.hist.external(
    `Agent · ${summary}`,
    () => PM.WS.restoreHistorySnapshot(checkpoint),
    () => PM.WS.restoreHistorySnapshot(after),
  );
  finishSteps();
  S.activity = ''; S.plan = null;
  S.panelRun = { checkpoint, after, historyId, actions, summary };
  S.conversation.push({ role: 'assistant', text: `${summary}. This change is in Undo history.` });
  /* Deliberate Svelte deviation from HEAD: result transitions restore the
     persistent composer's focus instead of relying on a DOM rebuild. */
  S.phase = 'result'; PM.AgentUI?.update({ focusComposer: true });
}

async function applyPlan() {
  if (!S.plan || !['conversation', 'preview'].includes(S.phase)) return;
  const plan: any = S.plan;
  if (plan.kind === 'scene') {
    await applyScenePlan(plan);
    return;
  }
  if (plan.kind === 'panels') {
    await applyPanelPlan(plan);
    return;
  }
  S.phase = 'applying'; S.activity = 'Applying the editable change…'; setStepProgress(0); PM.AgentUI?.update();
  await new Promise((resolve: any) => window.requestAnimationFrame(resolve));
  const checkpoint: any = PM.WS.historySnapshot();
  if (plan.kind === 'workspace') {
    const manifest: any = plan.workspaceEdit;
    const created: any = PM.WS.create({
      name: manifest.name, base: PM.WS.current.id, density: manifest.density,
      theme: { ...(PM.WS.current.theme || {}), accent: manifest.accent },
      custom: manifest.sections,
      layout: { docks: manifest.docks },
    });
    PM.toast(`Created workspace · ${created.name}`);
    finishWorkspaceRun(checkpoint, `Created ${created.name}`);
    return;
  }
  if (plan.kind === 'chrome') {
    let changed: any = false;
    PM.WS.mutate((workspace: any) => { changed = applyChromeEdit(workspace, plan.chromeEdit); });
    if (changed) PM.toast('Updated preview corner style');
    if (changed) finishWorkspaceRun(checkpoint, 'Applied the interface edit');
    else {
      finishSteps(); S.plan = null; S.activity = ''; S.phase = 'conversation';
      S.conversation.push({ role: 'assistant', text: 'That interface setting was already in place.' }); PM.AgentUI?.update({ focusComposer: true });
    }
    return;
  }
  if (plan.kind === 'interface') {
    let changed: any = false;
    PM.WS.mutate((workspace: any) => { changed = PM.WS.applyInterfaceEdit(workspace, plan.interfaceEdit); });
    if (changed) PM.toast('Updated Timeline design');
    if (changed) finishWorkspaceRun(checkpoint, 'Applied the Timeline redesign');
    else {
      finishSteps(); S.plan = null; S.activity = ''; S.phase = 'conversation';
      S.conversation.push({ role: 'assistant', text: 'Those Timeline settings were already in place.' }); PM.AgentUI?.update({ focusComposer: true });
    }
    return;
  }
  const current: any = PM.WS.current;
  const target: any = locatePanel(current, plan.targetPanelId || S.context.targetPanelId);
  const targetIsCustom: any = (current.custom || []).some((p: any) => p.id === target?.dock?.panels?.[target.index]?.id);
  const replacing: any = plan.operation === 'modify' && !!target;
  const desiredId: any = replacing && targetIsCustom ? target.dock.panels[target.index].id : plan.section.id;
  const sectionId: any = uniqueSectionId(current, desiredId, replacing && targetIsCustom ? desiredId : '');
  /* A rebuilt custom panel must not retain the old definition/cache. Remove its
     live instance before WS.activate() reconstructs the edited workspace. */
  if (replacing && targetIsCustom) delete PM.panelInst[desiredId];
  PM.WS.mutate((workspace: any) => {
    workspace.custom = Array.isArray(workspace.custom) ? workspace.custom : [];
    const oldId: any = target && target.dock.panels[target.index].id;
    if (replacing && targetIsCustom) workspace.custom = workspace.custom.filter((p: any) => p.id !== oldId);
    workspace.custom.push({ ...plan.section, id: sectionId });

    const liveTarget: any = locatePanel(workspace, oldId || '');
    let dock: any = liveTarget?.dock || (workspace.layout.docks || []).find((d: any) => d.id === plan.dockId)
      || (workspace.layout.docks || []).find((d: any) => d.id === 'center') || workspace.layout.docks[0];
    if (!dock) {
      dock = { id: 'center', flex: true, panels: [] };
      workspace.layout = workspace.layout || {}; workspace.layout.docks = [dock];
    }
    const spec: any = { id: sectionId, size: plan.section.size };
    if (replacing && liveTarget) liveTarget.dock.panels.splice(liveTarget.index, 1, spec);
    else {
      const index: any = liveTarget ? liveTarget.index + (plan.placement === 'before' ? 0 : 1) : dock.panels.length;
      dock.panels.splice(index, 0, spec);
    }
  });
  PM.toast((replacing ? 'Redesigned ' : 'Added ') + plan.section.title);
  finishWorkspaceRun(checkpoint, `${replacing ? 'Redesigned' : 'Added'} ${plan.section.title}`);
}

async function applyPanelPlan(plan: any) {
  const checkpoint: any = PM.WS.historySnapshot();
  S.phase = 'applying'; S.activity = 'Applying panel changes…'; setStepProgress(0); PM.AgentUI?.update();
  let result: any = { applied: [], runtime: [] };
  try {
    PM.WS.mutate((workspace: any) => { result = applyPanelEdit(workspace, plan.panelEdit); });
    await new Promise((resolve: any) => window.requestAnimationFrame(() => window.requestAnimationFrame(resolve)));
    for (const action of result.runtime) {
      let changed: any = false;
      if (action.type === 'collapse') changed = PM.Layout.setCollapsed(action.panelId, true);
      else if (action.type === 'expand') changed = PM.Layout.setCollapsed(action.panelId, false);
      if (changed) result.applied.push(action);
    }
    if (!result.applied.length) throw new Error('Those panels were already arranged that way');
    const summary: any = `${result.applied.length} panel change${result.applied.length === 1 ? '' : 's'} applied`;
    finishWorkspaceRun(checkpoint, summary, result.applied);
    PM.toast(summary);
  } catch (error: any) {
    PM.WS.restoreHistorySnapshot(checkpoint);
    const current: any = S.steps.find((step: any) => step.status === 'active'); if (current) current.status = 'error';
    S.activity = ''; S.plan = null; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', error: true, text: `${String(error.message || error).slice(0, 180)}. Nothing was changed.` });
    PM.AgentUI?.update({ focusComposer: true });
  }
}

function undoPanelRun() {
  if (!S.panelRun) return;
  const checkpoint: any = S.panelRun.checkpoint;
  if (!PM.hist.undoIfTop(S.panelRun.historyId)) {
    const current: any = PM.WS.historySnapshot();
    PM.WS.restoreHistorySnapshot(checkpoint);
    PM.hist.external('Restore before agent change',
      () => PM.WS.restoreHistorySnapshot(current),
      () => PM.WS.restoreHistorySnapshot(checkpoint));
  }
  S.panelRun = null; S.phase = 'conversation';
  S.conversation.push({ role: 'assistant', text: 'I restored the previous panel layout.' });
  PM.AgentUI?.update({ focusComposer: true }); PM.toast('Panel changes undone');
}

function keepPanelRun() {
  if (!S.panelRun) return;
  S.panelRun = null; S.phase = 'conversation';
  S.conversation.push({ role: 'assistant', text: 'Kept the agent change. Command-Z can still reverse it.' });
  PM.AgentUI?.update({ focusComposer: true });
}

async function applyScenePlan(plan: any) {
  S.phase = 'applying'; S.plan = null; S.activity = 'Applying structured source edit…';
  setStepProgress(0);
  PM.AgentUI?.update();
  try {
    let progressIndex: any = 0;
    const run: any = await PM.AgentHarness.execute(S.requestText, plan.sceneEdit, (value: any) => {
      setStepProgress(Math.min(progressIndex++, S.steps.length - 1));
      S.activity = value; PM.AgentUI?.update();
    });
    finishSteps();
    S.activity = ''; S.run = run;
    showSceneResult(run);
  } catch (error: any) {
    const current: any = S.steps.find((step: any) => step.status === 'active'); if (current) current.status = 'error';
    S.activity = ''; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', error: true, text: `${String(error.message || error).slice(0, 180)} Nothing was applied.` });
    PM.AgentUI?.update({ focusComposer: true });
  }
}

function showSceneResult(run: any) {
  S.phase = 'result'; S.run = run;
  /* Deliberate Svelte deviation from HEAD: result transitions restore the
     persistent composer's focus instead of relying on a DOM rebuild. */
  PM.AgentUI?.update({ focusComposer: true });
}

function keepSceneRun() {
  if (!S.run) return;
  PM.toast('Kept agent change');
  S.conversation.push({ role: 'assistant', text: S.run.autonomous
    ? (S.run.changed ? 'Kept the autonomous result. Command-Z can still reverse its Powermove changes.' : 'Closed the completed autonomous run. Its artifacts remain available in the project workspace.')
    : `Kept ${S.run.applied.length} editable source changes. Command-Z can still reverse the complete run.` });
  S.run = null; S.phase = 'conversation'; PM.AgentUI?.update({ focusComposer: true });
}

async function undoSceneRun() {
  if (!S.run) return;
  let extensionRestored: any = !S.run.extensionChangeSetId;
  let extensionError: any = '';
  if (S.run.extensionChangeSetId) {
    try {
      await (window as any).powermove.codex.restoreChangeSet({
        projectId: S.run.projectId,
        changeSetId: S.run.extensionChangeSetId
      });
      extensionRestored = true;
    } catch (error: any) {
      extensionError = String(error?.message || error);
    }
  }
  const projectRestored: any = !S.run.changed || PM.AgentHarness.rollback(S.run.checkpoint);
  const restored: any = extensionRestored && projectRestored;
  PM.toast(restored ? 'Agent change undone' : 'Could not fully restore the agent change');
  S.conversation.push({
    role: 'assistant',
    text: restored
      ? 'I restored the project checkpoint and the previous app-extension version. Tell me what to try differently.'
      : `I could not fully restore that run.${extensionError ? ` ${extensionError}` : ''}`
  });
  S.run = null; S.phase = 'conversation'; PM.AgentUI?.update({ focusComposer: true });
}

function onKey(event: any) {
  if (!S.active) return;
  if (event.target?.closest?.('.panel-focus-popup')) return;
  if (event.key === 'Escape') {
    event.preventDefault(); event.stopPropagation();
    if (S.phase === 'applying') return;
    if (S.phase === 'result') return undoSceneRun();
    cancel();
  }
}

function dismissOverlay(preserveContext: any) {
  if (!S.active) return;
  S.active = false;
  window.removeEventListener('keydown', onKey, true);
  window.cancelAnimationFrame(S.hintFrame); S.hintFrame = 0; S.hintPoint = null;
  if (S.renderStop) S.renderStop();
  S.attachmentUI?.dispose(); S.attachmentUI = null;
  if (S.focusPicker) { void unmount(S.focusPicker); S.focusPicker = null; }
  S.root?.remove();
  Object.assign(S, {
    root: null, ink: null, path: null, shadePath: null, hint: null, card: null,
    outline: null, renderStop: null, points: [], sceneFrame: null,
  });
  if (!preserveContext) {
    S.region = null; S.context = null; S.regionImage = null;
    S.phase = S.conversation.length || S.plan || S.run ? 'conversation' : 'idle';
  }
  window.setTimeout(refreshSceneCache, 80);
}

function cancel() {
  if (!S.active) return;
  dismissOverlay(false);
  PM.AgentUI?.update();
}

function startRipple(host: any, origin: any, sceneBitmap: any) {
  if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
    sceneBitmap?.close?.();
    host.dataset.renderer = 'reduced-motion';
    return () => {};
  }
  let stopped: any = false, failed: any = false, fallbackTimer: any = 0, component: any = null;
  const fallback: any = (reason: any) => {
    if (stopped || failed) return;
    failed = true;
    if (component) { void unmount(component); component = null; }
    else sceneBitmap?.close?.();
    host.dataset.renderer = 'css-fallback';
    host.style.background = `radial-gradient(circle at ${origin.x}px ${origin.y}px,rgba(255,107,26,.34),rgba(76,35,88,.16) 30%,rgba(8,8,12,.05) 64%,transparent 78%)`;
    host.style.opacity = '1';
    host.style.transition = 'opacity var(--dur-3) var(--ease)';
    /* The selection workflow remains visibly active even without WebGPU. The
       old fallback faded to zero, which made a recoverable renderer failure
       look exactly like a dead feature. */
    fallbackTimer = window.setTimeout(() => { if (!stopped) host.style.opacity = '.24'; }, 900);
    if (reason) window.console.warn('Motion GPU ripple unavailable; using visual fallback', reason);
  };
  host.dataset.renderer = 'motion-gpu-initializing';
  try {
    component = mount(RippleCanvas, { target: host, props: {
      origin,
      sceneBitmap,
      onError: (report: any) => fallback(new Error(report?.rawMessage || report?.message || 'Motion GPU failed')),
      onFirstFrame: () => { if (!stopped && !failed) host.dataset.renderer = 'motion-gpu'; },
      onSettled: () => { if (!stopped && !failed) host.dataset.renderer = 'motion-gpu-settled'; },
    } });
  } catch (error: any) { fallback(error); }

  return () => {
    stopped = true; window.clearTimeout(fallbackTimer);
    if (component) { void unmount(component); component = null; }
  };
}
}
