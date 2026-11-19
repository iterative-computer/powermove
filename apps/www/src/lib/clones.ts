/* Effect clones: a paid plugin's look, rebuilt in Powermove from one prompt.
   Each entry renders /clones/<slug> and feeds the sitemap and llms.txt. To add
   a clone, add an entry here and its short to static/clones/<slug>.mp4. */

export type Clone = {
  slug: string;
  /** The plugin as people search for it, e.g. "Sapphire S_Glint". */
  plugin: string;
  vendor: string;
  /** What the original costs, as the vendor lists it. */
  price: string;
  priceNote: string;
  /** One sentence on what the original effect does. */
  what: string;
  /** The exact one-sentence prompt given to the agent. */
  prompt: string;
  /** The agent and model the prompt ran on, as shown in the composer. */
  agent?: { provider: string; model: string; effort: string };
  /** Vertical short, served from static/. Absent until the short is published. */
  video?: { src: string; poster: string; /** 16:9 still for cards. */ thumb: string; duration: string; uploaded: string; description: string };
  /** Controls the generated effect exposes in the Effects panel. */
  controls?: string[];
  /** Share link for the free extension. Null until it is published. */
  extensionUrl: string | null;
};

export const CLONES: Clone[] = [
  {
    slug: 'sapphire-glint',
    plugin: 'Sapphire S_Glint',
    vendor: 'Boris FX',
    price: '$1,865',
    priceNote: 'perpetual license for Boris FX Sapphire, the suite S_Glint ships in',
    what: 'S_Glint turns the bright points of a shot into star-shaped glints, with rays you can lengthen, rotate, and soften.',
    prompt: 'Recreate the sapphire S_Glint effect',
    agent: { provider: 'Claude', model: 'Opus 5.5', effort: 'High' },
    video: {
      src: '/clones/sapphire-glint.mp4',
      poster: '/clones/sapphire-glint.jpg',
      thumb: '/clones/sapphire-glint-wide.jpg',
      duration: 'PT17S',
      uploaded: '2026-11-17',
      description: 'A one-sentence prompt in Powermove rebuilds the Sapphire S_Glint effect: star glints on the street lamps of a night shot, with Brightness, Threshold, Ray Length, and Rays controls.',
    },
    controls: ['Brightness', 'Threshold', 'Ray Length', 'Rays', 'Rotate', 'Threshold Softness', 'Alternate Ray Length', 'Falloff', 'Ray Width', 'Core Glow'],
    extensionUrl: null,
  },
  {
    slug: 'optical-flares',
    plugin: 'Optical Flares',
    vendor: 'Video Copilot',
    price: '$124.95',
    priceNote: 'for the Video Copilot Optical Flares plugin',
    what: 'Optical Flares adds lens flares to a shot, from soft glows and streaks to rings and ghosts that move with the light source.',
    prompt: 'Recreate the Video Copilot Optical Flares effect',
    extensionUrl: null,
  },
];

export const cloneTitle = (clone: Clone) => `Free ${clone.plugin} alternative, made in Powermove`;

export function cloneDescription(clone: Clone): string {
  return clone.video
    ? `${clone.plugin} costs ${clone.price}. Powermove, the video editor you customize with AI, rebuilt its look from one sentence. Watch the short and see how it works.`
    : `${clone.plugin} costs ${clone.price}. See how Powermove, the video editor you customize with AI, rebuilds plugin effects like it from a one-sentence prompt.`;
}

export const cloneBySlug = (slug: string) => CLONES.find((clone) => clone.slug === slug);
