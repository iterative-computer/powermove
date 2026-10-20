import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './spatial';
import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
import { AGENT_MODELS } from '../../../../shared/agent-models';

function spatialRegistry(provider = 'chatgpt', stored: Record<string, unknown> = {}): PMRegistry {
  const PM: PMRegistry = {
    store: { get: (key: string, fallback: any) => key === 'agentProvider' ? provider : Object.hasOwn(stored, key) ? stored[key] : fallback, set: vi.fn() },
    h() {},
    uid: () => 'id',
    clamp: (value: any, min: any, max: any) => Math.max(min, Math.min(max, value)),
    round: (value: any, places = 0) => Number(Number(value).toFixed(places)),
    snapF: (time: any, fps: any) => Math.round(time * fps) / fps,
    TYPE_META: { text: {} },
    Ease: {
      PRESETS: {
        linear: [0, 0, 1, 1],
        easeInOut: [.42, 0, .58, 1],
        power: [.62, .05, 0, 1],
        backOut: [.34, 1.56, .64, 1],
      },
      bezier: () => (value: any) => value,
      nameOf: () => 'custom',
    },
    commands: { fitView: { id: 'fitView', label: 'Fit view' } },
    PANELS: {
      viewer: { title: 'Composition' },
      timeline: { title: 'Timeline' },
      inspector: { title: 'Inspector' },
      assets: { title: 'Project' },
    },
    registerPanel(id: any, definition: any) { this.PANELS[id] = definition; },
    firstSel: () => null,
    L: () => null,
    byName: () => null,
    findProp: () => null,
    AgentHarness: {
      sceneSchema: () => ({ type: 'object' }),
      sanitizeProposal: (value: any) => ({ commands: value?.commands || [] }),
      promptContext: () => '',
      describeCommand: (command: any) => command.type,
    },
  };
  install(PM);
  return PM;
}

