import { effects, transitions } from '../catalog';
import { newProject, type Builder, type Channel, type Value } from '../doc';
import { GRADIENT, project, testCard, type CorpusProject } from './common';

/** Non-default values per built-in effect; one parameter animates across the clip. */
function effectValues(b: Builder, id: string): Record<string, Value | Channel> {
  const k = (from: Value, to: Value) => b.ch(from, [[0, from], [2.5, to]]);
  switch (id) {
    case 'blur': return { amount: k(0, 40) };
    case 'motionblurDir': return { amount: 40, angle: k(0, 90) };
    case 'sharpen': return { amount: k(40, 220) };
    case 'glow': return { threshold: 40, radius: k(30, 90), intensity: 150 };
    case 'color': return { exposure: .4, contrast: 20, saturation: 50, hue: k(0, 90), temperature: 30 };
    case 'levels': return { inBlack: 10, inWhite: k(100, 70), gamma: 1.4 };
    case 'duotone': return { shadow: k('#1B2A4A', '#5A0A2A'), highlight: '#FFD66B', amount: 80 };
    case 'grain': return { amount: k(10, 40), size: 2 };
    case 'vignette': return { amount: k(30, 90), feather: 40 };
    case 'chroma': return { amount: k(0, 20) };
    case 'pixelate': return { size: k(4, 24) };
    case 'posterize': return { levels: k(16, 3) };
    case 'displace': return { amount: k(10, 40), scale: 3, speed: 1 };
    case 'shadow': return { distance: k(5, 40), angle: 135, softness: 20, opacity: 80, color: '#000000' };
    case 'invert': return { amount: k(0, 100) };
    case 'gradient': return { startColor: '#FF6B1A', endColor: '#1B2A4A', angle: k(0, 180), midpoint: 40, radial: false, amount: 60 };
    default: throw new Error(`No corpus values for effect ${id}`);
  }
}

function effectProject(id: string): CorpusProject {
  const b = newProject(`effects/${id}`, { name: `Effect ${id}`, backgroundFill: GRADIENT });
  const [art, , , , , , title] = testCard(b);
  art!.fx.push(b.effect(id, effectValues(b, id)));
  // A second instance on text exercises padding and bounds growth on a smaller layer.
  title!.fx.push(b.effect(id, effectValues(b, id)));
  const covers = [`effect ${id}`, 'keyframed effect parameter'];
  if (id === 'blur') {
    art!.fx.push(b.effect('color', { saturation: -100 }, b.ch(true, [[0, true], [1.5, false]])));
    covers.push('animatable effect enable', 'effect stacking');
  }
  if (id === 'gradient') {
    title!.fx[0] = b.effect('gradient', { radial: true, shade: true, centerX: 20, centerY: -10, spread: 150, dither: 4 });
    covers.push('toggle parameters as float');
  }
  return project(b, { group: 'Effects and transitions', title: `Effect ${id}`, covers, samples: [0, 1.25, 2.5], alpha: id === 'shadow' || id === 'glow' });
}

function transitionProject(id: string): CorpusProject {
  const b = newProject(`transitions/${id}`, { name: `Transition ${id}`, backgroundFill: GRADIENT });
  const definition = transitions.find(t => t.id === id)!;
  const values = Object.fromEntries(definition.params.map(param => {
    if (param.k === 'angle') return [param.k, 30];
    if (param.k === 'softness') return [param.k, 20];
    if (param.k === 'direction') return [param.k, 90];
    if (param.k === 'amount') return [param.k, 60];
    return [param.k, param.def as Value];
  }));
  const card = testCard(b);
  const art = card[0]!;
  art.from = .2; art.dur = 2.6;
  art.transitionIn = b.transition(id, .8, values);
  art.transitionOut = b.transition(id, .6, values);
  const title = card[6]!;
  title.transitionIn = b.transition(id, 1.2);
  return project(b, {
    group: 'Effects and transitions', title: `Transition ${id}`,
    covers: [`transition ${id}`, 'transition in', 'transition out', 'transition parameters'],
    samples: [.3, .6, .9, 1.5, 2.4, 2.7], alpha: true,
  });
}

function extensionEffectProjects(): CorpusProject[] {
  const typewriter = newProject('effects/extension-typewriter', { name: 'Extension effect: Typewriter', backgroundFill: GRADIENT });
  const card = testCard(typewriter);
  const title = card[6]!;
  title.fx.push(typewriter.foreignEffect('typewriter', {
    progress: typewriter.ch(0, [[0, 0], [2.5, 100]]), cursor: true, blinkRate: 0, gradAngle: typewriter.ch(90, [[0, 90], [2.5, 270]]),
  }));
  const second = typewriter.add(typewriter.layer('text', {
    name: 'Second typed line', d: { text: 'typed by an extension', size: 30, weight: 500, color: '#E8E2CF', align: 'left' },
    p: { 'position.x': 30, 'position.y': 24 },
  }));
  second.fx.push(typewriter.foreignEffect('typewriter', { progress: 60, cursor: false }), typewriter.effect('glow', { radius: 40, intensity: 120 }));
  const missing = newProject('effects/missing-definition', { name: 'Missing effect definition', backgroundFill: GRADIENT });
  const [art, , , , , , label] = testCard(missing);
  art!.fx.push(missing.foreignEffect('com.example.missing-word-swap', { progress: missing.ch(0, [[0, 0], [2, 100]]), tint: '#FF6B1A' }));
  label!.fx.push(missing.effect('blur', { amount: 6 }), missing.foreignEffect('com.example.missing-wobble', { amount: 4 }, missing.ch(true, [[0, true], [1, false]])));
  return [
    project(typewriter, {
      group: 'Effects and transitions', title: 'Extension effect (Typewriter fixture extension)',
      covers: ['extension effects', 'extension raster hooks', 'extension effect stacked with a built-in'], samples: [0, .8, 1.6, 2.5],
      extensions: ['compat-typewriter'], alpha: true,
    }),
    project(missing, {
      group: 'Effects and transitions', title: 'Missing effect definitions',
      covers: ['missing definition behavior', 'animated parameters kept on a missing effect', 'animatable enable on a missing effect'], samples: [0, 1.5],
      notes: 'core/validate sanitizeProject drops non-built-in effects; the app loader keeps them as missing (pass-through).',
    }),
  ];
}

export function effectProjects(): CorpusProject[] {
  if (effects.length !== 16) throw new Error(`Expected 16 built-in effects, found ${effects.length}`);
  if (transitions.length !== 4) throw new Error(`Expected 4 built-in transitions, found ${transitions.length}`);
  return [...effects.map(effect => effectProject(effect.id)), ...transitions.map(transition => transitionProject(transition.id)), ...extensionEffectProjects()];
}
