import { isProperty } from '../legacy/core/content-properties';

/** Conservative: unsupported source must choose WebGL, never disappear. */
export function inspectSvgExport(project: any) {
  const reasons = new Set<string>();
  const fixed = (v: any, expected: any) => v === undefined || (isProperty(v) ? !v.expr && !v.kf.length && v.v === expected : v === expected);
  if (project.backgroundFill && project.backgroundFill.type !== 'solid') reasons.add('Composition background: gradient rendering requires WebGL');
  for (const layer of project.layers || []) {
    const label = layer.name || layer.id;
    const reject = (reason: string) => reasons.add(`${label}: ${reason}`);
    if (!['text', 'solid', 'shape', 'group'].includes(layer.type)) reject(`${layer.type} layers require WebGL`);
    if (layer.blend && layer.blend !== 'normal') reject('blend mode requires WebGL');
    if (layer.mblur) reject('motion blur requires WebGL');
    if (layer.masks?.length || layer.matte || layer.trackMatte) reject('masks or mattes require WebGL');
    if (layer.transitionIn || layer.transitionOut) reject('transitions require WebGL');
    if (layer.fx?.some((fx: any) => !fx.missing && !fixed(fx.on, false))) reject('effects require WebGL');
    for (const key of ['position.z', 'anchor.z', 'rotation.x', 'rotation.y', 'orientation.x', 'orientation.y', 'orientation.z']) {
      if (!fixed(layer.p?.[key], 0)) reject('3D transforms require WebGL');
    }
    if (!fixed(layer.p?.['scale.z'], 100)) reject('3D scale requires WebGL');
    if (layer.type === 'text') {
      if (layer.d.paragraph || layer.d.styles?.length || layer.d.animators?.length) reject('paragraph/rich text or character animators require WebGL');
      for (const key of Object.keys(layer.d)) if (key.startsWith('fontAxis.') && key !== 'fontAxis.wght') reject('custom font axes require WebGL');
    }
    if (layer.type === 'shape') {
      if (layer.d.paths?.length || !['rect', 'ellipse'].includes(layer.d.shape)) reject('this shape geometry requires WebGL');
      if (layer.d.fill && layer.d.fill.type !== 'solid') reject('gradient fills require WebGL');
    }
  }
  return { supported: reasons.size === 0, reasons: [...reasons] };
}
