import { newProject } from '../doc';
import { GRADIENT, project, type CorpusProject } from './common';

function primitives(): CorpusProject {
  const b = newProject('shapes/primitives', { name: 'Shape primitives', backgroundFill: GRADIENT });
  const shapes: Array<[string, Record<string, unknown>]> = [
    ['Rect radius', { shape: 'rect', w: 120, h: 90, radius: 20, color: '#FF6B1A' }],
    ['Rect per-corner', { shape: 'rect', w: 120, h: 90, independentCorners: true, radiusTL: 0, radiusTR: 30, radiusBR: 8, radiusBL: 45, color: '#4C8DFF' }],
    ['Rect smoothing', { shape: 'rect', w: 120, h: 90, radius: 30, smoothing: 80, color: '#3FCF8E' }],
    ['Ellipse stroked', { shape: 'ellipse', w: 120, h: 80, color: '#F2C14E', stroke: 6, strokeColor: '#FFFFFF' }],
    ['Polygon', { shape: 'polygon', w: 110, h: 110, points: 6, color: '#A73366' }],
    ['Star', { shape: 'star', w: 120, h: 120, points: 7, color: '#E8E2CF', stroke: 3, strokeColor: '#FF6B1A' }],
    ['Line', { shape: 'line', w: 140, h: 0, stroke: 8, strokeColor: '#4C8DFF', color: '#4C8DFF' }],
    ['Stroke only', { shape: 'rect', w: 110, h: 110, radius: 14, color: '#00000000', stroke: 4, strokeColor: '#3FCF8E' }],
  ];
  shapes.forEach(([name, d], index) => b.add(b.layer('shape', { name, d, p: { 'position.x': 90 + (index % 4) * 150, 'position.y': 100 + Math.floor(index / 4) * 160 } })));
  b.add(b.layer('shape', {
    name: 'Animated size and radius', d: { shape: 'rect', color: '#FFFFFF', w: b.ch(40, [[0, 40], [2.5, 160]]), h: 30, radius: b.ch(0, [[0, 0], [2.5, 15]]), points: 5 },
    p: { 'position.x': 320, 'position.y': 330, opacity: 70 },
  }));
  return project(b, { group: 'Shapes and paths', title: 'Shape primitives', covers: ['rect', 'per-corner radius', 'smoothing', 'ellipse', 'polygon', 'star (0.46 inner ratio)', 'line', 'stroke', 'animated shape content'], samples: [0, 1.25, 2.5], alpha: true });
}

function paths(): CorpusProject[] {
  const b = newProject('shapes/editable-paths', { name: 'Editable paths', backgroundFill: GRADIENT });
  const layer = b.add(b.layer('shape', { name: 'Paths', d: { shape: 'rect', w: 480, h: 480, color: '#E8E2CF', radius: 0, stroke: 0, strokeColor: '#ffffff', points: 5 } }));
  const wave = b.path('Wave', [[-200, 0, 0, 0, 60, -90], [0, 0, -60, 90, 60, -90], [200, 0, -60, 90, 0, 0]], {
    fillEnabled: false, stroke: '#4C8DFF', strokeWidth: 8, trimEnd: b.ch(0, [[0, 0], [2, 100]]), trimOffset: b.ch(0, [[0, 0], [2.5, 90]]), y: -90,
  });
  const badge = b.path('Badge', [[-60, -40, 0, 0, 30, 0], [60, -40, -30, 0, 0, 0], [60, 40], [-60, 40]], {
    closed: true, fill: '#FF6B1A', fillOpacity: 80, stroke: '#FFFFFF', strokeWidth: 3, strokeOpacity: 60, x: -150, y: 80,
  });
  const repeated = b.path('Repeated petal', [[0, 0, 0, 0, 20, -30], [0, -70, 20, 0, -20, 0]], {
    closed: true, fill: '#3FCF8E', copies: b.ch(1, [[0, 1], [2.5, 12]]), repeatX: 0, repeatY: 0, repeatRotation: 30, x: 150, y: 80, stroke: '#0C0D12', strokeWidth: 1,
  });
  const child = b.path('Child of badge', [[-20, -20], [20, -20], [20, 20], [-20, 20]], { closed: true, fill: '#1B2A4A', rotation: b.ch(0, [[0, 0], [2.5, 180]]) });
  child.parent = badge.id;
  const many = b.path('Repeater at 256', [[0, 0], [6, 0], [6, 6]], { closed: true, fill: '#F2C14E', copies: 300, repeatX: 2, repeatY: .4, repeatRotation: 1.4, x: -230, y: -170 });
  layer.d.paths = [wave, badge, child, repeated, many];
  const svgPaths = newProject('shapes/svg-import-paths', { name: 'SVG-style compound path', backgroundFill: GRADIENT });
  const icon = svgPaths.add(svgPaths.layer('shape', { name: 'Imported icon paths', d: { shape: 'rect', w: 200, h: 200, color: '#E8E2CF', radius: 0, stroke: 0, strokeColor: '#ffffff', points: 5 }, p: { 'scale.x': 160, 'scale.y': 160 } }));
  icon.d.paths = [
    svgPaths.path('Outer', [[-50, -50, 0, 0, 0, 0], [50, -50], [50, 50], [-50, 50]], { closed: true, fill: '#4C8DFF' }),
    svgPaths.path('Inner curve', [[-30, 10, 0, 0, 10, -40], [30, 10, -10, -40, 0, 0], [0, 35]], { closed: true, fill: '#FFFFFF', fillOpacity: 90 }),
  ];
  return [
    project(b, { group: 'Shapes and paths', title: 'Editable cubic paths', covers: ['editable paths', 'cubic segments', 'fill and stroke opacity', 'trim start/end/offset', 'repeater', 'repeater cap at 256', 'path group parenting'], samples: [0, 1, 2.5], alpha: true }),
    project(svgPaths, { group: 'Shapes and paths', title: 'Compound paths as SVG import produces them', covers: ['paths from SVG import', 'closed paths'], samples: [0] }),
  ];
}

export function shapeProjects(): CorpusProject[] {
  return [primitives(), ...paths()];
}
