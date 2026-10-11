import { captionModel } from '../catalog';
import { newProject } from '../doc';
import { GRADIENT, project, type CorpusProject } from './common';

function textProperties(): CorpusProject {
  const b = newProject('text/properties', { name: 'Text properties', backgroundFill: GRADIENT });
  const rows: Array<[string, Record<string, unknown>]> = [
    ['SF Pro Display 600, centered', { text: 'Powermove', font: 'SF Pro Display', weight: 600, size: 48 }],
    ['Helvetica Neue light, left, tracking', { text: 'Light and wide', font: 'Helvetica Neue', weight: 300, size: 36, tracking: 12, align: 'left' }],
    ['Georgia italic, right', { text: 'Serif italic', font: 'Georgia', weight: 400, italic: true, size: 40, align: 'right' }],
    ['Menlo, mono', { text: 'mono 0123456789', font: 'Menlo', weight: 400, size: 28, color: '#3FCF8E' }],
  ];
  rows.forEach(([name, d], index) => b.add(b.layer('text', { name, d, p: { 'position.x': 320, 'position.y': 50 + index * 62 } })));
  b.add(b.layer('text', {
    name: 'Wrapped paragraph in a box',
    d: { text: 'Paragraph text wraps inside its box and keeps its leading and alignment across lines.', size: 18, weight: 500, leading: 1.4, align: 'left', boxWidth: b.ch(260, [[0, 260], [2.5, 420]]), boxHeight: b.ch(0) },
    // Box text hangs right and down from its position.
    p: { 'position.x': 40, 'position.y': 272 },
  }));
  b.add(b.layer('text', {
    name: 'Animated content', d: {
      text: b.ch('One', [[0, 'One'], [1, 'Two'], [2, 'Three']]),
      size: b.ch(30, [[0, 30], [2.5, 60]]), color: b.ch('#FF6B1A', [[0, '#FF6B1A'], [2.5, '#4C8DFF']]),
    },
    p: { 'position.x': 540, 'position.y': 300 },
  }));
  return project(b, {
    group: 'Text', title: 'Text properties', covers: ['fonts', 'weight', 'italic', 'size', 'tracking', 'leading', 'alignment', 'box wrapping', 'animated text string (steps)', 'animated size', 'color interpolation'],
    samples: [0, 1.25, 2.5], alpha: true,
  });
}

function variableAxes(): CorpusProject {
  const b = newProject('text/variable-axes', { name: 'Variable font axes', backgroundFill: GRADIENT });
  b.add(b.layer('text', { name: 'Weight axis', d: { text: 'Variable weight', font: 'SF Pro', size: 54, 'fontAxis.wght': b.ch(100, [[0, 100], [2.5, 900]]) }, p: { 'position.y': 120 } }));
  b.add(b.layer('text', { name: 'Width axis', d: { text: 'Variable width', font: 'SF Pro', size: 54, weight: 500, 'fontAxis.wdth': b.ch(30, [[0, 30], [2.5, 150]]) }, p: { 'position.y': 240 } }));
  return project(b, { group: 'Text', title: 'Variable font axes', covers: ['variable axes (fontAxis.wght, fontAxis.wdth)'], samples: [0, 1.25, 2.5],
    notes: 'Uses the system SF Pro family; if the installed face lacks an axis the legacy renderer ignores it, and the capture records what it drew.' });
}

function rangeStyles(): CorpusProject {
  const b = newProject('text/range-styles', { name: 'Text range styles', backgroundFill: GRADIENT });
  const layer = b.add(b.layer('text', { name: 'Styled ranges', d: { text: 'Range styles mix weights and colors', size: 40, weight: 400 } }));
  layer.d.styles = [
    { id: b.id('ts'), start: 0, end: 5, p: { color: b.ch('#FF6B1A'), weight: b.ch(800), size: b.ch(52) } },
    { id: b.id('ts'), start: 13, end: 25, p: { color: b.ch('#3FCF8E', [[0, '#3FCF8E'], [2, '#F2C14E']]), weight: b.ch(700), size: b.ch(40) } },
  ];
  return project(b, { group: 'Text', title: 'Text range styles', covers: ['range styles', 'animated range style color'], samples: [0, 2] });
}

