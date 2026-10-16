/**
 * Render parity cases for pm-render's effects and transitions
 * (docs/engine/effects.md): every built-in effect and transition on content
 * pm-render draws (solids and the tested still classes), so the whole
 * staged path is compared with today's compositor through the app: the
 * layer buffer, the effect passes with today's uniforms, `u_orig`, the
 * composite, transitions in and out, effect stacks, animated enables,
 * missing definitions, effects on groups and inside precomps, partial
 * opacity, and transparent output.
 */
import { effects, transitions } from '../catalog';
import { newProject, type Builder, type Channel, type Value } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, type CorpusProject } from './common';

const PHOTO = 'corpus-photo-lossless-opaque.webp';

/** Two parameter sets per effect: one animates across the clip. */
function values(b: Builder, id: string): Array<Record<string, Value | Channel>> {
  const k = (from: Value, to: Value) => b.ch(from, [[0, from], [2, to]]);
  switch (id) {
    case 'blur': return [{ amount: k(2, 30) }, { amount: 120 }];
    case 'motionblurDir': return [{ amount: 40, angle: k(0, 90) }, { amount: 160, angle: -30 }];
    case 'sharpen': return [{ amount: k(40, 220) }, { amount: 300 }];
    case 'glow': return [{ threshold: 40, radius: k(20, 90), intensity: 150 }, { threshold: 10, radius: 250, intensity: 350 }];
    case 'color': return [{ exposure: .4, contrast: 20, saturation: 50, hue: k(0, 90), temperature: 30 }, { exposure: 2.5, contrast: -60, saturation: 180, hue: -150, temperature: -80 }];
    case 'levels': return [{ inBlack: 10, inWhite: k(100, 70), gamma: 1.4 }, { inBlack: 40, inWhite: 60, gamma: .3 }];
    case 'duotone': return [{ shadow: k('#1B2A4A', '#5A0A2A'), highlight: '#FFD66B', amount: 80 }, { shadow: '#000000', highlight: '#3FA7D6', amount: 35 }];
    case 'grain': return [{ amount: k(10, 40), size: 2 }, { amount: 90, size: 6.5 }];
    case 'vignette': return [{ amount: k(30, 90), feather: 40 }, { amount: 100, feather: 3 }];
    case 'chroma': return [{ amount: k(0, 20) }, { amount: 70 }];
    case 'pixelate': return [{ size: k(4, 24) }, { size: 1 }];
    case 'posterize': return [{ levels: k(16, 3) }, { levels: 2 }];
    case 'displace': return [{ amount: k(10, 40), scale: 3, speed: 1 }, { amount: 300, scale: 15, speed: -3 }];
    case 'shadow': return [{ distance: k(5, 40), angle: 135, softness: 20, opacity: 80, color: '#000000' }, { distance: 200, angle: -45, softness: 250, opacity: 100, color: '#FF6B1A' }];
    case 'invert': return [{ amount: k(0, 100) }, { amount: 50 }];
    case 'gradient': return [{ startColor: '#FF6B1A', endColor: '#1B2A4A', angle: k(0, 180), midpoint: 40, radial: false, amount: 60 }, { radial: true, shade: true, centerX: 20, centerY: -10, spread: 150, dither: 4, midpoint: 70, amount: 100 }];
    default: throw new Error(`No render corpus values for effect ${id}`);
  }
}

function effectProject(id: string): CorpusProject {
  const b = newProject(`render/fx-${id}`, { name: `Render effect ${id}`, backgroundFill: GRADIENT, w: 480, h: 270, dur: 2 });
  // Fresh channels per use: keyframe ids are unique in a document.
  const first = () => values(b, id)[0]!;
  const second = values(b, id)[1]!;
  b.add(b.layer('solid', { name: 'Back', d: { color: '#202634', w: 300, h: 160 }, p: { 'position.x': 20, 'position.y': 20, opacity: 90 } }));
  const photo = b.add(b.layer('image', { name: 'Photo', d: { asset: ref(PHOTO), fit: 'stretch', w: 250, h: 150 }, p: { 'position.x': 60, 'position.y': 50, rotation: 6 } }));
  photo.fx.push(b.effect(id, first()));
  const alpha = b.add(b.layer('image', { name: 'Alpha image', d: { asset: ref('still-alpha.png'), fit: 'contain', w: 160, h: 90 }, p: { 'position.x': 300, 'position.y': 150, opacity: 85 } }));
  alpha.fx.push(b.effect(id, second));
  const chip = b.add(b.layer('solid', { name: 'Chip', d: { color: '#3FCF8E', w: 60, h: 60 }, p: { 'position.x': 380, 'position.y': 40, rotation: 20 } }));
  chip.fx.push(b.effect(id, first()));
  return project(b, { group: 'Render', title: `Effect ${id}`, covers: [`effect ${id}`, 'keyframed effect parameter', 'effect at extreme values', 'effect on partial opacity'], samples: [0, 1, 2], alpha: true });
}

