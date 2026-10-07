import type { EffectParamDefinition, ParameterWidget } from 'powermove';

const numeric = (p?: EffectParamDefinition) => !!p && p.type !== 'color' && p.type !== 'toggle' && Number.isFinite((p as any).min) && Number.isFinite((p as any).max) && (p as any).min < (p as any).max;
const secondary = /^(dither|seed|quality|shade|keepLuminance)$/i;

/** Old definitions get useful layouts too; invalid hints never swallow a parameter. */
export function parameterLayout(params: EffectParamDefinition[], authored?: ParameterWidget[]) {
  const byKey = new Map(params.map(p=>[p.k,p]));
  const hints: ParameterWidget[] = [...(Array.isArray(authored) ? authored : [])];
  if (!hints.some(w=>w?.kind==='ramp')) {
    for (const [start,end] of [['startColor','endColor'],['shadow','highlight']] as const) {
      if (byKey.get(start)?.type==='color' && byKey.get(end)?.type==='color') {
        hints.push({kind:'ramp',start,end,midpoint:numeric(byKey.get('midpoint'))?'midpoint':undefined,mode:byKey.get('radial')?.type==='toggle'?'radial':undefined}); break;
      }
    }
  }
  for (const p of params) {
    if (!/X$/.test(p.k) || !numeric(p)) continue;
    const y=p.k.slice(0,-1)+'Y';
    if (numeric(byKey.get(y)) && !hints.some(w=>w?.kind==='point' && (w.x===p.k || w.y===y))) hints.push({kind:'point',x:p.k,y,label:p.label.replace(/\s*X$/i,''),advanced:p.advanced ?? byKey.get(y)?.advanced});
  }
  const used = new Set<string>(), widgets: ParameterWidget[]=[];
  for (const hint of hints) {
    if (!hint || typeof hint!=='object') continue;
    const keys=hint.kind==='ramp'?[hint.start,hint.end,hint.midpoint,hint.mode].filter((k):k is string=>!!k):hint.kind==='point'?[hint.x,hint.y]:[];
    if (!keys.length || new Set(keys).size!==keys.length || keys.some(k=>used.has(k)||!byKey.has(k))) continue;
    if (hint.kind==='ramp' && (byKey.get(hint.start)?.type!=='color'||byKey.get(hint.end)?.type!=='color'||(hint.midpoint&&!numeric(byKey.get(hint.midpoint)))||(hint.mode&&byKey.get(hint.mode)?.type!=='toggle'))) continue;
    if (hint.kind==='point' && (!numeric(byKey.get(hint.x))||!numeric(byKey.get(hint.y)))) continue;
    keys.forEach(k=>used.add(k)); widgets.push(hint);
  }
  // Ramp colors remain independently editable and keyframeable below the bar.
  const hidden=new Set(widgets.flatMap(w=>w.kind==='ramp'?[w.midpoint,w.mode].filter((k):k is string=>!!k):[w.x,w.y]));
  let visible=0;
  const primary: EffectParamDefinition[]=[], more: EffectParamDefinition[]=[];
  for (const param of params) {
    if (hidden.has(param.k)) { if (widgets.some(w=>w.kind==='ramp'&&w.midpoint===param.k)) more.push(param); continue; }
    const rampColor=widgets.some(w=>w.kind==='ramp'&&(w.start===param.k||w.end===param.k));
    const advanced=!rampColor && (param.advanced ?? (secondary.test(param.k)||(visible>=5&&!/^(amount|mix)$/i.test(param.k))));
    if (advanced) more.push(param); else {primary.push(param); visible++;}
  }
  primary.sort((a,b)=>Number(/^(amount|mix)$/i.test(a.k))-Number(/^(amount|mix)$/i.test(b.k)));
  return { widgets, primary, more };
}
