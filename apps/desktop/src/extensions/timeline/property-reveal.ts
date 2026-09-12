/** Timeline presentation only: these shortcuts never mutate animation data. */
export const propertyShortcuts = [
  ['p', 'revealPos', 'Position'], ['s', 'revealScale', 'Scale'],
  ['r', 'revealRot', 'Rotation'], ['t', 'revealOpacity', 'Opacity'],
  ['a', 'revealAnchor', 'Anchor point'], ['u', 'revealKeys', 'Animated properties'],
  ['m', 'revealMasks', 'Mask controls'], ['f', 'revealFeather', 'Mask feather'],
  ['e', 'revealEffects', 'Effects'], ['l', 'revealAudio', 'Audio levels'],
] as const;

export function revealedProperties(PM: any, layer: any, props: any[]): any[] {
  const reveal = PM.UIState.getReveal(layer);
  return props.filter(row => reveal == null ? row.prop.kf.length > 0
    : reveal.includes('*') || (row.channels ?? [row]).some((axis: any) => reveal.includes(axis.key)));
}

function keysFor(PM: any, layer: any, key: string): string[] {
  const props = PM.allProps(layer);
  const effects = new Set((layer.fx || []).map((fx: any) => fx.id));
  const selected = new Set(PM.sel.keys || []);
  let defaults: Map<string, any> | undefined;
  if (key === 'uu' && PM.mkLayer) defaults = new Map(PM.allProps(PM.mkLayer(layer.type)).map((p: any) => [p.key, p.prop.v]));
  return props.filter((p: any) => {
    const k = p.key;
    switch (key) {
      case 'p': return k.startsWith('position.');
      case 's': return k.startsWith('scale.');
      case 'r': return /^(rotation|orientation)(\.|$)/.test(k);
      case 'a': return k.startsWith('anchor.');
      case 't': return k === 'opacity';
      case 'u': return p.prop.kf.length || p.prop.expr;
      case 'uu': return p.prop.kf.length || p.prop.expr || (defaults && (!defaults.has(k) || JSON.stringify(defaults.get(k)) !== JSON.stringify(p.prop.v)));
      case 'm': return /^m\..+\.(shape|x|y|w|h|rotation)$/.test(k);
      case 'mm': return k.startsWith('m.');
      case 'f': return k.startsWith('m.') && k.endsWith('.feather');
      case 'tt': return k.startsWith('m.') && k.endsWith('.opacity');
      case 'e': return effects.has(k.split('.')[0]);
      case 'ee': return !!p.prop.expr;
      case 'l': return k === 'c.gain';
      // Waveforms already live on audio strips; LL hides the property rows.
      case 'll': return false;
      case 'rr': return k === 'c.timeRemap' || k === 'c.sourceTime';
      case 'ss': return k === PM.sel.chan || (PM.sel.chan === 'scale' && k.startsWith('scale.')) || p.prop.kf.some((kf: any) => selected.has(kf.i));
      case 'aa': return p.group === 'Material Options';
      case 'pp': return /^(Paint|Puppet|Roto Brush)/.test(p.group || '');
      case 'ff': return (layer.fx || []).some((fx: any) => fx.id === k.split('.')[0] && !PM.FX?.[fx.type]);
      default: return false;
    }
  }).map((p: any) => p.key);
}

export function createPropertyReveal(PM: any) {
  let last: { key: string; shift: boolean; time: number; layers: any[]; before: any[] } | undefined;
  return (key: string, shift = false) => {
    const selected = PM.selLayers();
    const layers = selected.length ? selected : PM.proj.layers;
    if (!layers.length) return;
    const now = performance.now();
    const before = layers.map((L: any) => ({ collapsed: PM.UIState.getLayerCollapsed(L), reveal: PM.UIState.getReveal(L) }));
    // M without selection is an immediate, repeatable global disclosure toggle.
    if (key === 'all' || (key === 'm' && !selected.length)) {
      const close = layers.every((L: any) => !PM.UIState.getLayerCollapsed(L));
      layers.forEach((L: any) => { PM.UIState.setReveal(L, ['*']); PM.UIState.setLayerCollapsed(L, close); });
      last = undefined;
    } else {
      const double = last && last.key === key && last.shift === shift && now - last.time < 300
        && layers.length === last.layers.length && layers.every((L: any, i: number) => L === last!.layers[i]);
      const base = double ? last!.before : before;
      const mode = double ? key + key : key;
      const targets = layers.map((L: any) => keysFor(PM, L, mode));
      const close = !shift && targets.every((keys: string[], i: number) => !base[i].collapsed
        && JSON.stringify([...(base[i].reveal || [])].sort()) === JSON.stringify([...keys].sort()));
      layers.forEach((L: any, i: number) => {
        let keys = targets[i];
        if (shift) {
          const current = base[i].collapsed ? [] : base[i].reveal?.includes('*') ? PM.allProps(L).map((p: any) => p.key)
            : base[i].reveal ?? PM.allProps(L).filter((p: any) => p.prop.kf.length).map((p: any) => p.key);
          keys = keys.length && keys.every((k: string) => current.includes(k)) ? current.filter((k: string) => !keys.includes(k)) : [...new Set([...current, ...keys])];
        }
        PM.UIState.setReveal(L, keys);
        PM.UIState.setLayerCollapsed(L, close || (shift && !keys.length));
        for (const group of PM.groupAncestors?.(L) || []) PM.UIState.setGroupCollapsed(group, false);
      });
      last = double ? undefined : { key, shift, time: now, layers: [...layers], before };
    }
    PM.bus.emit('layers');
    PM.invalidate('timeline');
  };
}