function transitionProject(id: string): CorpusProject {
  const b = newProject(`render/tr-${id}`, { name: `Render transition ${id}`, backgroundFill: GRADIENT, w: 480, h: 270, dur: 3 });
  const definition = transitions.find(t => t.id === id)!;
  const params = Object.fromEntries(definition.params.map(param => {
    if (param.k === 'angle') return [param.k, 30];
    if (param.k === 'softness') return [param.k, 20];
    if (param.k === 'direction') return [param.k, 90];
    if (param.k === 'amount') return [param.k, 60];
    return [param.k, param.def as Value];
  }));
  b.add(b.layer('solid', { name: 'Back', d: { color: '#202634', w: 480, h: 270 }, p: { opacity: 70 } }));
  const photo = b.add(b.layer('image', { name: 'Photo', from: .2, dur: 2.6, d: { asset: ref(PHOTO), fit: 'stretch', w: 250, h: 150 }, p: { 'position.x': 100, 'position.y': 60 } }));
  photo.transitionIn = b.transition(id, .8, params);
  photo.transitionOut = b.transition(id, .6, params);
  const chip = b.add(b.layer('solid', { name: 'Chip', d: { color: '#FF6B1A', w: 120, h: 80 }, p: { 'position.x': 300, 'position.y': 160, opacity: 80 } }));
  chip.transitionIn = b.transition(id, 1.2);
  return project(b, {
    group: 'Render', title: `Transition ${id}`, covers: [`transition ${id}`, 'transition in', 'transition out', 'transition parameters'],
    samples: [.2, .5, .9, 1.5, 2.5, 2.7], alpha: true,
  });
}

/** Stacks, enables, missing definitions, groups, precomps, and opacity. */
function combinations(): CorpusProject[] {
  const stack = newProject('render/fx-stack', { name: 'Effect stack', backgroundFill: GRADIENT, w: 480, h: 270, dur: 2 });
  const photo = stack.add(stack.layer('image', { name: 'Photo', d: { asset: ref(PHOTO), fit: 'stretch', w: 250, h: 150 }, p: { 'position.x': 60, 'position.y': 40 } }));
  photo.fx.push(
    stack.effect('glow', { threshold: 30, radius: 40, intensity: 160 }),
    stack.effect('color', { saturation: -100 }, stack.ch(true, [[0, true], [1, false]])),
    stack.effect('shadow', { distance: 18, softness: 30, opacity: 70 }),
    stack.effect('blur', { amount: 3 }, false),
    stack.foreignEffect('com.example.missing-wobble', { amount: 4 }),
  );
  const chip = stack.add(stack.layer('solid', { name: 'Missing only', d: { color: '#4C8DFF', w: 80, h: 80 }, p: { 'position.x': 360, 'position.y': 150 } }));
  chip.fx.push(stack.foreignEffect('com.example.missing-word-swap', { progress: 30 }));

  const grouped = newProject('render/fx-group-precomp', { name: 'Effects on groups and in precomps', backgroundFill: GRADIENT, w: 480, h: 270, dur: 2 });
  const group = grouped.layer('group', { name: 'Blurred group', p: { opacity: 80 } });
  grouped.add(grouped.layer('solid', { name: 'Member A', group: group.id, d: { color: '#FF6B1A', w: 100, h: 100 }, p: { 'position.x': 30, 'position.y': 30 } }));
  grouped.add(grouped.layer('solid', { name: 'Member B', group: group.id, d: { color: '#4C8DFF', w: 100, h: 100 }, p: { 'position.x': 80, 'position.y': 80 } }));
  group.fx.push(grouped.effect('blur', { amount: grouped.ch(4, [[0, 4], [2, 24]]) }));
  grouped.add(group);
  const inner = grouped.composition({ name: 'Inner', w: 200, h: 120, dur: 2, bg: '#000000' });
  grouped.within(inner, () => {
    const image = grouped.add(grouped.layer('image', { name: 'Inner photo', d: { asset: ref(PHOTO), fit: 'stretch', w: 200, h: 120 } }));
    image.fx.push(grouped.effect('vignette', { amount: 80, feather: 50 }), grouped.effect('chroma', { amount: 12 }));
  });
  const pre = grouped.add(grouped.layer('precomp', { name: 'Precomp with effects', d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 250, 'position.y': 120, rotation: -5 } }));
  pre.fx.push(grouped.effect('levels', { inBlack: 5, inWhite: 85, gamma: 1.2 }));
  const fading = grouped.layer('group', { name: 'Fading group with a transition', from: .1, dur: 1.8 });
  grouped.add(grouped.layer('solid', { name: 'Fading member', group: fading.id, d: { color: '#E8E2CF', w: 140, h: 60 }, p: { 'position.x': 300, 'position.y': 20 } }));
  fading.transitionIn = grouped.transition('wipe', .9, { angle: 45, softness: 30 });
  grouped.add(fading);

  return [
    project(stack, { group: 'Render', title: 'Effect stack', covers: ['effect stacking', 'animatable effect enable', 'disabled effect', 'missing effect skipped with its data kept', 'keepOrig effects in a stack'], samples: [0, .5, 1.5], alpha: true }),
    project(grouped, { group: 'Render', title: 'Effects on groups and in precomps', covers: ['effect on a group (compositing boundary)', 'effects inside a precomp', 'effect on a precomp layer', 'transition on a group'], samples: [0, .4, 1, 2], alpha: true }),
  ];
}

export function renderEffectProjects(): CorpusProject[] {
  return [...effects.map(e => effectProject(e.id)), ...transitions.map(t => transitionProject(t.id)), ...combinations()];
}