describe('legacy spatial assistant install', () => {
  it.each(['chatgpt', 'claude', 'compatible'])('discovers only the active provider at startup (%s), then refreshes on switching', async provider => {
    const models = { chatgpt: vi.fn(async () => []), claude: vi.fn(async () => []) };
    const changed: Record<string, (status: any) => void> = {};
    installBridgeForTests({
      chatgpt: { models: models.chatgpt, onChanged: (callback: any) => { changed.chatgpt = callback; } },
      claude: { models: models.claude, onChanged: (callback: any) => { changed.claude = callback; } },
    } as any);
    try {
      const PM = spatialRegistry(provider);
      expect(models.chatgpt).toHaveBeenCalledTimes(provider === 'chatgpt' ? 1 : 0);
      expect(models.claude).toHaveBeenCalledTimes(provider === 'claude' ? 1 : 0);
      const next = provider === 'claude' ? 'chatgpt' : 'claude';
      changed[next]!({ state: 'connected' });
      expect(models[next]).not.toHaveBeenCalled();
      PM.AgentUI.setProvider(next);
      expect(models[next]).toHaveBeenCalledTimes(1);
      changed[next]!({ state: 'connected' });
      expect(models[next]).toHaveBeenCalledTimes(2);
      changed[next]!({ state: 'disconnected' });
      expect(models[next]).toHaveBeenCalledTimes(2);
      await Promise.resolve();
    } finally { resetBridgeForTests(); }
  });
  it.each([['claude', 'chatgpt'], ['chatgpt', 'claude']])('discovers the saved active thread provider %s → %s exactly once at startup', async (storedProvider, threadProvider) => {
    const models = { chatgpt: vi.fn(async () => []), claude: vi.fn(async () => []) };
    const threadModel = threadProvider === 'chatgpt' ? 'saved-startup/Codex' : 'sonnet';
    const previousModels = [...AGENT_MODELS.chatgpt];
    installBridgeForTests({ chatgpt: { models: models.chatgpt }, claude: { models: models.claude } } as any);
    let PM: PMRegistry | undefined;
    try {
      PM = spatialRegistry(storedProvider, {
        'agentThreads.': { version: 1, activeId: 'saved', threads: [
          { id: 'saved', conversation: [], composerDraft: '', provider: threadProvider, model: threadModel }
        ] }
      });
      expect(PM.AgentUI.state.provider).toBe(threadProvider);
      expect(PM.AgentUI.state.model).toBe(threadModel);
      expect(models.chatgpt).toHaveBeenCalledTimes(threadProvider === 'chatgpt' ? 1 : 0);
      expect(models.claude).toHaveBeenCalledTimes(threadProvider === 'claude' ? 1 : 0);
      await Promise.resolve();
    } finally {
      PM?.AgentUI.flushThreads();
      AGENT_MODELS.chatgpt = previousModels;
      resetBridgeForTests();
    }
  });
  it.each(['claude', 'compatible'])('restores a saved custom Codex thread before discovery from %s', async provider => {
    const customModel = `saved-${provider}/Codex:fast`;
    let resolveModels!: (models: any[]) => void;
    const models = vi.fn(() => new Promise<any[]>(resolve => { resolveModels = resolve; }));
    const previousModels = [...AGENT_MODELS.chatgpt];
    installBridgeForTests({ chatgpt: { models }, claude: { models: async () => [] } } as any);
    let PM: PMRegistry | undefined;
    try {
      PM = spatialRegistry(provider, {
        'agentThreads.': { version: 1, activeId: 'active', threads: [
          { id: 'active', conversation: [], composerDraft: '', provider, model: provider === 'claude' ? 'sonnet' : 'configured' },
          { id: 'saved-codex', conversation: [], composerDraft: '', provider: 'chatgpt', model: customModel, reasoningEffort: 'high' }
        ] }
      });
      expect(models).not.toHaveBeenCalled();
      PM.AgentUI.switchThread('saved-codex');
      expect(PM.AgentUI.state.provider).toBe('chatgpt');
      expect(PM.AgentUI.state.model).toBe(customModel);
      expect(models).toHaveBeenCalledOnce();
      resolveModels([{ id: customModel, label: 'Saved custom Codex', reasoningEfforts: ['high'] }]);
      await Promise.resolve();
      expect(PM.AgentUI.state.provider).toBe('chatgpt');
      expect(PM.AgentUI.state.model).toBe(customModel);
    } finally {
      PM?.AgentUI.flushThreads();
      AGENT_MODELS.chatgpt = previousModels;
      resetBridgeForTests();
    }
  });

  it('keeps a newer selected Codex model when saved-thread discovery resolves later', async () => {
    const customModel = 'saved/delayed-codex';
    let resolveModels!: (models: any[]) => void;
    const previousModels = [...AGENT_MODELS.chatgpt];
    installBridgeForTests({ chatgpt: { models: () => new Promise<any[]>(resolve => { resolveModels = resolve; }) } } as any);
    let PM: PMRegistry | undefined;
    try {
      PM = spatialRegistry('compatible', {
        'agentThreads.': { version: 1, activeId: 'active', threads: [
          { id: 'active', conversation: [], composerDraft: '', provider: 'compatible', model: 'configured' },
          { id: 'saved-codex', conversation: [], composerDraft: '', provider: 'chatgpt', model: customModel }
        ] }
      });
      PM.AgentUI.switchThread('saved-codex');
      expect(PM.AgentUI.state.model).toBe(customModel);
      PM.AgentUI.setModel('gpt-6-luna', 'high');
      resolveModels([{ id: customModel, label: 'Saved custom Codex', reasoningEfforts: ['high'] }]);
      await Promise.resolve();
      expect(PM.AgentUI.state.model).toBe('gpt-6-luna');
    } finally {
      PM?.AgentUI.flushThreads();
      AGENT_MODELS.chatgpt = previousModels;
      resetBridgeForTests();
    }
  });
  it.each(['bad model', 'x'.repeat(129)])('does not restore an invalid saved Codex model (%s)', async model => {
    const models = vi.fn(async () => []), previousModels = [...AGENT_MODELS.chatgpt];
    installBridgeForTests({ chatgpt: { models } } as any);
    let PM: PMRegistry | undefined;
    try {
      PM = spatialRegistry('compatible', {
        'agentThreads.': { version: 1, activeId: 'active', threads: [
          { id: 'active', conversation: [], composerDraft: '', provider: 'compatible', model: 'configured' },
          { id: 'invalid-codex', conversation: [], composerDraft: '', provider: 'chatgpt', model }
        ] }
      });
      PM.AgentUI.switchThread('invalid-codex');
      expect(PM.AgentUI.state.provider).toBe('compatible');
      expect(PM.AgentUI.state.model).toBe('configured');
      expect(models).not.toHaveBeenCalled();
      expect(AGENT_MODELS.chatgpt.some(item => item.id === model)).toBe(false);
    } finally {
      PM?.AgentUI.flushThreads();
      AGENT_MODELS.chatgpt = previousModels;
      resetBridgeForTests();
    }
  });
  it('resolves rectangular selections in either direction', () => {
    const math = spatialRegistry().SpatialAssistant.math;

    expect(math.selectionRect({ x: 310, y: 260 }, { x: 120, y: 80 })).toEqual(
      { x: 120, y: 80, width: 190, height: 180 },
    );
    expect(math.selectionRect({ x: 40, y: 50 }, { x: 40, y: 50 })).toEqual(
      { x: 40, y: 50, width: 0, height: 0 },
    );
  });

  it('distinguishes click cancellation from drag selection', () => {
    const math = spatialRegistry().SpatialAssistant.math;

    expect(math.isClickGesture([{ x: 100, y: 100 }, { x: 103, y: 102 }])).toBe(true);
    expect(math.isClickGesture([{ x: 100, y: 100 }, { x: 125, y: 102 }])).toBe(false);
    expect(math.overlayPointerAction('arming', 0, false)).toBe('cancel');
    expect(math.overlayPointerAction('selecting', 0, false)).toBe('select');
    expect(math.overlayPointerAction('composing', 0, true)).toBe('ignore');
    expect(math.overlayPointerAction('composing', 2, false)).toBe('ignore');
  });

  it('bounds large editable catalogs while prioritizing selected and named layers', () => {
    const math = spatialRegistry().SpatialAssistant.math;
    const controls = Array.from({ length: 80 }, (_, index) => ({
      path: `properties.control-${index}`,
      label: `Control ${index}`,
      control: 'slider',
      value: index,
      min: 0,
      max: 100,
    }));
    const layers = Array.from({ length: 700 }, (_, index) => ({
      id: `layer-${index}`,
      name: index === 699 ? 'Hero headline' : `Layer ${index}`,
      type: 'text',
      group: null,
      parent: null,
      controls,
    }));

    const result = math.boundedEditableSource(
      { target: '$composition', composition: [], operations: ['set_property'], layers },
      'Polish the Hero headline',
      ['layer-698'],
      72_000,
    );

    expect(JSON.stringify(result).length).toBeLessThanOrEqual(72_000);
    expect(result.truncated).toBe(true);
    expect(result.totalLayers).toBe(700);
    expect(result.layers.slice(0, 2).map((layer: any) => layer.id)).toEqual(['layer-698', 'layer-699']);
    expect(result.layerIndex[0]).toMatchObject({ id: 'layer-0', name: 'Layer 0', type: 'text' });
  });

  it('keeps oversized assembled prompts inside the IPC limit without dropping the user request', () => {
    const math = spatialRegistry().SpatialAssistant.math;
    const request = 'Explain what we can do after showing the Powermove window';
    const prompt = `AGENT INSTRUCTIONS\n${'source-property '.repeat(260_000)}\n\nUSER REQUEST\n${request}`;

    const result = math.boundedAgentPrompt(prompt, request, 200_000);

    expect(result.length).toBeLessThanOrEqual(200_000);
    expect(result).toContain('Earlier project context was clipped');
    expect(result.endsWith(`USER REQUEST\n${request}`)).toBe(true);
  });
  it('keeps automatic control grouping unspecified while preserving explicit overrides', () => {
    const math = spatialRegistry().SpatialAssistant.math;
    const plan = math.sanitizePlan({operation:'create',section:{title:'Settings',controls:
      [null,false,true].map((advanced,index)=>({type:'slider',label:`Value ${index}`,stateKey:`value${index}`,min:0,max:1,defaultValue:.5,advanced}))
    }},{});
    expect(plan.section.controls).toHaveLength(3);
    expect(plan.section.controls[0]).not.toHaveProperty('advanced');
    expect(plan.section.controls[1].advanced).toBe(false);
    expect(plan.section.controls[2].advanced).toBe(true);
  });

  it('does not let an oversized selected layer hide all other editable targets', () => {
    const result = spatialRegistry().SpatialAssistant.math.boundedEditableSource({
      operations: ['set_property'], layers: [
        { id: 'large', name: 'Large', controls: [{ value: 'x'.repeat(90_000) }] },
        { id: 'small', name: 'Logo', controls: [{ path: 'opacity', value: 100 }] },
      ],
    }, 'Adjust the Logo', ['large']);
    expect(result.layers.some((layer: any) => layer.id === 'small')).toBe(true);
    expect(result.note).toContain('get_project_state');
  });

  it('preserves whole conversation and target sections ahead of an oversized manifest', () => {
    const request = 'Move the logo right';
    const conversation = JSON.stringify({ threadId: 'thread', messages: [{ role: 'user', text: 'Keep the readable hold.' }] });
    const catalog = JSON.stringify({ layers: [{ id: 'logo', controls: [{ path: 'position.x' }] }] });
    const prompt = `Instructions\n\nCURRENT WORKSPACE MANIFEST\n${JSON.stringify({ large: 'x'.repeat(210_000) })}\n\nCONVERSATION SO FAR\n${conversation}\n\nEDITABLE SOURCE CATALOG\n${catalog}\n\nUSER REQUEST\n${request}`;
    const result = spatialRegistry().SpatialAssistant.math.boundedAgentPrompt(prompt, request, 1500);
    expect(result).toContain(conversation);
    expect(result).toContain(catalog);
    expect(result).not.toContain('CURRENT WORKSPACE MANIFEST');
    expect(result.length).toBeLessThanOrEqual(1500);
  });
});
