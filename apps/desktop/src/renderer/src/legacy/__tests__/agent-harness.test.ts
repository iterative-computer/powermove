import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from './make-pm';

function harnessEditor(): PMRegistry {
  vi.stubGlobal('window', {
    requestAnimationFrame: (resolve: FrameRequestCallback) => resolve(0),
  });
  const PM = makePM(
    'core/easing', 'core/model', 'core/selection', 'core/anim',
    'core/history', 'core/editing', 'assistant/harness',
  );
  PM.proj = PM.mkProject({ name: 'Harness test', w: 1920, h: 1080, fps: 30, dur: 6 });
  PM.time = 1;
  PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
  PM.mkEffect = () => null;
  PM.Export = { snapshot: (time: number) => `data:image/jpeg;base64,frame-${time}` };
  return PM;
}

afterEach(() => vi.unstubAllGlobals());

describe('agent harness oracle', () => {
  it('pages large animated properties without silently losing their counts', () => {
    const PM = harnessEditor(), layer = PM.mkLayer('shape');
    PM.proj.layers = [layer];
    const properties = PM.allProps(layer);
    properties[0].prop.kf = Array.from({ length: 240 }, (_, t) => ({ t: t / 30, v: t }));
    const state = PM.AgentHarness.projectState({ layerId: layer.id, propertyLimit: 1, keyframeLimit: 3, keyframeOffset: 100 });
    expect(state.layers).toHaveLength(1);
    expect(state.layers[0].propertyCount).toBe(properties.length);
    expect(state.layers[0].properties).toHaveLength(1);
    expect(state.layers[0].properties[0].keyframeCount).toBe(240);
    expect(state.layers[0].properties[0].keyframes.map((k: any) => k.value)).toEqual([100, 101, 102]);
  });

  it('groups and parents through live agent commands and exposes membership in project state', async () => {
    const PM = harnessEditor(), a = PM.mkLayer('shape'), b = PM.mkLayer('shape'), rig = PM.mkLayer('null');
    PM.proj.layers = [a,b,rig]; PM.ProjectIndex.invalidate();
    const before = PM.worldMatrix(a, PM.time);
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({runId:'group-parent-run',callId:tool,tool,arguments:args,baseRevision:0});
    const grouped = await call('apply_commands',{commands:[{type:'group_layers',targets:[a.id,b.id],name:'Agent group'}]});
    expect(grouped.ok).toBe(true);
    const group = PM.firstSel(); expect(group.type).toBe('group');
    await call('apply_commands',{commands:[{type:'set_layer',target:a.id,patch:{parent:rig.id}}]});
    expect(PM.L(a.id).parent).toBe(rig.id); expect(PM.worldMatrix(PM.L(a.id),PM.time)).toEqual(before);
    const state = PM.AgentHarness.projectState(); expect(state.layers.find((layer:any)=>layer.id===a.id).group).toBe(group.id);
    await call('rollback_changes');
    expect(PM.proj.layers).toHaveLength(3); expect(PM.L(a.id).group).toBeUndefined(); expect(PM.L(a.id).parent).toBeNull();
  });

  it('guards and rolls back video tool edits through the live run transaction', async () => {
    const PM = harnessEditor();
    const video = PM.mkLayer('video', { from: 0, dur: 5 });
    PM.proj.layers.push(video);
    const before = JSON.stringify(PM.proj.layers);
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({ runId: 'video-rollback', callId: tool, tool, arguments: args, baseRevision: 0 });
    const split = await call('edit_video', { operation: 'split', layerId: video.id, at: 2 });
    expect(JSON.parse(split.content[0].text).clip.tailId).toBeTruthy();
    await call('rollback_changes');
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    await call('edit_video', { operation: 'move', layerId: video.id, from: 1 });
    PM.Edit.apply({ type: 'set_layer', target: video.id, patch: { name: 'User edit' } }, { origin: 'inspector' });
    await expect(call('edit_video', { operation: 'remove', layerId: video.id })).rejects.toThrow('project changed');
    const finish = await call('__finish_run', { commit: false });
    expect(finish.ok).toBe(false);
    expect(PM.L(video.id).name).toBe('User edit');
  });

  it('lets a native provider inspect and transactionally edit the live project through the preload bridge', async () => {
    let handleRequest: ((request: any) => void) | null = null;
    const responses: any[] = [];
    vi.stubGlobal('window', {
      requestAnimationFrame: (resolve: FrameRequestCallback) => resolve(0),
      atob: (value: string) => Buffer.from(value, 'base64').toString('binary'),
      powermove: {
        agentTools: {
          onRequest(callback: (request: any) => void) { handleRequest = callback; return () => {}; },
          respond(response: any) { responses.push(response); }
        }
      }
    });
    const PM = makePM(
      'core/easing', 'core/model', 'core/selection', 'core/anim',
      'core/history', 'core/editing', 'assistant/harness',
    );
    PM.proj = PM.mkProject({ name: 'Native tools', w: 1920, h: 1080, fps: 30, dur: 6 });
    PM.time = 0;
    PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
    PM.mkEffect = () => null;
    PM.Export = { snapshot: (time: number) => `data:image/png;base64,${Buffer.from(`frame-${time}`).toString('base64')}` };
    expect(handleRequest).toBeTypeOf('function');

    handleRequest!({
      runId: 'native-run-1234', callId: 'call-state', tool: 'get_project_state',
      arguments: {}, baseRevision: 0
    });
    await vi.waitFor(() => expect(responses).toHaveLength(1));
    expect(JSON.parse(responses[0].content[0].text).composition.revision).toBe(0);

    handleRequest!({
      runId: 'native-run-1234', callId: 'call-apply', tool: 'apply_commands',
      arguments: {
        label: 'Add native title',
        commands: [JSON.stringify({
          type: 'add_layer', id: 'native-title', layerType: 'text', name: 'Native title',
          content: { text: 'Live' }, properties: { 'position.x': 960, 'position.y': 540 }
        })]
      },
      baseRevision: 0
    });
    await vi.waitFor(() => expect(responses).toHaveLength(2));
    expect(responses[1]).toMatchObject({ ok: true, changed: true, revision: 1 });
    expect(PM.L('native-title')).toBeTruthy();

    handleRequest!({
      runId: 'native-run-1234', callId: 'call-render', tool: 'render_frames',
      arguments: { times: [0, 3], width: 640 }, baseRevision: 0
    });
    await vi.waitFor(() => expect(responses).toHaveLength(3));
    expect(responses[2].content.filter((item: any) => item.type === 'image')).toHaveLength(2);

    handleRequest!({
      runId: 'native-run-1234', callId: 'call-finish', tool: '__finish_run',
      arguments: { commit: true }, baseRevision: 0
    });
    await vi.waitFor(() => expect(responses).toHaveLength(4));
    expect(responses[3]).toMatchObject({ ok: true, changed: true });
    expect(responses[3].historyId).toBeTruthy();
    expect(PM.hist.list()).toHaveLength(1);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L('native-title')).toBeNull();
  });

  it('rolls native live edits back when the provider run fails', async () => {
    let handleRequest: ((request: any) => void) | null = null;
    const responses: any[] = [];
    vi.stubGlobal('window', {
      requestAnimationFrame: (resolve: FrameRequestCallback) => resolve(0),
      atob: (value: string) => Buffer.from(value, 'base64').toString('binary'),
      powermove: {
        agentTools: {
          onRequest(callback: (request: any) => void) { handleRequest = callback; return () => {}; },
          respond(response: any) { responses.push(response); }
        }
      }
    });
    const PM = makePM(
      'core/easing', 'core/model', 'core/selection', 'core/anim',
      'core/history', 'core/editing', 'assistant/harness',
    );
    PM.proj = PM.mkProject({ name: 'Native rollback', w: 1920, h: 1080, fps: 30, dur: 6 });
    PM.Kernel.services.register('shaderHooks', { syncShaderUniforms() {} });
    PM.mkEffect = () => null;

    handleRequest!({
      runId: 'native-run-failed', callId: 'call-apply', tool: 'apply_commands',
      arguments: {
        commands: [JSON.stringify({
          type: 'add_layer', id: 'temporary-title', layerType: 'text', name: 'Temporary title',
          content: { text: 'Temporary' }
        })]
      },
      baseRevision: 0
    });
    await vi.waitFor(() => expect(PM.L('temporary-title')).toBeTruthy());

    handleRequest!({
      runId: 'native-run-failed', callId: 'call-finish', tool: '__finish_run',
      arguments: { commit: false }, baseRevision: 0
    });
    await vi.waitFor(() => expect(responses).toHaveLength(2));

    expect(responses[1]).toMatchObject({ ok: true, changed: false });
    expect(PM.L('temporary-title')).toBeNull();
  });

  it('observes source and real frames, applies one revision, and rolls the whole run back', async () => {
    const PM = harnessEditor();
    const bridgeCalls: any[] = [];
    PM.CodexBridge = {
      async request(prompt: string, schema: unknown, images: string[]) {
        bridgeCalls.push({ prompt, schema, images });
        return JSON.stringify({
          status: 'pass', message: 'Looks correct', critique: '', commands: [], reviewTimes: [0, 3, 6],
        });
      },
    };
    const proposal = PM.AgentHarness.sanitizeProposal({
      label: 'Add title', summary: 'Add a real editable title', reviewTimes: [0, 3, 6],
      commands: [JSON.stringify({
        type: 'add_layer', id: 'agent-title', layerType: 'text', name: 'Agent title',
        from: 0, duration: 6, content: { text: 'Connected' },
        properties: { 'position.x': 960, 'position.y': 540 },
      })],
    });

    const run = await PM.AgentHarness.execute('Add a title', proposal);

    expect(PM.L('agent-title')).toBeTruthy();
    expect(PM.proj.revision).toBe(1);
    expect(PM.proj.edits[0].origin).toBe('agent');
    expect(bridgeCalls[0].images.length).toBeGreaterThanOrEqual(3);
    expect(bridgeCalls[0].prompt).toMatch(/visual review stage/);
    expect(run.frames.images.length).toBeGreaterThanOrEqual(3);
    expect(PM.hist.list()).toHaveLength(1);
    expect(PM.AgentHarness.rollback(run.checkpoint)).toBe(true);
    expect(PM.L('agent-title')).toBeNull();
    expect(PM.hist.redo()).toBe(true);
    expect(PM.L('agent-title')).toBeTruthy();
  });

  it('collapses initial edits and visual-review repairs into one reversible agent step', async () => {
    const PM = harnessEditor();
    let review = 0;
    PM.CodexBridge = {
      async request(_prompt: string, _schema: unknown, _images: string[], options: any) {
        options?.onProgress?.('Checking title placement against the rendered frame');
        if (review++ === 0) {
          return JSON.stringify({
            status: 'repair', message: 'Move it right', critique: 'The title is too far left', reviewTimes: [1],
            commands: [JSON.stringify({
              type: 'set_property', target: 'agent-title', path: 'position.x', value: 800,
            })],
          });
        }
        return JSON.stringify({
          status: 'pass', message: 'Looks correct', critique: '', commands: [], reviewTimes: [1],
        });
      },
    };
    const proposal = PM.AgentHarness.sanitizeProposal({
      label: 'Add and place title', summary: 'Add a title', reviewTimes: [1],
      commands: [JSON.stringify({
        type: 'add_layer', id: 'agent-title', layerType: 'text', name: 'Agent title',
        content: { text: 'Reversible' }, properties: { 'position.x': 300, 'position.y': 300 },
      })],
    });

    await PM.AgentHarness.execute('Add a title', proposal);

    expect(PM.hist.list()).toHaveLength(1);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L('agent-title')).toBeNull();
    expect(PM.hist.redo()).toBe(true);
    const title = PM.L('agent-title');
    expect(PM.evP(title, PM.findProp(title, 'position.x'), 1, 'position.x')).toBe(800);
  });
});
