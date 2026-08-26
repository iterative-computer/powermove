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
  PM.syncShaderUniforms = () => {};
  PM.mkEffect = () => null;
  PM.Export = { snapshot: (time: number) => `data:image/jpeg;base64,frame-${time}` };
  return PM;
}

afterEach(() => vi.unstubAllGlobals());

describe('agent harness oracle', () => {
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
