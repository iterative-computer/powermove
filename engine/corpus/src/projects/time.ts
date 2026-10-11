import { newProject, type Builder, type Doc } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, testCard, type CorpusProject } from './common';

/** A composition whose content shows its own time: a sweeping bar and a time readout. */
function clock(b: Builder, name: string, options: { w?: number; h?: number; fps?: number; dur?: number } = {}): Doc {
  const comp = b.composition({ name, w: options.w ?? 320, h: options.h ?? 180, fps: options.fps ?? 30, dur: options.dur ?? 4, bg: '#1B2A4A' });
  b.within(comp, () => {
    b.add(b.layer('shape', { name: 'Sweep', d: { w: 12, h: comp.h, color: '#FF6B1A', radius: 0 }, p: { 'position.x': b.ch(0, [[0, 0], [comp.dur, comp.w]]) } }));
    b.add(b.layer('text', { name: 'Readout', d: { text: 'time', size: Math.round(comp.h * .25), weight: 700, color: '#FFFFFF' }, p: { rotation: b.ch(0, [], 'time * 90') } }));
  });
  return comp;
}

function precomps(): CorpusProject[] {
  const b = newProject('precomps/nesting', { name: 'Precomp nesting', backgroundFill: GRADIENT });
  const inner = clock(b, 'Inner clock', { w: 200, h: 120, fps: 24 });
  const middle = b.composition({ name: 'Middle', w: 360, h: 240, dur: 3, bg: '#0C0D12' });
  middle.params = { tilt: { name: 'tilt', label: 'Tilt', control: 'num', min: -45, max: 45, value: 20 } };
  b.within(middle, () => {
    b.add(b.layer('shape', { name: 'Middle frame', d: { w: 340, h: 220, color: '#00000000', stroke: 4, strokeColor: '#3FCF8E', radius: 16 }, p: { 'position.x': 180, 'position.y': 120 } }));
    b.add(b.layer('precomp', { name: 'Inner instance A', d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 20, 'position.y': 20, rotation: b.ch(0, [], 'param("tilt")') } }));
    b.add(b.layer('precomp', { name: 'Inner instance B (offset start)', from: 1, d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 150, 'position.y': 110, 'scale.x': 70, 'scale.y': 70 } }));
  });
  b.add(b.layer('precomp', {
    name: 'Middle (effects, mask, opacity)', d: { comp: middle.id, w: 360, h: 240 },
    fx: [b.effect('shadow', { distance: 14, softness: 18 })], p: { 'position.x': 60, 'position.y': 40, opacity: 90 },
    masks: [b.mask('ellipse', { x: 180, y: 120, w: 420, h: 300, feather: 40 })],
  }));
  b.add(b.layer('precomp', { name: 'Middle scaled up', d: { comp: middle.id, w: 360, h: 240 }, p: { 'position.x': 400, 'position.y': 200, 'scale.x': 150, 'scale.y': 150, rotation: -8 } }));
  const cycle = newProject('precomps/cycle', { name: 'Precomp cycle', backgroundFill: GRADIENT });
  const a = cycle.composition({ name: 'A', w: 200, h: 200, dur: 3 });
  const c = cycle.composition({ name: 'B', w: 200, h: 200, dur: 3 });
  cycle.within(a, () => { cycle.add(cycle.layer('precomp', { name: 'B in A', d: { comp: c.id, w: 200, h: 200 } })); cycle.add(cycle.layer('shape', { name: 'A mark', d: { w: 60, h: 60, color: '#FF6B1A' }, p: { 'position.x': 100, 'position.y': 100 } })); });
  cycle.within(c, () => { cycle.add(cycle.layer('precomp', { name: 'A in B', d: { comp: a.id, w: 200, h: 200 }, p: { 'scale.x': 60, 'scale.y': 60 } })); });
  cycle.add(cycle.layer('precomp', { name: 'A', d: { comp: a.id, w: 200, h: 200 }, p: { 'position.x': 220, 'position.y': 80 } }));
  return [
    project(b, { group: 'Composition', title: 'Nested precomps', covers: ['precomp', 'nesting two levels', 'nested frame rate 24 in 30', 'precomp effects and masks', 'precomp at its own size scaled up', 'composition parameters in a nested comp', 'shared nested composition'], samples: [0, 1.25, 2.5], alpha: true }),
    project(cycle, { group: 'Composition', title: 'Precomp cycle', covers: ['nesting with cycle checks (render depth limit)'], samples: [0], notes: 'The editor rejects cycles; the renderer stops at its nesting depth.' }),
  ];
}