function animators(): CorpusProject[] {
  const presets: Array<[string, string, string]> = [
    ['text/animator-rise', 'rise', 'Characters rise in'],
    ['text/animator-rise-words', 'rise-words', 'Words rise in one by one'],
    ['text/animator-pop', 'pop', 'Pop with back easing'],
    ['text/animator-spring-drop', 'drop', 'Spring drop'],
    ['text/animator-tumble', 'tumble', 'Tumble in'],
    ['text/animator-blur', 'blur', 'Blur in'],
    ['text/animator-tracking', 'tracking', 'Tracking in'],
    ['text/animator-typewriter', 'typewriter', 'Typewriter'],
    ['text/animator-wave', 'wave', 'Wave loop'],
    ['text/animator-pulse', 'pulse', 'Pulse loop'],
    ['text/animator-range', 'custom-range', 'Range selector sweep'],
  ];
  return presets.map(([slug, preset, text]) => {
    const b = newProject(slug, { name: `Animator ${preset}`, backgroundFill: GRADIENT });
    const layer = b.add(b.layer('text', { name: text, d: { text: text + '\nsecond line', size: 44, weight: 700, leading: 1.2 } }));
    const animator = b.animator(preset, 44);
    if (preset === 'custom-range') {
      animator.p.offset = b.ch(-100, [[0, -100], [2.5, 100]]);
      animator.p.smoothness = b.ch(40);
      animator.shape = 'triangle';
      animator.p.color = b.ch('#FF6B1A');
      animator.p.y = b.ch(-20);
    }
    layer.d.animators = [animator];
    return project(b, { group: 'Text', title: `Text animator ${preset}`, covers: ['text animators', `animator preset ${preset}`, `mode ${animator.mode}`, `unit ${animator.unit}`], samples: [.1, .4, .8, 1.6, 2.5] });
  });
}

function captions(): CorpusProject[] {
  const lines: Array<[number, number, string, boolean?]> = [
    [0, .9, 'Captions follow the spoken words', true],
    [.9, 1.8, 'with highlight and pop animation', true],
    [1.8, 2.9, 'and a long cue that wraps across more than two lines when the maximum width is narrow', true],
  ];
  const variants: Array<[string, Record<string, unknown>, string]> = [
    ['captions/classic', {}, 'classic'],
    ['captions/box-highlight-pop', { box: true, boxColor: '#101014', boxOpacity: 80, boxPadding: 14, boxRadius: 12, highlight: true, highlightColor: '#FFD66B', highlightScale: 120, wordAnimation: 'pop', placement: 'bottom' }, 'classic'],
    ['captions/fade-top-upper', { wordAnimation: 'fade', placement: 'top', textCase: 'upper', stroke: 0, offsetX: 40, offsetY: 10, safeMargin: 8 }, 'classic'],
    ['captions/middle-lines', { placement: 'middle', maxLines: 1, maxWidth: 40, align: 'left', textCase: 'lower', stroke: 6, strokeColor: '#4C8DFF' }, 'classic'],
  ];
  const presets = captionModel.presets;
  const out = variants.map(([slug, style, preset]) => {
    const b = newProject(slug, { name: slug, backgroundFill: GRADIENT });
    b.add(b.layer('captions', { name: 'Captions', d: b.captions(lines, style, preset) }));
    return project(b, { group: 'Captions', title: `Captions ${slug.split('/')[1]}`, covers: ['captions', ...Object.keys(style).map(key => `caption ${key}`)], samples: [.2, .7, 1.3, 2.4], alpha: slug.endsWith('box-highlight-pop') });
  });
  for (const preset of presets) {
    const slug = `captions/preset-${preset}`;
    const b = newProject(slug, { name: `Caption preset ${preset}`, backgroundFill: GRADIENT });
    b.add(b.layer('captions', { name: 'Captions', d: b.captions(lines, {}, preset) }));
    out.push(project(b, { group: 'Captions', title: `Caption preset ${preset}`, covers: ['captions', `caption preset ${preset}`], samples: [.5, 1.3] }));
  }
  return out;
}

export function textProjects(): CorpusProject[] {
  return [textProperties(), variableAxes(), rangeStyles(), ...animators(), ...captions()];
}
