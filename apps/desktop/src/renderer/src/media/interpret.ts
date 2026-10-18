/*
 * Interpret Footage (plan section 9, "Per-source interpretation"; appendix D,
 * asset right-click menu): the model behind the sheet. An asset's
 * `interpret` member holds only the overrides; every member left out reads
 * as the file says, or as section 9's default when the file says nothing
 * (pm-media `interpret.rs` parses the same shape). The job engine reports
 * each setting's value, automatic value, and origin (`MediaFacts`).
 */
import type { FactOrigin, InterpretSetting, MediaFacts } from '../../../shared/engine-protocol';

/** The members Interpret Footage writes, in the order the sheet shows them. */
export type InterpretMember =
  | 'primaries' | 'transfer' | 'matrix' | 'range' | 'siting'
  | 'alpha' | 'frameRate' | 'fieldOrder' | 'pixelAspect' | 'cleanAperture';

/** The stored `interpret` member (`asset.interpret`). */
export type Interpretation = {
  primaries?: string;
  transfer?: string;
  matrix?: string;
  range?: string;
  siting?: string;
  alpha?: string;
  alphaMatte?: string;
  frameRate?: number;
  frameRateRational?: [number, number];
  fieldOrder?: string;
  pixelAspect?: [number, number];
  cleanAperture?: boolean;
};

/** A draft: one value per member in the facts' string form, or `null` for
 *  Automatic. */
export type Draft = Partial<Record<InterpretMember, string | null>> & { alphaMatte?: string };

export const AUTOMATIC = '';

/** Choices per member: [stored value in the facts' form, label]. */
export const CHOICES: Record<InterpretMember, ReadonlyArray<readonly [string, string]>> = {
  primaries: [
    ['bt709', 'Rec. 709'],
    ['bt601_525', 'Rec. 601 (NTSC)'],
    ['bt601_625', 'Rec. 601 (PAL)'],
    ['displayP3', 'Display P3'],
    ['bt2020', 'Rec. 2020']
  ],
  transfer: [
    ['displayE', 'Standard (sRGB curve)'],
    ['bt1886', 'BT.1886 (gamma 2.4)'],
    ['sceneReferredBt709', 'Scene-referred (inverse BT.709)'],
    ['gamma22', 'Gamma 2.2'],
    ['linear', 'Linear'],
    ['pq', 'PQ (HDR10)'],
    ['hlg', 'HLG']
  ],
  matrix: [
    ['bt709', 'Rec. 709'],
    ['bt601', 'Rec. 601'],
    ['bt2020Ncl', 'Rec. 2020'],
    ['identity', 'RGB (no matrix)']
  ],
  range: [
    ['limited', 'Limited (video)'],
    ['full', 'Full']
  ],
  siting: [
    ['left', 'Left'],
    ['center', 'Center'],
    ['topLeft', 'Top left'],
    ['top', 'Top'],
    ['bottomLeft', 'Bottom left'],
    ['bottom', 'Bottom']
  ],
  alpha: [
    ['straight', 'Straight'],
    ['premultiplied', 'Premultiplied'],
    ['premultipliedWithColor', 'Premultiplied with color'],
    ['ignore', 'Ignore']
  ],
  frameRate: [
    ['24000/1001', '23.976 fps'],
    ['24/1', '24 fps'],
    ['25/1', '25 fps'],
    ['30000/1001', '29.97 fps'],
    ['30/1', '30 fps'],
    ['48000/1001', '47.952 fps'],
    ['48/1', '48 fps'],
    ['50/1', '50 fps'],
    ['60000/1001', '59.94 fps'],
    ['60/1', '60 fps'],
    ['120000/1001', '119.88 fps'],
    ['120/1', '120 fps']
  ],
  fieldOrder: [
    ['progressive', 'Progressive'],
    ['topFirst', 'Top field first'],
    ['bottomFirst', 'Bottom field first']
  ],
  pixelAspect: [
    ['1:1', 'Square'],
    ['10:11', 'D1/DV NTSC (0.91)'],
    ['40:33', 'D1/DV NTSC widescreen (1.21)'],
    ['59:54', 'D1/DV PAL (1.09)'],
    ['118:81', 'D1/DV PAL widescreen (1.46)'],
    ['4:3', 'HDV 1080, DVCPRO HD 720 (1.33)'],
    ['3:2', 'DVCPRO HD 1080 (1.5)'],
    ['2:1', 'Anamorphic 2:1']
  ],
  cleanAperture: [
    ['false', 'Full frame'],
    ['true', 'Clean aperture']
  ]
};