function retiming(): CorpusProject[] {
  const b = newProject('retiming/precomp', { name: 'Retimed precomps', backgroundFill: GRADIENT, dur: 4 });
  const source = clock(b, 'Clock', { dur: 4 });
  const instance = (name: string, x: number, y: number, d: Doc) => b.add(b.layer('precomp', { name, d: { comp: source.id, w: 320, h: 180, ...d }, p: { 'position.x': x, 'position.y': y, 'scale.x': 48, 'scale.y': 48 } }));
  instance('Normal', 10, 10, {});
  instance('Trim 1s', 170, 10, { trim: 1 });
  instance('Time remap ramp', 330, 10, { timeRemap: b.ch(true), sourceTime: b.ch(0, [[0, 0], [2, 3.5], [4, 1]]) });
  instance('Freeze at 1.5s', 490, 10, { timeRemap: b.ch(true), sourceTime: b.ch(1.5, [[0, 1.5, { hold: true }]]) });
  instance('Reverse', 10, 190, { timeRemap: b.ch(true), sourceTime: b.ch(4, [[0, 3.9], [3.9, 0]]) });
  const video = newProject('retiming/video', { name: 'Retimed video', w: 320, h: 180, dur: 2, bg: '#202020' });
  const clip = (name: string, x: number, y: number, d: Doc) => video.add(video.layer('video', { name, d: { asset: ref('h264-aac.mp4'), fit: 'contain', w: 160, h: 90, ...d }, p: { 'position.x': x, 'position.y': y } }));
  clip('Speed 2x', 40, 22.5, { speed: 2 });
  clip('Speed 0.5x', 120, 22.5, { speed: .5 });
  clip('Variable speed', 200, 22.5, { speed: video.ch(.5, [[0, .5], [1.5, 2]]) });
  clip('Trim 0.9s', 280, 22.5, { trim: .9 });
  clip('Remap reverse', 40, 112.5, { timeRemap: video.ch(true), sourceTime: video.ch(1.9, [[0, 1.9], [1.9, 0]]) });
  clip('Freeze', 120, 112.5, { timeRemap: video.ch(true), sourceTime: video.ch(1.4, [[0, 1.4, { hold: true }]]) });
  clip('Second decoder, same media', 200, 112.5, {});
  return [
    project(b, { group: 'Retiming', title: 'Retimed precomps', covers: ['trim', 'time remap', 'freeze', 'reverse', 'source time'], samples: [0, .8, 1.6, 2.4, 3.2] }),
    project(video, { group: 'Retiming', title: 'Retimed video', covers: ['speed', 'variable speed', 'trim', 'time remap reverse', 'freeze', 'independent decoders per instance'], samples: [.3, .7, 1.1, 1.6] }),
  ];
}

function multipleCompositions(): CorpusProject {
  const b = newProject('composition/multiple', { name: 'Multiple compositions', backgroundFill: GRADIENT });
  testCard(b);
  const other = b.composition({ name: 'Unused second composition', w: 1280, h: 720, fps: 60, dur: 5, bg: '#A73366' });
  b.within(other, () => b.add(b.layer('text', { name: 'Elsewhere', d: { text: 'Not rendered from the root' } })));
  return project(b, { group: 'Composition', title: 'Multiple compositions in one project', covers: ['multiple compositions', 'unreferenced composition'], samples: [0] });
}

export function timeProjects(): CorpusProject[] {
  return [...precomps(), ...retiming(), multipleCompositions()];
}
