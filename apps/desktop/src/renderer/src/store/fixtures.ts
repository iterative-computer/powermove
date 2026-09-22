/* Mock registry data for the Store design pass. Shapes follow the cloud notes:
   coordinates are publisher/id, the artifact is source, and lineage
   (forkedFrom) is recorded on publish. Nothing here talks to a server. */

export type StoreKind = 'effects' | 'transitions' | 'panels' | 'themes' | 'commands' | 'layers';

export type StoreListing = {
  publisher: string;
  id: string;
  name: string;
  tagline: string;
  kind: StoreKind;
  version: string;
  updated: string;
  installed?: boolean;
  /** Registry coordinate this was published from, as `publisher/id@version`. */
  forkedFrom?: string;
  /** Variables the extension reads at runtime. Values live on this Mac, never
   *  in the package, so publishing can't leak them. Required ones are asked
   *  for at install. */
  vars?: Array<{ key: string; label: string; secret?: boolean; required?: boolean; hint?: string }>;
  /** Two hues for the preview artwork, until real preview renders exist. */
  art: [string, string];
  about?: string;
  files?: string[];
  versions?: Array<{ version: string; date: string; note: string }>;
};

export const KIND_LABEL: Record<StoreKind, string> = {
  effects: 'Effect',
  transitions: 'Transition',
  panels: 'Panel',
  themes: 'Theme',
  commands: 'Commands',
  layers: 'Layer'
};

export const KIND_PLURAL: Record<StoreKind, string> = {
  effects: 'Effects',
  transitions: 'Transitions',
  panels: 'Panels',
  themes: 'Themes',
  commands: 'Commands',
  layers: 'Layers'
};

export const FEATURED: StoreListing[] = [
  {
    publisher: 'mara', id: 'glass-blur', name: 'Glass blur', kind: 'effects', version: '1.4.0', updated: 'Sep 19',
    tagline: 'Frosted backdrop blur with tint, grain and edge light, keyed to layer motion.',
    art: ['oklch(62% .13 250)', 'oklch(80% .08 300)'],
    about: 'A backdrop blur that reads like glass rather than a soft focus. Blur radius follows layer velocity, so fast moves smear and rests settle sharp. Tint, grain and a one-pixel edge highlight are separate controls.',
    files: ['manifest.json', 'glass-blur.ts', 'shaders/glass.frag', 'shaders/edge.frag', 'ui/GlassInspector.svelte'],
    versions: [
      { version: '1.4.0', date: 'Sep 19', note: 'Edge light follows the composition background.' },
      { version: '1.3.2', date: 'Aug 30', note: 'Grain is stable across frame rates.' },
      { version: '1.3.0', date: 'Aug 12', note: 'Velocity-linked radius.' }
    ]
  },
  {
    publisher: 'ollie', id: 'kinetic-type', name: 'Kinetic type', kind: 'layers', version: '0.9.1', updated: 'Sep 17',
    tagline: 'Per-glyph offsets, weight ramps and tracking curves on any text layer.',
    art: ['oklch(58% .16 30)', 'oklch(78% .1 80)']
  },
  {
    publisher: 'powermove', id: 'camera-rig', name: 'Camera rig', kind: 'panels', version: '2.0.0', updated: 'Sep 14',
    tagline: 'Dolly, orbit and rack focus for 3D layers from one panel.',
    art: ['oklch(45% .06 200)', 'oklch(70% .1 160)']
  }
];

export const PICKS: StoreListing[] = [
  { publisher: 'noor', id: 'film-grain', name: 'Film grain', kind: 'effects', version: '2.1.0', updated: 'Sep 20',
    tagline: 'Stock-accurate grain with halation and gate weave.', art: ['oklch(40% .05 60)', 'oklch(65% .09 40)'] },
  { publisher: 'theo', id: 'match-cut', name: 'Match cut', kind: 'transitions', version: '1.0.3', updated: 'Sep 18',
    tagline: 'Cuts on shape and colour, with a morph you can keyframe.', art: ['oklch(50% .12 320)', 'oklch(72% .1 350)'] },
  { publisher: 'mara', id: 'bento', name: 'Bento', kind: 'panels', version: '0.6.0', updated: 'Sep 18',
    tagline: 'Lay out layers on a grid and animate between arrangements.', art: ['oklch(55% .09 140)', 'oklch(78% .07 110)'] },
  { publisher: 'ines', id: 'paper', name: 'Paper', kind: 'themes', version: '1.2.0', updated: 'Sep 16', installed: true,
    tagline: 'A warm light theme with ink-on-paper contrast.', art: ['oklch(90% .02 80)', 'oklch(75% .04 60)'] },
  { publisher: 'theo', id: 'shake', name: 'Shake', kind: 'effects', version: '3.0.0', updated: 'Sep 15',
    tagline: 'Handheld camera shake with a real sensor-motion profile.', art: ['oklch(48% .1 20)', 'oklch(66% .08 0)'] },
  { publisher: 'kai', id: 'lower-thirds', name: 'Lower thirds', kind: 'layers', version: '1.1.0', updated: 'Sep 12',
    tagline: 'Name plates that fit the composition and time themselves.', art: ['oklch(52% .1 230)', 'oklch(74% .08 200)'] }
];

export const NEW: StoreListing[] = [
  { publisher: 'ana', id: 'wipe-set', name: 'Wipe set', kind: 'transitions', version: '0.3.0', updated: 'Today',
    tagline: 'Twelve wipes with a shared feather and angle.', art: ['oklch(60% .1 100)', 'oklch(80% .06 90)'] },
  { publisher: 'kai', id: 'color-match', name: 'Colour match', kind: 'commands', version: '0.1.2', updated: 'Yesterday',
    tagline: 'Pull a palette from one layer onto another.', art: ['oklch(55% .14 10)', 'oklch(72% .12 280)'],
    about: 'Reads the dominant colours of one layer and maps them onto another, keeping luminance. Uses a hosted model for the mapping, so it needs a key.',
    vars: [
      { key: 'OPENAI_API_KEY', label: 'OpenAI API key', secret: true, required: true, hint: 'Used for the colour mapping model.' },
      { key: 'PALETTE_SIZE', label: 'Palette size', hint: 'How many colours to pull. Defaults to 5.' }
    ] },
  { publisher: 'noor', id: 'timeline-mini', name: 'Timeline mini', kind: 'panels', version: '1.0.0', updated: 'Sep 19', forkedFrom: 'powermove/timeline@1.0.0',
    tagline: 'The built-in timeline, cut down to one strip.', art: ['oklch(35% .03 260)', 'oklch(55% .05 240)'],
    about: 'Noor’s cut of the built-in timeline: one track strip, no keyframe lanes, for when the composition is simple and the panel should be small.' },
  { publisher: 'ollie', id: 'nord', name: 'Nord', kind: 'themes', version: '2.0.0', updated: 'Sep 19',
    tagline: 'The Nord palette on Powermove’s surfaces.', art: ['oklch(40% .04 250)', 'oklch(70% .06 210)'] }
];

export const ALL: StoreListing[] = [...FEATURED, ...PICKS, ...NEW];

export const INSTALLED: StoreListing[] = [
  PICKS.find((l) => l.id === 'paper')!,
  { publisher: 'you', id: 'ease-lab', name: 'Ease lab', kind: 'panels', version: '0.2.0', updated: 'Sep 20',
    tagline: 'Compare easing curves side by side. Made with your agent.', installed: true, art: ['oklch(50% .08 170)', 'oklch(70% .06 190)'] }
];

export function coordinate(l: StoreListing): string {
  return `${l.publisher}/${l.id}`;
}