export const LABELS: Record<InterpretMember, string> = {
  primaries: 'Primaries',
  transfer: 'Transfer',
  matrix: 'Matrix',
  range: 'Range',
  siting: 'Chroma siting',
  alpha: 'Alpha',
  frameRate: 'Frame rate',
  fieldOrder: 'Fields',
  pixelAspect: 'Pixel aspect',
  cleanAperture: 'Clean aperture'
};

export const SECTIONS: ReadonlyArray<{ title: string; members: InterpretMember[] }> = [
  { title: 'Color', members: ['primaries', 'transfer', 'matrix', 'range', 'siting'] },
  { title: 'Alpha', members: ['alpha'] },
  { title: 'Time', members: ['frameRate', 'fieldOrder'] },
  { title: 'Geometry', members: ['pixelAspect', 'cleanAperture'] }
];

const gcd = (a: number, b: number): number => (b ? gcd(b, a % b) : Math.abs(a) || 1);

/** A value's label: the choice's, or a readable form of an unlisted value. */
export function valueLabel(member: InterpretMember, value: string): string {
  const hit = CHOICES[member].find(([v]) => v === value);
  if (hit) return hit[1];
  if (member === 'frameRate') {
    const [n, d] = value.split('/').map(Number);
    if (n && d) return `${Math.round((n / d) * 1000) / 1000} fps`;
  }
  if (member === 'pixelAspect') {
    const [n, d] = value.split(':').map(Number);
    if (n && d) return `${value} (${Math.round((n / d) * 100) / 100})`;
  }
  return value;
}

/** The choices a row offers: the member's list, plus the automatic and
 *  current values when the list does not have them. */
export function choicesFor(member: InterpretMember, setting: InterpretSetting | undefined): Array<readonly [string, string]> {
  const list: Array<readonly [string, string]> = [...CHOICES[member]];
  for (const value of [setting?.automatic, setting?.value]) {
    if (value && !list.some(([v]) => v === value)) list.push([value, valueLabel(member, value)]);
  }
  return list;
}

/** "Automatic (Rec. 709)". */
export function automaticLabel(member: InterpretMember, setting: InterpretSetting | undefined): string {
  return setting ? `Automatic (${valueLabel(member, setting.automatic)})` : 'Automatic';
}

/** Where a value comes from, in a sentence for the row's secondary line. */
export function originText(origin: FactOrigin | undefined, automatic?: string, member?: InterpretMember): string {
  if (!origin) return '';
  switch (origin.kind) {
    case 'file': return `From the file (${origin.source})`;
    case 'default': return origin.rule;
    case 'override': return automatic && member ? `Overridden · the file reads ${valueLabel(member, automatic)}` : 'Overridden';
  }
}

