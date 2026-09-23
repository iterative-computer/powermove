import { installBridgeForTests, resetBridgeForTests } from '../../kernel/bridge';
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
  it('selects panel targets directly and rejects missing IDs without partial selection or edits', async () => {
    const PM = harnessEditor();
    const a = PM.mkLayer('shape'), b = PM.mkLayer('text');
    PM.proj.layers = [a, b]; PM.ProjectIndex.invalidate();
    const before = JSON.stringify(PM.proj);
    const call = (layerIds: string[], add = false) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'select-targets', callId: 'select', tool: 'select_layers', arguments: { layerIds, add }, baseRevision: 0
    });
    await call([a.id]);
    expect(PM.sel.layers).toEqual([a.id]);
    await expect(call([b.id, 'missing'])).rejects.toThrow('No selection was changed');
    expect(PM.sel.layers).toEqual([a.id]);
    await call([b.id], true);
    expect(PM.sel.layers).toEqual([a.id, b.id]);
    await call([]);
    expect(PM.sel.layers).toEqual([]);
    expect(JSON.stringify(PM.proj)).toBe(before);
  });

  it('observes source without capturing frames unless the agent requests times', async () => {
    const PM = harnessEditor();
    const snapshot = vi.fn(PM.Export.snapshot);
    PM.Export.snapshot = snapshot;
    for (const times of [undefined, [], [NaN]]) {
      const observation = await PM.AgentHarness.observe(times);
      expect(observation.state.composition.playhead).toBe(1);
      expect(observation.times).toEqual([]);
      expect(observation.images).toEqual([]);
    }
    expect(snapshot).not.toHaveBeenCalled();
    expect(PM.AgentHarness.sanitizeProposal({ commands: [] }).reviewTimes).toEqual([]);
    expect(PM.AgentHarness.sanitizeProposal({ commands: [], reviewTimes: [] }).reviewTimes).toEqual([]);
    const requested = await PM.AgentHarness.observe([0, 2]);
    expect(requested.times).toEqual([0, 2]);
    expect(requested.images).toHaveLength(2);
    expect(snapshot).toHaveBeenCalledTimes(2);
    expect(PM.time).toBe(1);
  });

  it('reviews and completes an edit without frames when the agent leaves reviewTimes empty', async () => {
    const PM = harnessEditor();
    PM.Export.snapshot = vi.fn(() => { throw new Error('Unexpected frame capture'); });
    PM.CodexBridge = { request: vi.fn(async () => JSON.stringify({ status: 'pass', message: 'Source checked', commands: [], reviewTimes: [] })) };
    const proposal = PM.AgentHarness.sanitizeProposal({
      commands: [{ type: 'add_layer', layerType: 'text', name: 'Title', content: { text: 'Hello' } }],
      reviewTimes: [],
    });
    const run = await PM.AgentHarness.execute('Add a title', proposal);
    expect(PM.proj.layers).toHaveLength(1);
    expect(run.reviewError).toBe('');
    expect(run.frames.images).toEqual([]);
    expect(PM.CodexBridge.request.mock.calls[0][2]).toEqual([]);
    expect(PM.Export.snapshot).not.toHaveBeenCalled();
  });

  it('preflights effect definitions with the registration validator without changing the project', async () => {
    const PM = harnessEditor();
    const before = JSON.stringify(PM.proj);
    const definition = {
      id: 'typewriter', label: 'Typewriter', group: 'Text',
      params: Array.from({ length: 36 }, (_, i) => ({ k: `p${i}`, label: `P${i}`, def: 0, min: 0, max: 1 })),
      frag: 'o = texture(u_tex, v_st);'
    };
    const call = () => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'validate-effect', callId: 'validate', tool: 'validate_effect',
      arguments: { definition }, baseRevision: 0
    });
    await expect(call()).rejects.toThrow('too many params (36 > 32)');
    definition.params = definition.params.slice(0, 32);
    const result = await call();
    expect(JSON.parse(result.content[0].text)).toMatchObject({ valid: true, id: 'typewriter', parameterCount: 32 });
    expect(result.changed).not.toBe(true);
    expect(JSON.stringify(PM.proj)).toBe(before);
    definition.params[1]!.k = definition.params[0]!.k;
    await expect(call()).rejects.toThrow('duplicate param key');
  });

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

  it('recovers from a stale run-start revision after reading the live project', async () => {
    const PM = harnessEditor();
    PM.proj.revision = 6;
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'stale-start', callId: tool, tool, arguments: args, baseRevision: 3,
    });
    const commands = [{ type: 'add_layer', id: 'fresh-title', layerType: 'text' }];
    await expect(call('apply_commands', { commands })).rejects.toThrow('revision 3');
    await call('get_project_state');
    expect((await call('apply_commands', { commands })).ok).toBe(true);
    expect(PM.L('fresh-title')).toBeTruthy();
    await call('rollback_changes');
    expect(PM.L('fresh-title')).toBeNull();
    expect(PM.proj.revision).toBe(6);
  });

  it('resumes after interleaved user edits without absorbing them into agent Undo or rollback', async () => {
    const PM = harnessEditor();
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'interleaved', callId: tool, tool, arguments: args, baseRevision: 0,
    });
    await call('apply_commands', { commands: [{ type: 'add_layer', id: 'first', layerType: 'text' }] });
    PM.Edit.apply({ type: 'set_layer', target: 'first', patch: { name: 'User title' } });
    const commands = [{ type: 'add_layer', id: 'second', layerType: 'text' }];
    await expect(call('apply_commands', { commands })).rejects.toThrow('project changed');
    await call('get_project_state');
    expect((await call('apply_commands', { commands })).ok).toBe(true);
    await expect(call('rollback_changes')).rejects.toThrow('Undo');
    expect(PM.L('first').name).toBe('User title');
    expect((await call('__finish_run', { commit: true })).ok).toBe(true);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L('second')).toBeNull();
    expect(PM.L('first').name).toBe('User title');
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L('first').name).not.toBe('User title');
  });

  it('applies complete large batches, animation curves, and delete selections', async () => {
    const PM = harnessEditor();
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'large-edit', callId: tool, tool, arguments: args, baseRevision: 0,
    });
    const commands = Array.from({ length: 90 }, (_, i) => ({ type: 'add_layer', id: `layer-${i}`, layerType: 'text' }));
    await call('apply_commands', { commands });
    expect(PM.proj.layers).toHaveLength(90);
    const keyframes = Array.from({ length: 150 }, (_, i) => ({ time: i / 30, value: i }));
    await call('apply_commands', { commands: [{ type: 'replace_keyframes', target: 'layer-0', path: 'opacity', keyframes }] });
    expect(PM.L('layer-0').p.opacity.kf).toHaveLength(150);
    await call('apply_commands', { commands: [{ type: 'delete_layers', targets: commands.map(c => c.id) }] });
    expect(PM.proj.layers).toHaveLength(0);
    await call('rollback_changes');
    expect(PM.proj.layers).toHaveLength(0);
  });

  it('rejects an invalid command without partially applying the rest of its batch', async () => {
    const PM = harnessEditor();
    await expect(PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'invalid-batch', callId: 'apply', tool: 'apply_commands', baseRevision: 0,
      arguments: { commands: [{ type: 'add_layer', layerType: 'text' }, { type: 'unknown_operation' }] },
    })).rejects.toThrow();
    expect(PM.proj.layers).toHaveLength(0);
  });

  it('requires another read if the project changes after inspection and isolates run baselines', async () => {
    const PM = harnessEditor();
    const call = (tool: string, runId = 'reader', args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId, callId: tool, tool, arguments: args, baseRevision: 0,
    });
    PM.proj.revision = 3;
    await call('get_project_state');
    PM.proj.revision = 6;
    const args = { commands: [{ type: 'add_layer', id: 'fresh', layerType: 'text' }] };
    await expect(call('apply_commands', 'reader', args)).rejects.toThrow('revision 3');
    await call('get_project_state');
    await expect(call('apply_commands', 'other-run', args)).rejects.toThrow('revision 0');
    expect((await call('apply_commands', 'reader', args)).ok).toBe(true);
  });

  it('does not acknowledge a state read that exceeds the response budget', async () => {
    const PM = harnessEditor();
    const layer = PM.mkLayer('text');
    layer.d.text = 'x'.repeat(2_000_000);
    PM.proj.layers.push(layer);
    PM.proj.revision = 6;
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'failed-read', callId: tool, tool, arguments: args, baseRevision: 3,
    });
    await expect(call('get_project_state')).rejects.toThrow('response budget');
    await expect(call('apply_commands', { commands: [{ type: 'add_layer', layerType: 'text' }] })).rejects.toThrow('revision 3');
  });

  it('never refreshes or rolls a transaction into a different project with the same revision', async () => {
    const PM = harnessEditor();
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'project-switch', callId: tool, tool, arguments: args, baseRevision: 0,
    });
    await call('apply_commands', { commands: [{ type: 'add_layer', layerType: 'text' }] });
    PM.proj = PM.mkProject({ name: 'Other project' });
    PM.proj.revision = 1;
    await expect(call('get_project_state')).rejects.toThrow('active project changed');
    await expect(call('apply_commands', { commands: [{ type: 'add_layer', layerType: 'text' }] })).rejects.toThrow('active project changed');
    await expect(call('rollback_changes')).rejects.toThrow('active project changed');
    expect((await call('__finish_run', { commit: false })).ok).toBe(false);
    expect(PM.proj.layers).toHaveLength(0);
  });

  it('finishes successfully with separate Undo entries when the user edits after the last tool call', async () => {
    const PM = harnessEditor();
    const call = (tool: string, args: any = {}) => PM.AgentHarness.test.handleLiveAgentTool({
      runId: 'finish-drift', callId: tool, tool, arguments: args, baseRevision: 0,
    });
    await call('apply_commands', { commands: [{ type: 'add_layer', id: 'title', layerType: 'text' }] });
    PM.Edit.apply({ type: 'set_layer', target: 'title', patch: { name: 'User title' } });
    const finish = await call('__finish_run', { commit: true });
    expect(finish).toMatchObject({ ok: true, changed: true });
    expect(finish.historyId).toBeUndefined();
    expect(PM.hist.list()).toHaveLength(2);
    expect(PM.L('title').name).toBe('User title');
  });

  it('supports larger state pages and returns complete editable content', () => {
    const PM = harnessEditor();
    PM.proj.layers = Array.from({ length: 25 }, () => PM.mkLayer('text'));
    const layer = PM.proj.layers[0];
    layer.d.text = 'Text '.repeat(200);
    layer.d.code = 'code '.repeat(2000);
    for (let i = 0; i < 120; i++) layer.p[`custom-${i}`] = PM.P(i);
    layer.p.opacity.kf = Array.from({ length: 150 }, (_, t) => ({ t: t / 30, v: t }));
    const state = PM.AgentHarness.projectState({ layerLimit: 25, propertyLimit: 200, keyframeLimit: 150 });
    expect(state.layers).toHaveLength(25);
    expect(state.layers[0].properties.length).toBeGreaterThan(100);
    expect(state.layers[0].properties.find((p: any) => p.path === 'opacity').keyframes).toHaveLength(150);
    expect(state.layers[0].content).toEqual(layer.d);
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
  installBridgeForTests((window as any).powermove);
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
  installBridgeForTests((window as any).powermove);
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
    expect(bridgeCalls[0].prompt).toMatch(/review stage/);
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