/** The draft of an asset's stored member. */
export function draftOf(stored: unknown): Draft {
  const draft: Draft = {};
  if (!stored || typeof stored !== 'object' || Array.isArray(stored)) return draft;
  const s = stored as Interpretation;
  for (const key of ['primaries', 'transfer', 'matrix', 'range', 'siting', 'alpha', 'fieldOrder'] as const) {
    const v = s[key];
    if (typeof v === 'string' && v && v !== 'auto') draft[key] = v;
  }
  if (Array.isArray(s.frameRateRational) && s.frameRateRational.length === 2) {
    const [n, d] = s.frameRateRational;
    if (Number.isSafeInteger(n) && Number.isSafeInteger(d) && n > 0 && d > 0) draft.frameRate = `${n}/${d}`;
  } else if (typeof s.frameRate === 'number' && s.frameRate > 0) {
    draft.frameRate = rateOf(s.frameRate);
  }
  if (Array.isArray(s.pixelAspect) && s.pixelAspect.length === 2) draft.pixelAspect = `${s.pixelAspect[0]}:${s.pixelAspect[1]}`;
  if (typeof s.cleanAperture === 'boolean') draft.cleanAperture = String(s.cleanAperture);
  if (typeof s.alphaMatte === 'string') draft.alphaMatte = s.alphaMatte;
  return draft;
}

/** A float rate as the rational the engine snaps it to (NTSC and integer rates). */
function rateOf(fps: number): string {
  for (const k of [24, 30, 48, 60, 120, 240]) {
    const ntsc = (k * 1000) / 1001;
    if (Math.abs(fps - ntsc) <= 0.0002 * ntsc) return `${k * 1000}/1001`;
  }
  const r = Math.round(fps);
  if (Math.abs(fps - r) <= 0.0002 * r) return `${r}/1`;
  return `${Math.round(fps * 1000)}/1000`;
}

/** The stored member for a draft; `null` when nothing is overridden (the
 *  member is removed, and the project no longer needs the new engine for it). */
export function storedOf(draft: Draft): Interpretation | null {
  const out: Interpretation = {};
  for (const key of ['primaries', 'transfer', 'matrix', 'range', 'siting', 'alpha', 'fieldOrder'] as const) {
    const v = draft[key];
    if (v) out[key] = v;
  }
  if (draft.alpha === 'premultipliedWithColor') out.alphaMatte = /^#[0-9a-f]{6}$/i.test(draft.alphaMatte ?? '') ? draft.alphaMatte!.toUpperCase() : '#000000';
  if (draft.frameRate) {
    const [n, d] = draft.frameRate.split('/').map(Number);
    if (n && d) {
      const g = gcd(n, d);
      out.frameRateRational = [n / g, d / g];
      out.frameRate = n / d;
    }
  }
  if (draft.pixelAspect) {
    const [n, d] = draft.pixelAspect.split(':').map(Number);
    if (n && d) {
      const g = gcd(n, d);
      out.pixelAspect = [n / g, d / g];
    }
  }
  if (draft.cleanAperture === 'true' || draft.cleanAperture === 'false') out.cleanAperture = draft.cleanAperture === 'true';
  return Object.keys(out).length ? out : null;
}

/** The setting of a member in the facts. */
export function settingOf(facts: MediaFacts | null, member: InterpretMember): InterpretSetting | undefined {
  return facts?.settings.find((s) => s.member === member);
}

/** What an apply changes on the asset: its `interpret` member, and its
 *  display size and duration when the reading changes them. */
export type AssetChange = { interpret: Interpretation | null; w?: number; h?: number; dur?: number };

export function assetChange(draft: Draft, facts: MediaFacts | null, asset: { w?: unknown; h?: unknown; dur?: unknown }): AssetChange {
  const change: AssetChange = { interpret: storedOf(draft) };
  const size = facts?.displaySize;
  if (size && (size[0] !== asset.w || size[1] !== asset.h)) {
    change.w = size[0];
    change.h = size[1];
  }
  const dur = facts?.duration;
  if (typeof dur === 'number' && Number.isFinite(dur) && dur > 0 && Math.abs(dur - Number(asset.dur)) > 1e-9) change.dur = dur;
  return change;
}

/** Applies a change to an asset record in place. */
export function applyChange(asset: Record<string, unknown>, change: AssetChange): void {
  if (change.interpret) asset['interpret'] = change.interpret;
  else delete asset['interpret'];
  if (change.w !== undefined) asset['w'] = change.w;
  if (change.h !== undefined) asset['h'] = change.h;
  if (change.dur !== undefined) asset['dur'] = change.dur;
}
